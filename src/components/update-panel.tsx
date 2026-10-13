import { useState } from 'react'
import { Download, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Progress } from '@/components/ui/progress'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { downloadProgress, updateControls, type UpdateState, type UpdateView } from '@/lib/update'

export interface UpdatePanelProps {
  state: UpdateState
  onCheck: () => Promise<void>
  onConfirm: (candidateId: string) => Promise<void>
}

const stageText: Partial<Record<UpdateView['phase'], string>> = {
  verifying: '正在校验更新包签名，尚未开始安装。',
  verified: '签名校验通过，准备优雅排空网关。',
  quiescing: '正在优雅排空在途请求，超时会取消安装并恢复接单，不强杀。',
  install_pending: '已交给原生安装器，请在安装器中继续；本面板无法确认安装结果。',
}

/** 受控更新卡只由设置页渲染；候选说明永远作为纯文本。 */
export function UpdatePanel({ state, onCheck, onConfirm }: UpdatePanelProps) {
  const { view, error } = state
  const { busy, canCheck, canConfirm } = updateControls(state)
  const [confirmation, setConfirmation] = useState<UpdateView['release']>(null)
  const confirmationCurrent = canConfirm && confirmation?.id === view?.release?.id
  const download = view?.phase === 'downloading' ? downloadProgress(view.download) : null
  const stage = view ? stageText[view.phase] : null

  return (
    <section aria-label="AG Relay 更新" data-testid="update-panel" className="rounded-2xl border border-border bg-card px-5 py-4 text-sm">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <h2 className="font-medium">AG Relay 更新{view?.release ? ` · v${view.release.version}` : ''}</h2>
          <p role="status" className="text-xs leading-5 text-muted-foreground">{view?.detail ?? '正在读取更新状态…'}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" disabled={!canCheck} onClick={() => void onCheck()}>
            <RefreshCw className={view?.phase === 'checking' ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />检查更新
          </Button>
          {canConfirm && <Button size="sm" onClick={() => setConfirmation(view!.release)}>
            <Download aria-hidden="true" />{view?.phase === 'deferred' ? '稍后重试更新' : '查看并更新'}
          </Button>}
        </div>
      </div>
      {(download || stage) && <div className="mt-3 space-y-2" role="status" aria-atomic="true">
        <p className="text-xs leading-5 tabular-nums">{download?.text ?? stage}</p>
        <Progress value={download?.value ?? null} aria-label={download ? '更新包下载进度' : '更新处理进度'} aria-valuetext={download?.text ?? stage ?? undefined} />
      </div>}
      {view?.recovery && <p className="mt-2 text-xs leading-5">{view.recovery}</p>}
      {error && <p role="alert" className="mt-2 text-xs leading-5 text-destructive">{error}</p>}
      {view?.release && <details className="mt-3 text-xs leading-5" open={canConfirm ? true : undefined}>
        <summary className="cursor-pointer font-medium">更新说明 · v{view.currentVersion} → v{view.release.version}</summary>
        <p className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words text-muted-foreground">{view.release.notes || '此版本未提供更新说明。'}</p>
      </details>}
      <AlertDialog open={confirmation !== null} onOpenChange={open => { if (!open) setConfirmation(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>下载并安装 AG Relay v{confirmation?.version}？</AlertDialogTitle>
            <AlertDialogDescription>
              确认后才下载，完整验签通过后再优雅排空在途请求。排空超时会取消安装并恢复接单，请稍后重试；不会强制结束请求。安装器启动后本窗口会退出，完成后重开将恢复更新前的运行或停止状态。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <p className="max-h-48 overflow-auto whitespace-pre-wrap break-words text-xs leading-5 text-muted-foreground">{confirmation?.notes || '此版本未提供更新说明。'}</p>
          <p className="text-xs leading-5 text-muted-foreground">只更新本次安装；外部网关不在更新管理范围。连接设置和日志不会被清空。首次使用旧 ZIP 版需先手动安装一次具备更新能力的版本。</p>
          {!confirmationCurrent && <p role="alert" className="text-xs text-destructive">候选版本已变化或操作正在进行，请取消后重新检查更新。</p>}
          <AlertDialogFooter>
            <AlertDialogCancel>暂不更新</AlertDialogCancel>
            <AlertDialogAction disabled={busy || !confirmationCurrent} onClick={() => {
              if (!confirmationCurrent || !confirmation) return
              const id = confirmation.id
              setConfirmation(null)
              void onConfirm(id)
            }}>确认下载并安装</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}
