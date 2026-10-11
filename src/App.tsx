import { useCallback, useEffect, useRef, useState } from 'react'
import {
  ActivityIcon, ArrowUpRightIcon, CheckIcon, CopyIcon, FileTextIcon, LogOutIcon, MonitorIcon,
  MoreHorizontalIcon, NetworkIcon, PlayIcon, RotateCwIcon, SearchIcon, Settings2Icon,
  ShieldAlertIcon, ShieldCheckIcon, SquareIcon, TerminalIcon,
} from 'lucide-react'
import { toast } from 'sonner'
import { Card, CardContent } from '@/components/ui/card'
import { Kbd } from '@/components/ui/kbd'
import { Button } from '@/components/ui/button'
import { Breadcrumb, BreadcrumbItem, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from '@/components/ui/breadcrumb'
import {
  Sidebar, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupContent,
  SidebarGroupLabel, SidebarHeader, SidebarInset, SidebarMenu, SidebarMenuButton,
  SidebarMenuItem, SidebarProvider, SidebarRail, SidebarTrigger, useSidebar,
} from '@/components/ui/sidebar'
import { Separator } from '@/components/ui/separator'
import {
  Command, CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList,
  CommandSeparator, CommandShortcut,
} from '@/components/ui/command'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
  AlertDialogFooter, AlertDialogHeader, AlertDialogMedia, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { TooltipProvider } from '@/components/ui/tooltip'
import { Input } from '@/components/ui/input'
import { Toaster } from '@/components/ui/sonner'
import { OverviewPage } from '@/pages/overview-page'
import { LogsPage } from '@/pages/logs-page'
import { SettingsPage } from '@/pages/settings-page'
import { ENDPOINT, stateNames } from '@/lib/types'
import type { ActivityEvent, GatewayAction, GatewayErrorInfo, GatewaySnapshot, PageId } from '@/lib/types'
import {
  getStatus, deriveSnapshot, getRecentActivity, getAppVersion, FALLBACK_APP_VERSION,
  startGateway, stopGateway, restartGateway, openLogsDir, quitApp, STATUS_POLL_MS,
} from '@/lib/gateway'

const pageNames: Record<PageId, string> = { overview: '网关总览', logs: '诊断日志', settings: '偏好设置' }
// start_gateway 失败原因 → 中文吐司（port_already_in_use 需先探测，单独处理，见 runStart）。
const START_FAILURE_TEXT: Record<string, string> = {
  node_missing: '自带 Node 运行时缺失，无法启动网关。',
  gateway_script_missing: '网关脚本缺失，无法启动网关。',
  spawn_failed: '拉起网关进程失败。',
  job_setup_failed: '进程树兜底建立失败，已回滚。',
}
// 端口冲突错误态详情（T7 错误态；端口被占且探测不可达时持久呈现于总览）。
const PORT_CONFLICT_DETAIL = '另一个程序正在占用 8787 端口，且未响应网关探测。请关闭占用该端口的程序后重试；本页面不会强制结束占用进程。'
const navigation = [
  { id: 'overview' as const, name: '网关总览', icon: ActivityIcon },
  { id: 'logs' as const, name: '诊断日志', icon: FileTextIcon },
  { id: 'settings' as const, name: '偏好设置', icon: Settings2Icon },
]

function AppSidebar({ page, onNavigate, onSearch, onAction, appVersion }: {
  page: PageId; onNavigate: (page: PageId) => void; onSearch: () => void;
  onAction: (action: GatewayAction) => void; appVersion: string;
}) {
  const { isMobile, setOpenMobile } = useSidebar()
  const navigate = (next: PageId) => { onNavigate(next); if (isMobile) setOpenMobile(false) }
  return (
    <Sidebar collapsible="icon" className="border-r-0">
      <SidebarHeader className="gap-5 px-4 pb-5 pt-7 group-data-[collapsible=icon]:px-2">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" onClick={() => navigate('overview')} className="gap-3 rounded-[18px] hover:bg-transparent" tooltip="AG Relay · 本地网关">
              <div className="flex size-10 shrink-0 items-center justify-center rounded-[14px] border border-border bg-white text-foreground"><NetworkIcon className="size-5" strokeWidth={1.6} /></div>
              <div className="grid gap-0.5 text-left"><span className="text-lg font-semibold tracking-tight">AG Relay</span><span className="text-xs text-muted-foreground">本地网关工作空间</span></div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
        <Button variant="secondary" onClick={onSearch} className="h-10 w-full justify-between px-3 text-muted-foreground group-data-[collapsible=icon]:size-10 group-data-[collapsible=icon]:p-0 group-data-[collapsible=icon]:justify-center" aria-label="搜索页面和操作 Ctrl K">
          <span className="flex items-center gap-2"><SearchIcon className="size-4" /><span className="group-data-[collapsible=icon]:hidden">搜索与快捷操作</span></span>
          <Kbd className="bg-transparent font-mono text-xs group-data-[collapsible=icon]:hidden">⌃ K</Kbd>
        </Button>
      </SidebarHeader>
      <SidebarContent className="gap-5 px-2">
        <SidebarGroup>
          <SidebarGroupLabel className="mb-1 text-xs font-normal">工作空间</SidebarGroupLabel>
          <SidebarGroupContent><SidebarMenu className="gap-1.5">
            {navigation.map(item => <SidebarMenuItem key={item.id}>
              <SidebarMenuButton onClick={() => navigate(item.id)} isActive={page === item.id} tooltip={item.name} className="h-11 gap-3 px-3 text-sm" aria-current={page === item.id ? 'page' : undefined}>
                <item.icon className="size-4" strokeWidth={1.7} /><span>{item.name}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>)}
          </SidebarMenu></SidebarGroupContent>
        </SidebarGroup>
        <SidebarGroup>
          <SidebarGroupLabel className="mb-1 text-xs font-normal">工具</SidebarGroupLabel>
          <SidebarGroupContent><SidebarMenu className="gap-1.5">
            <SidebarMenuItem><SidebarMenuButton onClick={() => onAction('open-logs')} tooltip="日志文件夹" className="h-10 gap-3 px-3 text-muted-foreground"><TerminalIcon className="size-4" /><span>日志文件夹</span><ArrowUpRightIcon className="ml-auto size-3.5" /></SidebarMenuButton></SidebarMenuItem>
          </SidebarMenu></SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter className="gap-4 px-4 pb-5 group-data-[collapsible=icon]:px-2">
        <Card className="sidebar-note-card gap-0 py-0 group-data-[collapsible=icon]:hidden"><CardContent className="p-4">
          <div className="mb-2 flex items-center gap-2 text-sm font-medium"><ShieldCheckIcon className="size-4" />仅在本机运行</div>
          <p className="text-xs leading-relaxed text-muted-foreground">只管理这台设备上的连接。<br />关闭前，有序结束在途请求。</p>
        </CardContent></Card>
        <SidebarMenu><SidebarMenuItem>
          <DropdownMenu><DropdownMenuTrigger asChild><SidebarMenuButton size="lg" className="gap-3 rounded-[18px]" tooltip="本机设备菜单">
            <div className="flex size-9 shrink-0 items-center justify-center rounded-[12px] bg-muted"><MonitorIcon className="size-4" /></div>
            <div className="grid min-w-0 gap-0.5"><span className="text-sm font-medium">本机设备</span><span className="whitespace-nowrap text-xs text-muted-foreground">Windows · v{appVersion}</span></div><MoreHorizontalIcon className="ml-auto size-4" />
          </SidebarMenuButton></DropdownMenuTrigger>
            <DropdownMenuContent side="top" align="start" className="w-56">
              <DropdownMenuLabel>本地工作空间</DropdownMenuLabel><DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => navigate('settings')}><Settings2Icon />偏好设置</DropdownMenuItem>
              <DropdownMenuSeparator /><DropdownMenuItem onSelect={() => onAction('exit')}><SquareIcon />退出应用</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </SidebarMenuItem></SidebarMenu>
      </SidebarFooter>
      <SidebarRail aria-label="展开或收起导航" title="展开或收起导航" />
    </Sidebar>
  )
}

