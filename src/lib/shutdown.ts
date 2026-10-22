import { invoke, isTauri } from './gateway.ts'

export interface ShutdownProgress {
  operationId: number
  kind: 'exit' | 'update'
  phase: 'waiting' | 'cancelling' | 'committing'
  elapsedMs: number
  remainingMs: number
  remainingRequests: number | null
}

export interface ShutdownState {
  progress: ShutdownProgress | null
  error: string | null
  /** 请求受理不等于取消完成，保留到后端进度结束或操作编号变化。 */
  cancelRequestedId: number | null
}

export async function getShutdownProgress(): Promise<ShutdownProgress | null> {
  return isTauri() ? invoke<ShutdownProgress | null>('get_shutdown_progress') : null
}

export async function cancelShutdown(operationId: number): Promise<void> {
  if (!Number.isSafeInteger(operationId) || operationId <= 0) throw new Error('等待操作编号无效。')
  if (!isTauri()) throw new Error('取消等待仅在桌面应用中可用。')
  await invoke<void>('cancel_shutdown', { operationId })
}

/** 不推算完成比例或请求数；尚未读到快照时明确显示未知。 */
export function shutdownProgressText(progress: ShutdownProgress | null): string {
  const count = progress && progress.remainingRequests !== null && Number.isSafeInteger(progress.remainingRequests) && progress.remainingRequests >= 0
    ? `剩余${progress.remainingRequests}个` : '数量未知'
  const elapsed = progress && Number.isFinite(progress.elapsedMs) && progress.elapsedMs >= 0
    ? `已等待${Math.floor(progress.elapsedMs / 1000)}秒` : '已等待时间读取中'
  return `暂停接收新请求 · ${count} · ${elapsed} · 最长10分钟`
}

export function shutdownControls(state: ShutdownState) {
  const requested = state.progress !== null && state.cancelRequestedId === state.progress.operationId
  const cancelling = state.progress?.phase === 'cancelling' || requested
  const committing = state.progress?.phase === 'committing'
  return {
    canCancel: state.progress?.phase === 'waiting' && !requested,
    cancelText: committing ? '已开始完成操作' : cancelling ? '正在取消等待…' : '取消等待',
    detail: committing
      ? '已进入停止网关或安装交接阶段，不能再取消；不会强制结束请求。'
      : cancelling
        ? '正在取消等待；后端确认取消完成前，仍暂停接收新请求。'
        : state.progress
          ? '可以取消等待；最长等待10分钟，超时取消操作，不强制退出。'
          : '正在读取等待进度；读到操作编号后才能取消。',
  }
}

/** 单一只读轮询，取消独立于退出/更新长任务；旧操作响应不能污染新操作。 */
export function createShutdownController() {
  let state: ShutdownState = { progress: null, error: null, cancelRequestedId: null }
  let sequence = 0
  let applied = 0
  let generation = 0
  let cancelError: { operationId: number; message: string } | null = null
  const listeners = new Set<() => void>()
  const publish = (patch: Partial<ShutdownState>) => {
    state = { ...state, ...patch }
    listeners.forEach(listener => listener())
  }
  const refresh = async () => {
    const ticket = ++sequence
    const epoch = generation
    try {
      const progress = await getShutdownProgress()
      if (epoch !== generation || ticket < applied) return
      applied = ticket
      if (cancelError?.operationId !== progress?.operationId) cancelError = null
      publish({
        progress,
        error: cancelError?.message ?? null,
        cancelRequestedId: progress?.operationId === state.cancelRequestedId ? state.cancelRequestedId : null,
      })
    } catch (error) {
      if (epoch !== generation || ticket < applied) return
      applied = ticket
      publish({ error: `无法读取等待进度：${error instanceof Error ? error.message : String(error)}` })
    }
  }
  const cancel = async (operationId: number): Promise<void> => {
    if (state.progress?.operationId !== operationId || !shutdownControls(state).canCancel) return
    const epoch = generation
    cancelError = null
    publish({ cancelRequestedId: operationId, error: null })
    try {
      await cancelShutdown(operationId)
      // void 只代表受理，不能在此清空进度、关闭确认框或宣称恢复接单。
    } catch (error) {
      if (epoch !== generation || state.progress?.operationId !== operationId || state.cancelRequestedId !== operationId) return
      cancelError = { operationId, message: `取消等待失败：${error instanceof Error ? error.message : String(error)}` }
      publish({ cancelRequestedId: null, error: cancelError.message })
    }
  }
  return {
    getSnapshot: () => state,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    refresh,
    cancel,
    invalidate: () => { generation++; applied = ++sequence },
  }
}
