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
