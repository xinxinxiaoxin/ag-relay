import { invoke, isTauri } from './gateway.ts'

export interface CloseRequest { id: number; trayAvailable: boolean }
export type CloseAction = 'cancel' | 'tray' | 'quit'

export async function respondToClose(requestId: number, action: CloseAction): Promise<void> {
  if (!isTauri()) throw new Error('关闭窗口操作仅在桌面应用中可用。')
  await invoke<void>('respond_to_close', { requestId, action })
}

/** 先接事件再读未决请求；迟到快照与重复事件不能重开已处理的对话框。 */
export function watchWindowLifecycle(callbacks: {
  onClose: (request: CloseRequest) => void
  onExitFailed: (detail: string) => void
  onError: (error: unknown) => void
}): () => void {
  if (!isTauri()) return () => {}
  let disposed = false
  let eventRevision = 0
  const seen = new Set<number>()
  const stops: (() => void)[] = []
  const deliver = (request: CloseRequest | null) => {
    if (disposed || !request || seen.has(request.id)) return
    seen.add(request.id)
    callbacks.onClose(request)
  }
  void (async () => {
    const listen = (globalThis as { __TAURI__?: { event?: {
      listen: <T>(name: string, handler: (event: { payload: T }) => void) => Promise<() => void>
    } } }).__TAURI__?.event?.listen
    if (!listen) throw new Error('窗口事件通道不可用，请重新打开面板。')
    const stopClose = await listen<CloseRequest>('window-close-requested', ({ payload }) => {
      eventRevision++
      deliver(payload)
    })
    if (disposed) { stopClose(); return }
    stops.push(stopClose)
    const stopFailure = await listen<{ detail: string }>('app-exit-failed', ({ payload }) => {
      if (!disposed) callbacks.onExitFailed(payload.detail)
    })
    if (disposed) { stopFailure(); return }
    stops.push(stopFailure)
    const revision = eventRevision
    const request = await invoke<CloseRequest | null>('get_close_request')
    if (revision === eventRevision) deliver(request)
  })().catch(error => { if (!disposed) callbacks.onError(error) })
  return () => { disposed = true; stops.forEach(stop => stop()) }
}

export interface DesktopWindow {
  minimize: () => Promise<void>
  toggleMaximize: () => Promise<void>
  isMaximized: () => Promise<boolean>
  close: () => Promise<void>
  onResized: (handler: () => void) => Promise<() => void>
}

export function desktopWindow(): DesktopWindow | null {
  return (globalThis as {
    __TAURI__?: { window?: { getCurrentWindow: () => DesktopWindow } }
  }).__TAURI__?.window?.getCurrentWindow() ?? null
}
