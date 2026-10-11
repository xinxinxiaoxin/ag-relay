import { useCallback, useEffect, useRef, useState } from 'react'
import {
  ActivityIcon, ArrowUpRightIcon, CheckIcon, CopyIcon, FileTextIcon, MonitorIcon,
  MoreHorizontalIcon, NetworkIcon, PlayIcon, RotateCwIcon, SearchIcon, Settings2Icon,
  ShieldCheckIcon, SquareIcon, TerminalIcon,
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
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { TooltipProvider } from '@/components/ui/tooltip'
import { Input } from '@/components/ui/input'
import { Toaster } from '@/components/ui/sonner'
import { OverviewPage } from '@/pages/overview-page'
import { LogsPage } from '@/pages/logs-page'
import { SettingsPage } from '@/pages/settings-page'
import { ENDPOINT, APP_VERSION, stateNames } from '@/lib/types'
import type { ActivityEvent, GatewayAction, GatewaySnapshot, PageId } from '@/lib/types'
import {
  getStatus, deriveSnapshot, getRecentEvents, getAppVersion,
  startGateway, stopGateway, restartGateway, openLogsDir, STATUS_POLL_MS,
} from '@/lib/gateway'

const pageNames: Record<PageId, string> = { overview: '网关总览', logs: '诊断日志', settings: '偏好设置' }
// start_gateway 失败原因 → 中文吐司（port_already_in_use 需先探测，单独处理，见 runStart）。
const START_FAILURE_TEXT: Record<string, string> = {
  node_missing: '自带 Node 运行时缺失，无法启动网关。',
  gateway_script_missing: '网关脚本缺失，无法启动网关。',
  spawn_failed: '拉起网关进程失败。',
  job_setup_failed: '进程树兜底建立失败，已回滚。',
}
const navigation = [
  { id: 'overview' as const, name: '网关总览', icon: ActivityIcon },
  { id: 'logs' as const, name: '诊断日志', icon: FileTextIcon },
  { id: 'settings' as const, name: '偏好设置', icon: Settings2Icon },
]

function AppSidebar({ page, onNavigate, onSearch, onAction }: {
  page: PageId; onNavigate: (page: PageId) => void; onSearch: () => void;
  onAction: (action: GatewayAction) => void;
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
            <div className="grid min-w-0 gap-0.5"><span className="text-sm font-medium">本机设备</span><span className="whitespace-nowrap text-xs text-muted-foreground">Windows · {APP_VERSION}</span></div><MoreHorizontalIcon className="ml-auto size-4" />
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
  const [autostart, setAutostart] = useState(false)
  const [commandOpen, setCommandOpen] = useState(false)
  const [copyFallbackOpen, setCopyFallbackOpen] = useState(false)
  // 变更型命令进行中的瞬态（受管启停/重启期间覆盖状态为 starting/draining，禁用控制并显示进度）。
  const [busy, setBusy] = useState<null | 'starting' | 'draining'>(null)
  // 运行版本「主显」：应用版本（Tauri app.getVersion()，浏览器回退 0.1.0）。
  const [appVersion, setAppVersion] = useState('0.1.0')
  const mainRef = useRef<HTMLElement>(null)

  // 立即刷新一次状态（变更型动作完成后调用，不等下一轮 3 秒轮询）。
  const refreshStatus = useCallback(async () => {
    setSnapshot(deriveSnapshot(await getStatus()))
  }, [])

  // 状态轮询：挂载即首刷，此后每 STATUS_POLL_MS 刷新（8787 上有网关即接管显示）。
  useEffect(() => {
    let active = true
    const tick = async () => {
      const status = await getStatus()
      if (active) setSnapshot(deriveSnapshot(status))
    }
    void tick()
    const timer = setInterval(() => void tick(), STATUS_POLL_MS)
    return () => { active = false; clearInterval(timer) }
  }, [])

  // 应用版本（主显运行版本）：挂载时取一次。
  useEffect(() => {
    let active = true
    void getAppVersion().then(v => { if (active) setAppVersion(v) })
    return () => { active = false }
  }, [])

  // 最近活动事件（T1 占位：mock；真实命令在 T5）。
  useEffect(() => {
    let active = true
    void getRecentEvents().then(list => { if (active) setEvents(list) })
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
    setBusy('starting')
    try {
      const r = await startGateway()
      if (r.kind === 'started') {
        toast('网关已启动', { icon: <CheckIcon className="size-4" /> })
      } else if (r.kind === 'failed') {
        if (r.reason === 'port_already_in_use') {
          // 端口被占：先探测——可达说明已有网关在跑（接管显示，非错误），不可达才是占用失败
          const status = await getStatus()
          if (status.available) toast('检测到已有网关在运行，已接管显示其状态。')
          else toast.error('端口 8787 已被占用，网关启动失败。')
        } else {
          toast.error(START_FAILURE_TEXT[r.reason] ?? `启动失败：${r.reason}`)
        }
      }
    } finally {
      setBusy(null)
      void refreshStatus()
    }
  }

  const runStop = async () => {
    setBusy('draining')
    try {
      const r = await stopGateway()
      switch (r.kind) {
        case 'stopped': toast(r.graceful ? '网关已优雅停止。' : '网关已停止（超时后强制结束）。'); break
        case 'stopped_external': toast(r.graceful ? '已优雅停止外部网关实例。' : '已强制停止外部网关实例。'); break
        case 'external_stop_abandoned': toast('检测到外部网关，但归属无法核实，未执行停止。'); break
        case 'not_running': toast('当前没有正在运行的网关。'); break
        case 'failed': toast.error(`停止失败：${r.reason}`); break
      }
    } finally {
      setBusy(null)
      void refreshStatus()
    }
  }

  const runRestart = async () => {
    setBusy('draining')
    try {
      const r = await restartGateway()
      switch (r.kind) {
        case 'restarted': toast('已受理安全重启，排空后将自动重新拉起。'); break
        case 'not_running': toast('网关未运行，请先启动。'); break
        case 'timeout': toast('重启等待超时，可能仍在排空在途请求。'); break
        case 'failed': toast.error(`安全重启失败：${r.detail}`); break
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

  const act = (action: GatewayAction) => {
    setCommandOpen(false)
    switch (action) {
      case 'copy': void copyEndpoint(); break
      case 'start': void runStart(); break
      case 'stop': void runStop(); break
      case 'restart': void runRestart(); break
      case 'open-logs': void runOpenLogs(); break
      case 'exit': toast('退出功能将随偏好设置页一并接入。'); break
    }
  }

  // 命令进行中用瞬态状态覆盖显示（禁用控制并显示进度），底层 snapshot 仍由轮询持续更新。
  const view: GatewaySnapshot = busy ? { ...snapshot, state: busy } : snapshot
  const common = { snapshot: view, events, onAction: act, onNavigate: navigate }
  return (
    <TooltipProvider delayDuration={200}>
      <SidebarProvider defaultOpen style={{ '--sidebar-width': '15rem', '--sidebar-width-icon': '4rem' } as React.CSSProperties}>
        <AppSidebar page={page} onNavigate={navigate} onSearch={() => setCommandOpen(true)} onAction={act} />
        <SidebarInset className="min-w-0 bg-background">
          <header className="app-header flex h-[72px] shrink-0 items-center justify-between gap-3 border-b border-border bg-white px-5 lg:px-8">
            <div className="flex min-w-0 items-center gap-3"><SidebarTrigger className="-ml-1 size-8 shrink-0" aria-label="展开或收起侧栏" /><Separator orientation="vertical" className="!h-4" />
              <Breadcrumb><BreadcrumbList className="text-sm"><BreadcrumbItem className="hidden sm:block">工作空间</BreadcrumbItem><BreadcrumbSeparator className="hidden sm:block" /><BreadcrumbItem><BreadcrumbPage>本地网关</BreadcrumbPage></BreadcrumbItem></BreadcrumbList></Breadcrumb>
            </div>
          </header>
          <main ref={mainRef} className="app-main min-w-0 flex-1 px-5 pb-7 pt-7 sm:px-6 lg:px-8 lg:pt-8" id="main-content">
            <div className="mx-auto w-full max-w-[1280px]">
              {page === 'overview' && <OverviewPage {...common} appVersion={appVersion} />}
              {page === 'logs' && <LogsPage {...common} />}
              {page === 'settings' && <SettingsPage {...common} autostart={autostart} onAutostartChange={value => { setAutostart(value); toast(value ? '占位：登录后启动将在后续版本接入' : '占位：已关闭（登录后启动将在后续版本接入）') }} />}
            </div>
          </main>
          <footer className="flex flex-wrap items-center justify-between gap-2 border-t border-border bg-white/60 px-6 py-3.5 text-xs text-muted-foreground lg:px-8"><span>AG Relay · 本地网关工作空间</span><span className="tabular-nums">{APP_VERSION}</span></footer>
        </SidebarInset>
      </SidebarProvider>
      <CommandDialog open={commandOpen} onOpenChange={setCommandOpen} title="搜索页面与快捷操作" description="搜索页面和可用操作。">
        <Command>
        <CommandInput aria-label="搜索页面或操作" placeholder="搜索页面或输入操作…" />
        <CommandList><CommandEmpty>没有找到匹配的页面或操作。</CommandEmpty>
          <CommandGroup heading="页面">{navigation.map(item => <CommandItem key={item.id} value={`${item.name} ${item.id}`} onSelect={() => navigate(item.id)}><item.icon /><span>{item.name}</span><CommandShortcut>↵</CommandShortcut></CommandItem>)}</CommandGroup>
          <CommandSeparator /><CommandGroup heading="网关操作">
            <CommandItem onSelect={() => act('copy')}><CopyIcon />复制连接地址</CommandItem>
            <CommandItem disabled={snapshot.state !== 'stopped'} onSelect={() => act('start')}><PlayIcon />启动网关</CommandItem>
            <CommandItem disabled={snapshot.state !== 'running'} onSelect={() => act('restart')}><RotateCwIcon />安全重启</CommandItem>
            <CommandItem disabled={snapshot.state !== 'running'} onSelect={() => act('stop')}><SquareIcon />停止网关</CommandItem>
            <CommandItem onSelect={() => act('open-logs')}><TerminalIcon />日志文件夹</CommandItem>
          </CommandGroup>
        </CommandList>
        </Command>
      </CommandDialog>
      <Dialog open={copyFallbackOpen} onOpenChange={setCopyFallbackOpen}><DialogContent><DialogHeader><DialogTitle>复制连接地址</DialogTitle><DialogDescription>系统暂不允许自动复制，请选中下方地址手动复制。</DialogDescription></DialogHeader><Input aria-label="手动复制的连接地址" value={ENDPOINT} readOnly autoFocus onFocus={event => event.currentTarget.select()} className="font-mono" /></DialogContent></Dialog>
      <Toaster theme="light" position="bottom-right" closeButton />
      <div className="sr-only" aria-live="polite" aria-atomic="true">{stateNames[view.state]}。{view.inflight === null ? '暂无运行数据' : `在途请求 ${view.inflight} 个`}</div>
    </TooltipProvider>
  )
}




