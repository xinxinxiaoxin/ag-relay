import { useEffect, useState } from 'react'
import { MinusIcon, SquareIcon, CopyIcon, XIcon } from 'lucide-react'
import { toast } from 'sonner'
import { desktopWindow } from '@/lib/window'

export function WindowControls() {
  const [maximized, setMaximized] = useState(false)
  const [nativeWindow] = useState(desktopWindow)

  useEffect(() => {
    if (!nativeWindow) return
    let active = true
    let unlisten: (() => void) | undefined
    const sync = async () => {
      const value = await nativeWindow.isMaximized()
      if (active) setMaximized(value)
    }
    void nativeWindow.onResized(() => { void sync().catch(() => {}) })
      .then(stop => {
        if (!active) { stop(); return }
        unlisten = stop
        return sync()
      }).catch(() => { if (active) toast.error('无法读取窗口状态，请重新打开面板。') })
    return () => { active = false; unlisten?.() }
  }, [nativeWindow])

  const run = async (action: 'minimize' | 'toggleMaximize' | 'close') => {
    if (!nativeWindow) {
      toast.info('窗口操作仅在桌面应用中可用。')
      return
    }
    try {
      await nativeWindow[action]()
    } catch {
      toast.error('窗口操作未完成，请重试。')
    }
  }

  return (
    <div className="window-controls" role="group" aria-label="窗口操作">
      <button type="button" className="window-control" aria-label="最小化窗口" title="最小化" onClick={() => void run('minimize')}><MinusIcon aria-hidden="true" /></button>
      <button type="button" className="window-control" aria-label={maximized ? '还原窗口' : '最大化窗口'} title={maximized ? '还原' : '最大化'} onClick={() => void run('toggleMaximize')}>
        {maximized ? <CopyIcon aria-hidden="true" /> : <SquareIcon aria-hidden="true" />}
      </button>
      <button type="button" className="window-control window-control-close" aria-label="关闭窗口并安全退出" title="关闭并安全退出" onClick={() => void run('close')}><XIcon aria-hidden="true" /></button>
    </div>
  )
}