export default function App() {
  const [page, setPage] = useState<PageId>('overview')
  const [snapshot, setSnapshot] = useState<GatewaySnapshot>({ state: 'stopped', inflight: null, uptime: null, version: null, owner: 'none' })
  const [events, setEvents] = useState<ActivityEvent[]>([])
  const [commandOpen, setCommandOpen] = useState(false)
  const [copyFallbackOpen, setCopyFallbackOpen] = useState(false)
  // 破坏性操作二次确认（T8：集中到壳层，使命令面板 / 页面按钮 / 侧栏菜单共用同一组守卫）。
  //  - confirmRestart：受管网关「安全重启」确认（排空 + 60 秒上限）。
  //  - externalConfirm：外部网关「停止 / 重启」二次确认（ADR-0003，非只读）。
  //  - confirmExit：退出应用确认（受管/外部语义不同，退出前优雅停网关）。
  const [confirmRestart, setConfirmRestart] = useState(false)
  const [externalConfirm, setExternalConfirm] = useState<null | 'stop' | 'restart'>(null)
  const [confirmExit, setConfirmExit] = useState(false)
  // 变更型命令进行中的瞬态（受管启停/重启期间覆盖状态为 starting/draining，禁用控制并显示进度）。
  const [busy, setBusy] = useState<null | 'starting' | 'draining'>(null)
  // 网关生命周期错误态（T7，ADR-0003）：由变更型命令的真实失败结果驱动，在总览上持久、可操作地呈现。
  const [gatewayError, setGatewayError] = useState<GatewayErrorInfo | null>(null)
  // 运行版本「主显」：应用版本（Tauri app.getVersion()，浏览器回退 FALLBACK_APP_VERSION）。
  // 单一来源：侧栏、页脚、总览均渲染此状态；初值取同一回退常量，不再散落字面量。
  const [appVersion, setAppVersion] = useState(FALLBACK_APP_VERSION)
  const mainRef = useRef<HTMLElement>(null)

  // 立即刷新一次状态（变更型动作完成后调用，不等下一轮 3 秒轮询）。
  const refreshStatus = useCallback(async () => {
    setSnapshot(deriveSnapshot(await getStatus()))
  }, [])

  // 状态轮询：挂载即首刷，此后每 STATUS_POLL_MS 刷新（8787 上有网关即接管显示）。
  // 「最近活动」随同一轮询并行拉取（与 getStatus 各自独立更新、互不阻塞）：启动后新产生的
  // 事件无需重启面板即可点亮；读取层只读、失败兜空，不改变 status 的轮询节奏。
  useEffect(() => {
    let active = true
    const tick = () => {
      void getStatus().then(status => {
        if (!active) return
        setSnapshot(deriveSnapshot(status))
        // 端口冲突态在网关恢复可达后自动消解（其余错误条保留到用户重试或忽略）。
        if (status.available) setGatewayError(prev => (prev?.portConflict ? null : prev))
      })
      void getRecentActivity().then(list => {
        if (active) setEvents(list)
      })
    }
    tick()
    const timer = setInterval(tick, STATUS_POLL_MS)
    return () => { active = false; clearInterval(timer) }
  }, [])

  // 应用版本（主显运行版本）：挂载时取一次。
  useEffect(() => {
    let active = true
    void getAppVersion().then(v => { if (active) setAppVersion(v) })
    return () => { active = false }
  }, [])

  useEffect(() => { document.title = `${pageNames[page]} · AG Relay` }, [page])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); setCommandOpen(value => !value) }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const navigate = (next: PageId) => { setPage(next); setCommandOpen(false); mainRef.current?.scrollTo({ top: 0 }) }

  const copyEndpoint = async () => {
    try { await navigator.clipboard.writeText(ENDPOINT); toast('连接地址已复制', { description: ENDPOINT, icon: <CheckIcon className="size-4" /> }) }
    catch { setCopyFallbackOpen(true) }
  }

  // 受管/外部网关 变更型动作（T2 接真）：启停/重启走真实命令，依结果给中文吐司（绝不伪造成功），
  // 完成后立即刷新状态（不等下一轮轮询）。starting/draining 为命令期间的瞬态 UI。
  const runStart = async () => {
    setGatewayError(null)
    setBusy('starting')
    try {
      const r = await startGateway()
      if (r.kind === 'started') {
        toast('网关已启动', { icon: <CheckIcon className="size-4" /> })
      } else if (r.kind === 'failed') {
        if (r.reason === 'port_already_in_use') {
          // 端口被占：先探测——可达说明已有网关在跑（接管显示，非错误），不可达才是占用失败
          const status = await getStatus()
          if (status.available) {
            toast('检测到已有网关在运行，已接管显示其状态。')
          } else {
            toast.error('端口 8787 已被占用，网关启动失败。')
            setGatewayError({ portConflict: true, title: '端口 8787 已被占用', detail: PORT_CONFLICT_DETAIL })
          }
        } else {
          const text = START_FAILURE_TEXT[r.reason] ?? `启动失败：${r.reason}`
          toast.error(text)
          setGatewayError({ portConflict: false, title: '网关启动失败', detail: text })
        }
      }
    } finally {
      setBusy(null)
      void refreshStatus()
    }
  }

  const runStop = async () => {
    setGatewayError(null)
    setBusy('draining')
    try {
      const r = await stopGateway()
      switch (r.kind) {
        case 'stopped': toast(r.graceful ? '网关已优雅停止。' : '网关已停止（超时后强制结束）。'); break
        case 'stopped_external': toast(r.graceful ? '已优雅停止外部网关实例。' : '已强制停止外部网关实例。'); break
        case 'external_stop_abandoned': toast('检测到外部网关，但归属无法核实，未执行停止。'); break
        case 'not_running': toast('当前没有正在运行的网关。'); break
        case 'failed':
          toast.error(`停止失败：${r.reason}`)
          setGatewayError({ portConflict: false, title: '停止网关失败', detail: `停止网关时出错：${r.reason}。请稍后重试，或打开日志文件夹查看详情。` })
          break
      }
    } finally {
      setBusy(null)
      void refreshStatus()
    }
  }

  const runRestart = async () => {
    setGatewayError(null)
    setBusy('draining')
    try {
      const r = await restartGateway()
      switch (r.kind) {
        case 'restarted': toast('已受理安全重启，排空后将自动重新拉起。'); break
        case 'not_running': toast('网关未运行，请先启动。'); break
        case 'timeout': toast('重启等待超时，可能仍在排空在途请求。'); break
        case 'failed':
          toast.error(`安全重启失败：${r.detail}`)
          setGatewayError({ portConflict: false, title: '安全重启失败', detail: `重启网关时出错：${r.detail}。请稍后重试，或打开日志文件夹查看详情。` })
          break
      }
    } finally {
      setBusy(null)
      void refreshStatus()
    }
  }

  const runOpenLogs = async () => {
    const r = await openLogsDir()
    if (r.kind === 'opened') toast('已打开日志文件夹。')
    else toast.error(`打开日志文件夹失败：${r.detail}`)
  }

  // 动作分发（T8 守卫中枢）：命令面板 / 页面按钮 / 侧栏菜单统一经此进入，确保三处共享同一组守卫。
  // 破坏性操作先弹二次确认，确认后才调真实命令（见下方 AlertDialog）；非破坏性动作直通。
  //  - 停止/重启外部网关 → 外部实例二次确认（ADR-0003）；受管重启 → 安全重启确认；退出 → 退出确认。
  const act = (action: GatewayAction) => {
    setCommandOpen(false)
    switch (action) {
      case 'copy': void copyEndpoint(); break
      case 'start': void runStart(); break
      case 'stop':
        if (snapshot.owner === 'external') setExternalConfirm('stop')
        else void runStop()
        break
      case 'restart':
        if (snapshot.owner === 'external') setExternalConfirm('restart')
        else setConfirmRestart(true)
        break
      case 'open-logs': void runOpenLogs(); break
      case 'exit': setConfirmExit(true); break
    }
  }

  // 命令进行中用瞬态状态覆盖显示（禁用控制并显示进度），底层 snapshot 仍由轮询持续更新。
  const base: GatewaySnapshot = busy ? { ...snapshot, state: busy } : snapshot
  // 端口冲突：无瞬态且底层未启动时，把大状态提升为「端口冲突」(error)，驱动总览错误态 UI（T7）。
  const view: GatewaySnapshot = !busy && gatewayError?.portConflict && base.state === 'stopped'
    ? { ...base, state: 'error' }
    : base
  // 命令面板动作可用性（与总览页按钮同源）：启动仅「未启动」可用；停止/重启 = 受管运行中 或 外部实例。
  const canStart = view.state === 'stopped'
  const canStopOrRestart = (view.state === 'running' && view.owner === 'window') || view.state === 'external'
  const common = { snapshot: view, events, onAction: act, onNavigate: navigate }
  return (
    <TooltipProvider delayDuration={200}>
      <SidebarProvider defaultOpen style={{ '--sidebar-width': '15rem', '--sidebar-width-icon': '4rem' } as React.CSSProperties}>
        <AppSidebar page={page} onNavigate={navigate} onSearch={() => setCommandOpen(true)} onAction={act} appVersion={appVersion} />
        <SidebarInset className="min-w-0 bg-background">
          <header className="app-header flex h-[72px] shrink-0 items-center justify-between gap-3 border-b border-border bg-white px-5 lg:px-8">
            <div className="flex min-w-0 items-center gap-3"><SidebarTrigger className="-ml-1 size-8 shrink-0" aria-label="展开或收起侧栏" /><Separator orientation="vertical" className="!h-4" />
              <Breadcrumb><BreadcrumbList className="text-sm"><BreadcrumbItem className="hidden sm:block">工作空间</BreadcrumbItem><BreadcrumbSeparator className="hidden sm:block" /><BreadcrumbItem><BreadcrumbPage>本地网关</BreadcrumbPage></BreadcrumbItem></BreadcrumbList></Breadcrumb>
            </div>
          </header>
          <main ref={mainRef} className="app-main min-w-0 flex-1 px-5 pb-7 pt-7 sm:px-6 lg:px-8 lg:pt-8" id="main-content">
            <div className="mx-auto w-full max-w-[1280px]">
              {page === 'overview' && <OverviewPage {...common} appVersion={appVersion} gatewayError={gatewayError} onDismissError={() => setGatewayError(null)} />}
              {page === 'logs' && <LogsPage {...common} />}
              {page === 'settings' && <SettingsPage {...common} />}
            </div>
          </main>
          <footer className="flex flex-wrap items-center justify-between gap-2 border-t border-border bg-white/60 px-6 py-3.5 text-xs text-muted-foreground lg:px-8"><span>AG Relay · 本地网关工作空间</span><span className="tabular-nums">v{appVersion}</span></footer>
        </SidebarInset>
      </SidebarProvider>
      <CommandDialog open={commandOpen} onOpenChange={setCommandOpen} title="搜索页面与快捷操作" description="搜索页面和可用操作。">
        <Command>
        <CommandInput aria-label="搜索页面或操作" placeholder="搜索页面或输入操作…" />
        <CommandList><CommandEmpty>没有找到匹配的页面或操作。</CommandEmpty>
          <CommandGroup heading="页面">{navigation.map(item => <CommandItem key={item.id} value={`${item.name} ${item.id}`} onSelect={() => navigate(item.id)}><item.icon /><span>{item.name}</span><CommandShortcut>↵</CommandShortcut></CommandItem>)}</CommandGroup>
          <CommandSeparator /><CommandGroup heading="网关操作">
            <CommandItem onSelect={() => act('copy')}><CopyIcon />复制连接地址</CommandItem>
            <CommandItem disabled={!canStart} onSelect={() => act('start')}><PlayIcon />启动网关</CommandItem>
            <CommandItem disabled={!canStopOrRestart} onSelect={() => act('restart')}><RotateCwIcon />安全重启</CommandItem>
            <CommandItem disabled={!canStopOrRestart} onSelect={() => act('stop')}><SquareIcon />停止网关</CommandItem>
            <CommandItem onSelect={() => act('open-logs')}><TerminalIcon />日志文件夹</CommandItem>
          </CommandGroup>
        </CommandList>
        </Command>
      </CommandDialog>
      <Dialog open={copyFallbackOpen} onOpenChange={setCopyFallbackOpen}><DialogContent><DialogHeader><DialogTitle>复制连接地址</DialogTitle><DialogDescription>系统暂不允许自动复制，请选中下方地址手动复制。</DialogDescription></DialogHeader><Input aria-label="手动复制的连接地址" value={ENDPOINT} readOnly autoFocus onFocus={event => event.currentTarget.select()} className="font-mono" /></DialogContent></Dialog>

      {/* 受管网关「安全重启」确认（排空 + 60 秒上限）。页面按钮与命令面板共用。 */}
      <AlertDialog open={confirmRestart} onOpenChange={setConfirmRestart}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>安全重启网关？</AlertDialogTitle>
            <AlertDialogDescription>
              将先优雅排空在途请求，再重启网关，最长等待 60 秒。网关受理重启不等于已恢复，恢复后面板状态会自动刷新。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction onClick={() => void runRestart()}>确认重启</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* 外部网关破坏性操作二次确认（ADR-0003：外部实例非只读，停止/重启前须确认）。页面按钮与命令面板共用。 */}
      <AlertDialog open={externalConfirm !== null} onOpenChange={(open) => { if (!open) setExternalConfirm(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogMedia>
              <ShieldAlertIcon aria-hidden="true" />
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
              onClick={() => { if (externalConfirm === 'restart') void runRestart(); else void runStop() }}
            >
              {externalConfirm === 'restart' ? '确认重启' : '确认停止'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* 退出应用确认（T3 语义，受管/外部网关退出不同）。侧栏菜单与偏好设置页的退出按钮共用。 */}
      <AlertDialog open={confirmExit} onOpenChange={setConfirmExit}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogMedia>
              <LogOutIcon aria-hidden="true" />
            </AlertDialogMedia>
            <AlertDialogTitle>退出 AG Relay？</AlertDialogTitle>
            <AlertDialogDescription>
              退出前会先优雅排空在途请求（最多等待 10 秒），再停止网关并退出应用。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="space-y-2 text-xs leading-5 text-muted-foreground">
            <p>
              <span className="font-medium text-foreground">受管网关</span>
              （本窗口拉起）：会被优雅停止，超时则强制结束其进程树。
              {view.owner === 'window' ? '当前实例由本窗口管理。' : ''}
            </p>
            <p>
              <span className="font-medium text-foreground">外部网关</span>
              （外部实例）：仅在核实其父进程后一并优雅停止，否则保持不动（宁可不杀，不可误杀）。
              {view.owner === 'external' ? '当前监听端口上为外部实例。' : ''}
            </p>
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => void quitApp()}>
              退出应用
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <Toaster theme="light" position="bottom-right" closeButton />
      <div className="sr-only" aria-live="polite" aria-atomic="true">{stateNames[view.state]}。{view.inflight === null ? '暂无运行数据' : `在途请求 ${view.inflight} 个`}</div>
    </TooltipProvider>
  )
}




