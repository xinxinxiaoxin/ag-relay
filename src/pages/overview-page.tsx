import { Activity, ArrowRight, Copy, LoaderCircle, Play, RotateCw, Settings2, Square, TriangleAlert, X } from 'lucide-react'
import { Alert, AlertAction, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ActivityTable } from '@/components/activity-table'
import { isTauri } from '@/lib/gateway'
import { RequestProgressCard } from '@/components/request-progress-card'
import { type GatewayErrorInfo, type GatewayState, type PageProps } from '@/lib/types'

const stateCopy: Record<GatewayState, { title: string; description: string }> = {
  running: { title: '运行中', description: '已就绪，可以连接客户端。' },
  stopped: { title: '未启动', description: '启动后即可连接客户端。' },
  external: { title: '运行中', description: '外部实例运行中，停止或重启需确认。' },
  error: { title: '端口冲突', description: '请先在设置中检查端口。' },
  starting: { title: '正在启动', description: '正在准备网关，请稍候。' },
  draining: { title: '等待请求结束', description: '完成后再停止，暂不接受新请求。' },
}

export function OverviewPage({ endpoint: connectionEndpoint, snapshot, events, activity, onAction, onNavigate, appVersion, gatewayError, onDismissError }: PageProps & { appVersion: string; gatewayError: GatewayErrorInfo | null; onDismissError: () => void }) {
  const { state } = snapshot
  const copy = stateCopy[state]
  const connected = ['running', 'external', 'draining'].includes(state)
  const pending = state === 'starting' || state === 'draining'
  const canStart = state === 'stopped'
  const canStop = (state === 'running' && snapshot.owner === 'window') || state === 'external'
  const ownerText = snapshot.owner === 'window' ? '当前窗口' : snapshot.owner === 'external' ? '外部进程' : '无'
  const recentEvents = events.filter(event => event.code !== 'request_status').slice(0, 6)
  const activityError = activity.error ?? (activity.status === 'error' ? activity.message : null)
  const controlNote = state === 'external' ? '外部实例：仅在确认可安全停止时操作。'
    : state === 'running' && !canStop ? '尚未核实实例归属，停止与重启暂不可用。'
    : pending ? '状态切换中，请稍候。' : state === 'error' ? '请先解决端口冲突。' : '仅控制本窗口启动的网关。'

  return (
    <div className="flex w-full min-w-0 flex-col gap-5">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-2xl font-semibold tracking-tight">网关总览</h1>
        <div className="flex items-center gap-3">
          {!isTauri() && <span className="text-xs text-foreground/65">示例数据 · 不连接真实网关</span>}
          <Button variant="outline" size="sm" onClick={() => onNavigate('settings')}><Settings2 aria-hidden="true" />启动与退出</Button>
        </div>
      </header>

      {gatewayError && (
        <Alert variant="destructive" className="border-destructive/25 bg-destructive/5">
          <TriangleAlert aria-hidden="true" />
          <AlertTitle>{gatewayError.title}</AlertTitle>
          <AlertDescription><p>{gatewayError.detail}</p>{gatewayError.portConflict && <Button variant="outline" size="sm" onClick={() => onNavigate('settings')}>检查设置<ArrowRight aria-hidden="true" /></Button>}</AlertDescription>
          <AlertAction><Button variant="ghost" size="icon-sm" aria-label="关闭错误提示" onClick={onDismissError}><X aria-hidden="true" /></Button></AlertAction>
        </Alert>
      )}

      <Card size="sm" className="min-w-0 gap-2" role="region" aria-label="网关控制与连接">
        <CardContent className="space-y-2.5">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-2.5" role="status" aria-live="polite" aria-atomic="true">
              {pending ? <LoaderCircle className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <span className={`size-2 rounded-full ${state === 'error' ? 'bg-destructive' : connected ? 'bg-foreground' : 'bg-muted-foreground'}`} aria-hidden="true" />}
              <h2 title={copy.description} className="text-lg font-semibold">{copy.title}</h2>
              <Badge variant="secondary" className="text-[11px] font-normal">{snapshot.owner === 'window' ? '受管网关' : snapshot.owner === 'external' ? '外部实例' : '未归属'}</Badge>
            </div>
            <div className="flex items-center gap-2" title={controlNote}>
              <Button size="sm" disabled={!canStart && !canStop} aria-describedby="gateway-control-note" onClick={() => onAction(canStart ? 'start' : 'stop')}>
                {pending ? <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : canStart ? <Play aria-hidden="true" /> : <Square aria-hidden="true" />}
                {pending ? '请稍候' : canStart ? '启动网关' : '停止网关'}
              </Button>
              <Button variant="outline" size="sm" disabled={!canStop} aria-describedby="gateway-control-note" onClick={() => onAction('restart')}><RotateCw aria-hidden="true" />重启</Button>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-x-5 gap-y-1 text-xs text-muted-foreground">
            <div className="flex min-w-0 items-center gap-1"><code className="break-all text-foreground">{connectionEndpoint}</code><Button variant="ghost" size="icon-sm" aria-label="复制连接地址" onClick={() => onAction('copy')}><Copy aria-hidden="true" /></Button></div>
            <span>运行 {connected ? snapshot.uptime ?? '未知' : '—'}</span>
            <span>v{appVersion}</span>
            <details><summary className="cursor-pointer">实例信息</summary><p className="mt-1 break-all">归属：{ownerText} · 脚本：{connected ? snapshot.version ?? '未知' : '—'}<br />{controlNote}</p></details>
          </div>
          <span id="gateway-control-note" className="sr-only">{controlNote}</span>
        </CardContent>
      </Card>

      <RequestProgressCard snapshot={snapshot} onOpenLogs={() => onNavigate('logs')} />
      <Card size="sm" className="min-w-0 gap-0 pb-0" role="region" aria-labelledby="recent-activity-heading">
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between gap-2">
            <CardTitle role="heading" aria-level={2} id="recent-activity-heading" className="flex items-center gap-2 text-sm"><Activity className="size-4 text-muted-foreground" aria-hidden="true" />最近活动</CardTitle>
            <Button variant="ghost" size="sm" onClick={() => onNavigate('logs')}>全部日志<ArrowRight aria-hidden="true" /></Button>
          </div>
        </CardHeader>
        {activityError && <div role="alert" className="flex items-center gap-3 px-4 pb-2 text-xs text-destructive">读取失败，保留上次记录。<Button variant="outline" size="sm" onClick={activity.retry}>重试</Button></div>}
        {recentEvents.length > 0 ? <ActivityTable events={recentEvents} /> : <p className="px-4 py-5 text-center text-xs text-muted-foreground">{activityError ? '无法读取记录' : activity.loading ? '正在加载…' : '暂无活动'}</p>}
      </Card>
    </div>
  )
}
