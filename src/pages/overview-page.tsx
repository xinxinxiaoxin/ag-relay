import { Fragment, useState, type ReactNode } from 'react'
import {
  Activity,
  ArrowRight,
  CircleAlert,
  Clock3,
  Copy,
  Ellipsis,
  ExternalLink,
  FileText,
  LoaderCircle,
  LockKeyhole,
  Network,
  Play,
  RotateCw,
  Server,
  Settings2,
  Shield,
  ShieldAlert,
  Square,
  TriangleAlert,
  X,
} from 'lucide-react'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Alert, AlertAction, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Item, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemTitle } from '@/components/ui/item'
import { Separator } from '@/components/ui/separator'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { ENDPOINT, type GatewayErrorInfo, type GatewayState, type PageProps } from '@/lib/types'

const endpoint = new URL(ENDPOINT)

const stateCopy: Record<GatewayState, { title: string; description: string }> = {
  running: { title: '运行中', description: '网关已就绪，可以连接本地客户端。' },
  stopped: { title: '未启动', description: '启动网关后，即可使用下方地址连接。' },
  external: { title: '运行中', description: '端口上有外部实例在运行，已接管显示其状态；停止或重启会先二次确认。' },
  error: { title: '端口冲突', description: '网关未能启动，请先检查端口占用情况。' },
  starting: { title: '正在启动', description: '正在准备网关，请稍候。' },
  draining: { title: '正在排空', description: '正在等待进行中的请求结束，请稍候。' },
}

