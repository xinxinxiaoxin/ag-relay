import { useEffect, useRef, useState } from 'react'
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
import { getStatus, deriveSnapshot, getRecentEvents, STATUS_POLL_MS } from '@/lib/gateway'

const pageNames: Record<PageId, string> = { overview: '网关总览', logs: '诊断日志', settings: '偏好设置' }
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
  const [snapshot, setSnapshot] = useState<GatewaySnapshot>({ state: 'stopped', inflight: null, uptime: null, version: null, owner: null })
  const [events, setEvents] = useState<ActivityEvent[]>([])
  const [autostart, setAutostart] = useState(false)
  const [commandOpen, setCommandOpen] = useState(false)
  const [copyFallbackOpen, setCopyFallbackOpen] = useState(false)
  const mainRef = useRef<HTMLElement>(null)

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

  // T1：变更型动作暂为占位（toast）。真实接线：start/stop/restart/open-logs（T2）、exit（T3）、命令面板/吐司（T8）。
  const act = (action: GatewayAction) => {
    setCommandOpen(false)
    switch (action) {
      case 'copy': void copyEndpoint(); break
      case 'start': toast('占位：启动网关将在后续版本接入'); break
      case 'stop': toast('占位：停止网关将在后续版本接入'); break
      case 'restart': toast('占位：安全重启将在后续版本接入'); break
      case 'open-logs': toast('占位：打开日志文件夹将在后续版本接入'); break
      case 'exit': toast('占位：退出应用将在后续版本接入'); break
    }
  }

  const common = { snapshot, events, onAction: act, onNavigate: navigate }
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
              {page === 'overview' && <OverviewPage {...common} />}
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
      <div className="sr-only" aria-live="polite" aria-atomic="true">{stateNames[snapshot.state]}。{snapshot.inflight === null ? '暂无运行数据' : `在途请求 ${snapshot.inflight} 个`}</div>
    </TooltipProvider>
  )
}




