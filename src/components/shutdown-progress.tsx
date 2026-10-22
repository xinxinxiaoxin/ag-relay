import { Button } from '@/components/ui/button'
import { shutdownControls, shutdownProgressText, type ShutdownState } from '@/lib/shutdown'

export interface ShutdownProgressProps {
  shutdown?: ShutdownState
  onCancelWait?: (operationId: number) => Promise<void>
  showCancel?: boolean
}

const unread: ShutdownState = { progress: null, error: null, cancelRequestedId: null }

export function ShutdownProgress({ shutdown = unread, onCancelWait, showCancel = true }: ShutdownProgressProps) {
  const controls = shutdownControls(shutdown)
  return <div data-testid="shutdown-progress" className="space-y-2 text-xs leading-5">
    <div role="status" aria-atomic="true" className="space-y-1">
      <p className="tabular-nums">{shutdownProgressText(shutdown.progress)}</p>
      <p className="text-muted-foreground">{controls.detail}</p>
    </div>
    {shutdown.error && <p role="alert" className="text-destructive">{shutdown.error}</p>}
    {showCancel && <Button size="sm" variant="outline" disabled={!controls.canCancel || !onCancelWait} onClick={() => {
      if (shutdown.progress) void onCancelWait?.(shutdown.progress.operationId)
    }}>{controls.cancelText}</Button>}
  </div>
}
