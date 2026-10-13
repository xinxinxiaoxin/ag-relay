import { FALLBACK_APP_VERSION, invoke, isTauri } from './gateway.ts'

export interface UpdateView {
  phase: 'disabled' | 'idle' | 'checking' | 'available' | 'downloading' | 'verifying' | 'verified'
    | 'quiescing' | 'deferred' | 'install_pending' | 'failed' | 'needs_attention'
  currentVersion: string
  release: { id: string; version: string; notes: string } | null
  detail: string
  recovery: string | null
  download: { downloadedBytes: number; totalBytes: number | null } | null
}

/** MB 采用十进制字节，百分比与原始字节同源。 */
export function downloadProgress(download: UpdateView['download']): { value: number | null; text: string } {
  if (!download || !Number.isSafeInteger(download.downloadedBytes) || download.downloadedBytes < 0) {
    return { value: null, text: '正在下载 · 进度暂不可用' }
  }
  const downloaded = download.downloadedBytes
  const total = download.totalBytes
  if (total === null || !Number.isSafeInteger(total) || total <= 0 || downloaded > total) {
    return { value: null, text: `已下载 ${(downloaded / 1_000_000).toFixed(1)} MB · 总大小未知` }
  }
  const value = downloaded / total * 100
  return { value, text: `${Math.floor(value)}% · ${(downloaded / 1_000_000).toFixed(1)} / ${(total / 1_000_000).toFixed(1)} MB` }
}

// 普通浏览器不冒充一次真实更新检查，更不模拟安装成功。
const unavailable: UpdateView = {
  phase: 'disabled', currentVersion: FALLBACK_APP_VERSION, release: null,
  detail: '浏览器预览不支持应用内更新，请在已安装的 AG Relay 中检查。', recovery: null, download: null,
}

export async function getUpdateView(): Promise<UpdateView> {
  return isTauri() ? invoke<UpdateView>('get_update_state') : unavailable
}

export async function checkForUpdate(): Promise<UpdateView> {
  return isTauri() ? invoke<UpdateView>('check_for_update') : unavailable
}

export async function confirmUpdate(candidateId: string): Promise<UpdateView> {
  // Rust 只接受候选编号，不接受下载 URL、安装路径、签名或包字节。
  if (!isTauri()) return unavailable
  return invoke<UpdateView>('confirm_update', { candidateId })
}

export interface UpdateState {
  view: UpdateView | null
  error: string | null
  working: boolean
  hasUpdate: boolean
}

/** 忙碌阶段不能启动另一个检查或下载，验签和原生安装交接也算忙碌。 */
const busyPhases = new Set<UpdateView['phase']>(['checking', 'downloading', 'verifying', 'verified', 'quiescing', 'install_pending'])
export function updateControls(state: UpdateState) {
  const busy = state.working || (state.view !== null && busyPhases.has(state.view.phase))
  return {
    busy,
    canCheck: !busy && state.view !== null && state.view.phase !== 'disabled',
    canConfirm: !busy && Boolean(state.view?.release) && ['available', 'deferred'].includes(state.view?.phase ?? ''),
  }
}

/** App 生命周期内的单一更新状态；只读 Rust 内存，不主动轮询发布端点。 */
export function createUpdateController() {
  let state: UpdateState = { view: null, error: null, working: false, hasUpdate: false }
  let sequence = 0
  let applied = 0
  let generation = 0
  let operationError: string | null = null
  const listeners = new Set<() => void>()
  const publish = (patch: Partial<UpdateState>) => {
    state = { ...state, ...patch }
    listeners.forEach(listener => listener())
  }
  const accept = (view: UpdateView, ticket: number, epoch: number) => {
    if (epoch !== generation || ticket < applied) return false
    applied = ticket
    const hasUpdate = view.release
      ? view.release.version !== view.currentVersion
      : ['checking', 'failed', 'needs_attention'].includes(view.phase) && state.hasUpdate
    publish({ view, error: operationError, hasUpdate })
    return true
  }
  const refresh = async () => {
    const ticket = ++sequence
    const epoch = generation
    try { accept(await getUpdateView(), ticket, epoch) }
    catch (error) {
      if (epoch !== generation || ticket < applied) return
      applied = ticket
      publish({ error: String(error) })
    }
  }
  const run = async (action: () => Promise<UpdateView>, checking: boolean) => {
    const epoch = ++generation // 使操作前发出的所有读取失效。
    const ticket = ++sequence
    applied = ticket
    operationError = null
    publish({ working: true, error: null, ...(checking && state.view ? {
      view: { ...state.view, phase: 'checking', release: null, download: null, detail: '正在检查更新…' },
    } : {}) })
    try {
      const view = await action()
      if (epoch !== generation) return
      // 屏蔽操作完成前仍在途的读取；操作响应也不能覆盖已显示的较新快照。
      const newerSnapshot = applied > ticket
      applied = ++sequence
      if (newerSnapshot) await refresh()
      else accept(view, applied, epoch)
    } catch (error) {
      if (epoch !== generation) return
      applied = ++sequence
      operationError = error instanceof Error ? error.message : String(error)
      publish({ error: operationError, ...(checking && state.view ? {
        view: { ...state.view, phase: 'failed', release: null, download: null, detail: '检查更新失败，请稍后重试。' },
      } : {}) })
    } finally {
      if (epoch === generation) publish({ working: false })
    }
  }
  return {
    getSnapshot: () => state,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    refresh,
    invalidate: () => { generation++; applied = ++sequence },
    check: async () => { if (updateControls(state).canCheck) await run(checkForUpdate, true) },
    confirm: async (candidateId: string) => {
      if (!updateControls(state).canConfirm || candidateId !== state.view?.release?.id) {
        publish({ error: '候选版本已变化或当前操作尚未结束，请重新检查更新。' })
        return
      }
      await run(() => confirmUpdate(candidateId), false)
    },
  }
}
