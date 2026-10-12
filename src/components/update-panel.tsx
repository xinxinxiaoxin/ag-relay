import { useEffect, useState } from 'react'
import { Download, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { checkForUpdate, confirmUpdate, getUpdateView, type UpdateView } from '@/lib/update'

const busyPhases = new Set<UpdateView['phase']>(['checking', 'downloading', 'verified', 'quiescing', 'install_pending'])

/** 所有页面共用更新提示；版本说明按纯文本展示，不执行远端 HTML。 */
export function UpdatePanel() {
  const [view, setView] = useState<UpdateView | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [working, setWorking] = useState(false)
  const [confirmation, setConfirmation] = useState<UpdateView['release']>(null)

  useEffect(() => {
    let active = true
    // 这里只轮询 Rust 内存状态；不轮询发布端点、不触碰网关控制端点。
    const refresh = () => void getUpdateView().then(next => {
      if (active) setView(next)
    }).catch(() => { if (active) setError('暂时无法读取更新状态，请稍后重试。') })
    refresh()
    const timer = setInterval(refresh, 1000)
    return () => { active = false; clearInterval(timer) }
  }, [])

  const run = async (action: () => Promise<UpdateView>) => {
    setWorking(true)
    setError(null)
    try { setView(await action()) }
    catch (reason) { setError(String(reason)) }
    finally { setWorking(false) }
  }
  const busy = working || (view !== null && busyPhases.has(view.phase))
  const canConfirm = view?.release && ['available', 'deferred'].includes(view.phase)

  return (
    <section aria-label="AG Relay 更新" className="mb-6 rounded-2xl border border-border bg-card px-5 py-4 text-sm">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <h2 className="font-medium">AG Relay 更新{view?.release ? ` · v${view.release.version}` : ''}</h2>
          <p role="status" className="text-xs leading-5 text-muted-foreground">{view?.detail ?? '正在读取更新状态…'}</p>
        </div>
        <div className="flex shrink-0 gap-2">
          <Button variant="outline" size="sm" disabled={busy || !view || view.phase === 'disabled'} onClick={() => void run(checkForUpdate)}>
            <RefreshCw className={busy ? 'animate-spin' : ''} aria-hidden="true" />检查更新
          </Button>
          {canConfirm && <Button size="sm" disabled={busy} onClick={() => setConfirmation(view.release)}>
            <Download aria-hidden="true" />{view.phase === 'deferred' ? '稍后重试更新' : '查看并更新'}
          </Button>}
        </div>
      </div>
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
          <AlertDialogFooter>
            <AlertDialogCancel>暂不更新</AlertDialogCancel>
            <AlertDialogAction disabled={busy} onClick={() => {
              const id = confirmation?.id
              setConfirmation(null)
              if (id) void run(() => confirmUpdate(id))
            }}>确认下载并安装</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}
