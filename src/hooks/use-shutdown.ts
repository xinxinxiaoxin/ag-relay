import { useEffect, useState, useSyncExternalStore } from 'react'
import { createShutdownController } from '@/lib/shutdown'

/** 仅 App 调用，退出与更新共享同一份每秒读取的后端内存快照。 */
export function useShutdown() {
  const [controller] = useState(createShutdownController)
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot)
  useEffect(() => {
    void controller.refresh()
    const timer = setInterval(() => { void controller.refresh() }, 1000)
    return () => { clearInterval(timer); controller.invalidate() }
  }, [controller])
  return { state, onCancelWait: controller.cancel }
}
