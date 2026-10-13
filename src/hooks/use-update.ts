import { useEffect, useState, useSyncExternalStore } from 'react'
import { createUpdateController } from '@/lib/update'

/** 仅 App 调用，切页不会卸载轮询或丢失候选提醒。 */
export function useUpdate() {
  const [controller] = useState(createUpdateController)
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot)
  useEffect(() => {
    void controller.refresh()
    const timer = setInterval(() => { void controller.refresh() }, 1000)
    return () => { clearInterval(timer); controller.invalidate() }
  }, [controller])
  return { state, onCheck: controller.check, onConfirm: controller.confirm }
}