export function OverviewPage({ snapshot, events, onAction, onNavigate, appVersion, gatewayError, onDismissError }: PageProps & { appVersion: string; gatewayError: GatewayErrorInfo | null; onDismissError: () => void }) {
  const { state } = snapshot
  const copy = stateCopy[state]
  const metricsAvailable = state === 'running' || state === 'external' || state === 'draining'
  const isPending = state === 'starting' || state === 'draining'
  const isExternal = state === 'external'
  // 受管运行中：当前窗口拉起且在运行 → 可直接停止/重启（无需外部二次确认）。
  const isManaged = state === 'running' && snapshot.owner === 'window'
  const canStart = state === 'stopped'
  // 停止/重启能力：受管运行中 或 外部实例 均保留（ADR-0003：外部非只读，但破坏性操作加二次确认）。
  const canStopOrRestart = isManaged || isExternal
  const primaryDisabled = !canStopOrRestart && !canStart
  // 二次确认（AlertDialog）就地管理于本页：受管「安全重启」+ 外部实例「停止 / 重启」。
  const [confirmRestartOpen, setConfirmRestartOpen] = useState(false)
  const [externalConfirm, setExternalConfirm] = useState<null | 'stop' | 'restart'>(null)
  const inflight = metricsAvailable && snapshot.inflight !== null
    ? snapshot.inflight.toLocaleString('zh-CN')
    : '—'
  const uptime = metricsAvailable ? snapshot.uptime ?? '—' : '—'
  // 运行版本「次显」：网关脚本短哈希（loadedVersion 前 8 位）；主显为应用版本 appVersion。
  const gatewayHash = metricsAvailable ? snapshot.version ?? '未知' : '—'
  // 实例归属：始终显示派生值（当前窗口 / 外部进程 / 无）——无网关在跑即「无」（GLOSSARY）。
  const ownerText = snapshot.owner === 'window' ? '当前窗口' : snapshot.owner === 'external' ? '外部进程' : '无'
  // 运行版本双显（主：应用版本 vX.Y.Z；次：网关脚本短哈希）。alignEnd 适配右对齐的连接信息卡。
  const versionCell = (alignEnd: boolean) => (
    <span className={`flex flex-col gap-0.5 ${alignEnd ? 'items-end' : 'items-start'}`}>
      <span>v{appVersion}</span>
      <span className="text-xs font-normal text-muted-foreground">网关脚本 {gatewayHash}</span>
    </span>
  )
  const recentEvents = events.slice(0, 4)
  const primaryLabel = isPending
    ? state === 'starting' ? '正在启动' : '正在排空'
    : canStopOrRestart ? '停止网关' : '启动网关'
  // 主控制按钮：未启动→启动；外部实例→停止前二次确认；受管运行中→直接停止。
  const handlePrimary = () => {
    if (canStart) { onAction('start'); return }
    if (isExternal) { setExternalConfirm('stop'); return }
    onAction('stop')
  }
  // 重启：外部实例→二次确认；受管→安全重启确认（排空 + 60 秒上限）。
  const handleRestart = () => {
    if (isExternal) setExternalConfirm('restart')
    else setConfirmRestartOpen(true)
  }
  const controlNote = isExternal
    ? '该网关并非本窗口拉起（外部实例）。可停止或重启，但会先二次确认，并仅在经 supervisor 父链核实归属后执行——宁可不杀，不可误杀。'
    : state === 'running' && !isManaged
      ? '端口上有网关在运行，但尚未核实归属，停止与重启暂不可用（宁可不杀，不可误杀）。'
      : isPending
        ? '状态切换期间，控制操作暂不可用。'
        : state === 'error'
          ? '端口冲突解决前，启动与重启不可用。请先检查占用程序。'
          : '仅管理当前窗口启动的网关，不会终止其他进程。'

  return (
    <div className="flex w-full min-w-0 flex-col gap-6">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0 space-y-2">
          <div className="flex flex-wrap items-center gap-3">
            <h1 className="text-3xl font-semibold leading-tight tracking-[-0.035em]">网关总览</h1>
          </div>
          <p className="text-sm leading-6 text-muted-foreground">管理本地连接，掌握网关的运行状态。</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="outline" onClick={() => onAction('copy')} className="text-sm">
            <Copy aria-hidden="true" />
            复制连接地址
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="icon" aria-label="更多操作">
                <Ellipsis aria-hidden="true" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-44">
              <DropdownMenuLabel>网关操作</DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => onAction('open-logs')}>
                <FileText aria-hidden="true" />查看日志
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => onNavigate('settings')}>
                <Settings2 aria-hidden="true" />启动设置
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </header>

      <section aria-label="网关状态与连接" className="grid min-w-0 gap-5 min-[1100px]:grid-cols-[minmax(0,1fr)_300px]">
        <Card className="min-w-0" aria-labelledby="gateway-control-heading">
          <CardHeader>
            <div className="flex flex-wrap items-center justify-between gap-3">
              <CardTitle id="gateway-control-heading" className="flex items-center gap-2 text-base font-medium">
                <Server className="size-4 text-muted-foreground" aria-hidden="true" />
                网关控制
              </CardTitle>
              <Badge variant="secondary" className="text-xs font-normal">
                {isExternal ? '接管显示' : isManaged ? '当前窗口管理' : '本地网关'}
              </Badge>
            </div>
          </CardHeader>
          <CardContent className="space-y-5">
            <div className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-4">
              <div className="min-w-0 space-y-2" role="status" aria-live="polite" aria-atomic="true">
                <div className={`flex flex-wrap items-center gap-3 ${state === 'error' ? 'text-destructive' : ''}`}>
                  {isPending ? (
                    <LoaderCircle className="size-5 shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                  ) : state === 'error' ? (
                    <TriangleAlert className="size-5 shrink-0" aria-hidden="true" />
                  ) : (
                    <span aria-hidden="true" className={`size-2.5 shrink-0 rounded-full ${metricsAvailable ? 'bg-foreground' : 'border-2 border-muted-foreground'}`} />
                  )}
                  <h2 className="text-4xl font-semibold leading-tight tracking-[-0.04em] max-[380px]:text-3xl">{copy.title}</h2>
                  {isExternal && (
                    <Badge className="h-6 gap-1 px-2.5 text-xs font-medium" aria-label="外部实例：非本窗口拉起">
                      <ExternalLink aria-hidden="true" />外部实例
                    </Badge>
                  )}
                </div>
                <p className="max-w-sm text-sm leading-6 text-muted-foreground">{copy.description}</p>
              </div>
              <div className="min-w-16 space-y-1 text-right">
                <p className="text-xs leading-5 text-muted-foreground">进行中请求</p>
                <p className="text-[42px] font-semibold leading-none tracking-[-0.045em] tabular-nums" aria-label={`进行中请求：${inflight}`}>{inflight}</p>
              </div>
            </div>

            <div className="space-y-2">
              <p id="gateway-endpoint-label" className="text-sm font-medium">{metricsAvailable ? '监听端点' : '连接地址'}</p>
              <div className="flex min-w-0 items-center gap-2">
                <Input
                  aria-labelledby="gateway-endpoint-label"
                  value={ENDPOINT}
                  readOnly
                  spellCheck={false}
                  className="h-10 min-w-0 flex-1 bg-muted/50 font-mono text-sm"
                  onFocus={(event) => event.currentTarget.select()}
                />
                <Button variant="secondary" size="icon" className="size-10 shrink-0" aria-label="复制监听端点" onClick={() => onAction('copy')}>
                  <Copy aria-hidden="true" />
                </Button>
              </div>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <Button
                disabled={primaryDisabled}
                aria-describedby="gateway-control-note"
                onClick={handlePrimary}
                className="min-w-28 text-sm"
              >
                {isPending ? <LoaderCircle className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : canStopOrRestart ? <Square aria-hidden="true" /> : <Play aria-hidden="true" />}
                {primaryLabel}
              </Button>
              <Button variant="outline" disabled={!canStopOrRestart} aria-describedby="gateway-control-note" onClick={handleRestart} className="text-sm">
                <RotateCw aria-hidden="true" />重启
              </Button>
              <Button variant="ghost" onClick={() => onAction('open-logs')} className="ml-auto text-sm">
                查看日志<ArrowRight aria-hidden="true" />
              </Button>
            </div>

            {gatewayError && (
              <Alert variant="destructive" className="border-destructive/25 bg-destructive/5">
                <TriangleAlert aria-hidden="true" />
                <AlertTitle>{gatewayError.title}</AlertTitle>
                <AlertDescription className="text-sm leading-6">
                  <p>{gatewayError.detail}</p>
                  {gatewayError.portConflict && (
                    <Button variant="outline" size="sm" className="mt-1 text-sm text-foreground" onClick={() => onNavigate('settings')}>
                      检查设置<ArrowRight aria-hidden="true" />
                    </Button>
                  )}
                </AlertDescription>
                <AlertAction>
                  <Button variant="ghost" size="icon" className="size-7 text-destructive hover:text-destructive" aria-label="忽略错误提示" onClick={onDismissError}>
                    <X aria-hidden="true" />
                  </Button>
                </AlertAction>
              </Alert>
            )}

            <Separator />
            <p id="gateway-control-note" className="flex items-start gap-2 text-xs leading-5 text-muted-foreground">
              <Shield className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
              {controlNote}
            </p>
          </CardContent>
        </Card>

        <Card className="min-w-0" aria-labelledby="connection-info-heading">
          <CardHeader>
            <CardTitle id="connection-info-heading" className="flex items-center gap-2 text-base font-medium">
              <Network className="size-4 text-muted-foreground" aria-hidden="true" />连接信息
            </CardTitle>
            <CardDescription className="text-sm leading-6">连接地址与当前实例详情。</CardDescription>
          </CardHeader>
          <CardContent className="space-y-5">
            <dl className="space-y-4 text-sm">
              {([
                ['主机地址', endpoint.hostname],
                [metricsAvailable ? '监听端口' : '配置端口', endpoint.port],
                ['运行时长', uptime],
                ['运行版本', versionCell(true)],
                ['实例归属', ownerText],
              ] as [string, ReactNode][]).map(([label, value]) => (
                <div key={label} className="flex items-start justify-between gap-4">
                  <dt className="shrink-0 text-muted-foreground">{label}</dt>
                  <dd className="min-w-0 break-words text-right font-medium tabular-nums [overflow-wrap:anywhere]">{value}</dd>
                </div>
              ))}
            </dl>
            <Separator />
            <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
              <span className="flex items-center gap-1.5"><LockKeyhole className="size-3.5" aria-hidden="true" />本机回环地址</span>
              <Badge variant="outline" className="text-xs font-normal">{endpoint.protocol.replace(':', '').toUpperCase()}</Badge>
            </div>
          </CardContent>
        </Card>
      </section>

      <section aria-label="活动与运行信息" className="grid min-w-0 items-start gap-5 min-[1100px]:grid-cols-[minmax(0,1fr)_300px]">
        <Tabs defaultValue="activity" className="min-w-0">
          <Card className="min-w-0 gap-4">
            <CardHeader>
              <div className="flex flex-wrap items-center justify-between gap-3">
                <TabsList aria-label="网关详情">
                  <TabsTrigger value="activity" className="gap-1.5 text-sm"><Activity className="size-4" aria-hidden="true" />最近活动</TabsTrigger>
                  <TabsTrigger value="runtime" className="gap-1.5 text-sm"><Clock3 className="size-4" aria-hidden="true" />运行信息</TabsTrigger>
                </TabsList>
                <Button variant="ghost" size="sm" onClick={() => onNavigate('logs')} className="text-sm">全部日志<ArrowRight aria-hidden="true" /></Button>
              </div>
            </CardHeader>
            <CardContent>
              <TabsContent value="activity" className="mt-0 space-y-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <p className="text-sm text-muted-foreground">最近的网关事件</p>
                </div>
                {recentEvents.length > 0 ? (
                  <ItemGroup className="gap-0">
                    {recentEvents.map((event, index) => (
                      <Fragment key={`${event.time}-${index}`}>
                        {index > 0 && <Separator />}
                        <Item className="items-start gap-3 px-0 py-3">
                          <ItemMedia className="mt-1 shrink-0" aria-label={event.level === 'error' ? '错误' : event.level === 'warn' ? '警告' : '信息'}>
                            {event.level === 'error' ? <TriangleAlert className="size-4 text-destructive" aria-hidden="true" /> : event.level === 'warn' ? <CircleAlert className="size-4 text-muted-foreground" aria-hidden="true" /> : <Activity className="size-4 text-muted-foreground" aria-hidden="true" />}
                          </ItemMedia>
                          <ItemContent className="min-w-0 gap-1">
                            <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                              <ItemTitle className={`min-w-0 break-words text-sm [overflow-wrap:anywhere] ${event.level === 'error' ? 'text-destructive' : ''}`}>{event.title}</ItemTitle>
                              <time className="shrink-0 text-xs leading-5 tabular-nums text-muted-foreground">{event.time}</time>
                            </div>
                            <ItemDescription className="break-words text-sm leading-6 [overflow-wrap:anywhere]">{event.detail}</ItemDescription>
                          </ItemContent>
                        </Item>
                      </Fragment>
                    ))}
                  </ItemGroup>
                ) : (
                  <Item className="px-0 py-6">
                    <ItemMedia><FileText className="size-5 text-muted-foreground" aria-hidden="true" /></ItemMedia>
                    <ItemContent>
                      <ItemTitle className="text-sm">暂无活动</ItemTitle>
                      <ItemDescription className="text-sm">网关事件会显示在这里，也可前往日志页面查看。</ItemDescription>
                    </ItemContent>
                  </Item>
                )}
                <Separator />
                <p className="pt-1 text-xs leading-5 text-muted-foreground">显示最近 {recentEvents.length} 条，共 {events.length} 条事件</p>
              </TabsContent>
              <TabsContent value="runtime" className="mt-0 space-y-5">
                <div className="space-y-1">
                  <h3 className="text-base font-medium">当前运行信息</h3>
                  <p className="text-sm leading-6 text-muted-foreground">{metricsAvailable ? '当前已连接实例的运行详情。' : '实例尚未就绪，无法获取的指标显示为 —。'}</p>
                </div>
                <dl className="grid grid-cols-1 gap-x-8 gap-y-5 min-[480px]:grid-cols-2">
                  {([
                    ['网关状态', copy.title],
                    ['实例归属', ownerText],
                    ['进行中请求', inflight],
                    ['运行时长', uptime],
                    ['运行版本', versionCell(false)],
                    ['连接协议', endpoint.protocol.replace(':', '').toUpperCase()],
                  ] as [string, ReactNode][]).map(([label, value]) => (
                    <div key={label} className="min-w-0 space-y-1.5">
                      <dt className="text-sm text-muted-foreground">{label}</dt>
                      <dd className="break-words text-base font-medium tabular-nums [overflow-wrap:anywhere]">{value}</dd>
                    </div>
                  ))}
                </dl>
                <Separator />
                <p className="text-xs leading-5 text-muted-foreground">运行指标仅在已连接实例时可用，不以 0 代替未知值。</p>
              </TabsContent>
            </CardContent>
          </Card>
        </Tabs>

        <Card className="min-w-0" aria-labelledby="startup-preferences-heading">
          <CardHeader>
            <CardTitle id="startup-preferences-heading" className="flex items-center gap-2 text-base font-medium">
              <Settings2 className="size-4 text-muted-foreground" aria-hidden="true" />启动与退出
            </CardTitle>
            <CardDescription className="text-sm leading-6">让实例控制保持清晰、可预期。</CardDescription>
          </CardHeader>
          <CardContent className="space-y-5">
            <div className="space-y-2">
              <h3 className="text-sm font-medium">启动偏好</h3>
              <p className="text-sm leading-6 text-muted-foreground">Windows 登录后启动应用，可在偏好设置中查看与调整。</p>
            </div>
            <Separator />
            <div className="space-y-2">
              <h3 className="text-sm font-medium">安全退出</h3>
              <p className="text-sm leading-6 text-muted-foreground">请求排空期间，控制操作暂不可用。停止或退出时，受管网关会优雅停止；外部实例仅在核实归属后才停止。</p>
            </div>
            <Button variant="outline" className="w-full justify-between text-sm" onClick={() => onNavigate('settings')}>
              管理启动设置<ArrowRight aria-hidden="true" />
            </Button>
          </CardContent>
        </Card>
      </section>

      <AlertDialog open={confirmRestartOpen} onOpenChange={setConfirmRestartOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>安全重启网关？</AlertDialogTitle>
            <AlertDialogDescription>
              将先优雅排空在途请求，再重启网关，最长等待 60 秒。网关受理重启不等于已恢复，恢复后面板状态会自动刷新。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction onClick={() => onAction('restart')}>确认重启</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* 外部实例破坏性操作二次确认（ADR-0003）：外部网关非只读，停止/重启前须确认。 */}
      <AlertDialog open={externalConfirm !== null} onOpenChange={(open) => { if (!open) setExternalConfirm(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogMedia>
              <ShieldAlert aria-hidden="true" />
            </AlertDialogMedia>
            <AlertDialogTitle>{externalConfirm === 'restart' ? '重启外部实例？' : '停止外部实例？'}</AlertDialogTitle>
            <AlertDialogDescription>
              {externalConfirm === 'restart'
                ? '将优雅排空在途请求后，重启监听端口上的外部网关，最长等待 60 秒。'
                : '将优雅停止监听端口上的外部网关（先排空在途请求）。'}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="space-y-2 text-xs leading-5 text-muted-foreground">
            <p>该网关<span className="font-medium text-foreground">并非本窗口拉起</span>（外部实例）。</p>
            <p>仅在经 supervisor 父链核实其父进程后才会真正执行；无法核实则保持不动——宁可不杀，不可误杀。</p>
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => { const action = externalConfirm; setExternalConfirm(null); if (action) onAction(action) }}
            >
              {externalConfirm === 'restart' ? '确认重启' : '确认停止'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}