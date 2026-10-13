import { StrictMode, useSyncExternalStore } from 'react'
import { createRoot } from 'react-dom/client'
import '@fontsource-variable/geist'
import '@fontsource-variable/geist-mono'
import '../src/index.css'
import App from '../src/App'
import type { UpdateView } from '../src/lib/update'
import type { CloseRequest } from '../src/lib/window'

// 只有此独立 HTML 加载模拟桥；生产入口不引用本文件。禁止在原生窗口里运行。
if ('__TAURI__' in globalThis || '__TAURI_INTERNALS__' in globalThis) throw new Error('隔离 fixture 只能在普通浏览器中运行。')
globalThis.fetch = async () => { throw new Error('隔离 fixture 禁止网络请求，不能访问真实网关。') }
XMLHttpRequest.prototype.open = () => { throw new Error('隔离 fixture 禁止 XMLHttpRequest。') }

const scenarios = {
  available: '可用版本', idle: '暂无更新', checking: '正在重查',
  download0: '下载 0%', download40: '下载 40%', download100: '下载 100%',
  unknown: '未知总量', zero: '总量为零', overflow: '已下载超过总量',
  verifying: '签名校验', quiescing: '优雅排空', install_pending: '原生安装交接',
  failed: '下载失败', needs_attention: '恢复需处理',
} as const
type Scenario = keyof typeof scenarios
const candidate = { id: 'fixture-candidate-034', version: '0.3.4', notes: '隔离模拟更新说明，不会下载任何文件。\n<strong>这是纯文本，不执行 HTML。</strong>\n修复更新进度与关闭选择。' }
const base: UpdateView = { phase: 'available', currentVersion: '0.3.3', release: candidate, detail: '发现新版本 v0.3.4（隔离模拟）', recovery: null, download: null }
function makeView(scenario: Scenario): UpdateView {
  if (scenario.startsWith('download') || ['unknown', 'zero', 'overflow'].includes(scenario)) {
    const downloadedBytes = scenario === 'download0' ? 0 : scenario === 'download100' ? 100_000_000 : 40_000_000
    return { ...base, phase: 'downloading', detail: '正在下载更新包（隔离模拟字节，不联网）', download: {
      downloadedBytes, totalBytes: scenario === 'unknown' ? null : scenario === 'zero' ? 0 : scenario === 'overflow' ? 20_000_000 : 100_000_000,
    } }
  }
  switch (scenario) {
    case 'idle': return { ...base, phase: 'idle', release: null, detail: '当前已是最新版本（隔离模拟）' }
    case 'checking': return { ...base, phase: 'checking', release: null, detail: '正在检查版本（隔离模拟）' }
    case 'verifying': return { ...base, phase: 'verifying', detail: '正在校验完整更新包签名（隔离模拟）' }
    case 'quiescing': return { ...base, phase: 'quiescing', detail: '正在等待在途请求完成（隔离模拟）' }
    case 'install_pending': return { ...base, phase: 'install_pending', detail: '已交给原生安装器（隔离模拟，未启动安装器）' }
    case 'failed': return { ...base, phase: 'failed', detail: '下载失败：隔离模拟的网络异常' }
    case 'needs_attention': return { ...base, phase: 'needs_attention', detail: '更新恢复需要处理（隔离模拟）', recovery: '网关未自动恢复，请在检查后手动处理。' }
    default: return { ...base }
  }
}
let selected = new URLSearchParams(location.search).get('scenario') as Scenario
if (!(selected in scenarios)) selected = 'available'
let update = makeView(selected)
let pendingClose: CloseRequest | null = null
let closeId = 0
let exitMode = 'failure'
let finishExit: ((error?: string) => void) | null = null
let updateReads = 0
let log = '隔离就绪：没有访问真实网关'
let fixtureRevision = 0
const fixtureListeners = new Set<() => void>()
const repaint = () => { fixtureRevision++; fixtureListeners.forEach(listener => listener()) }
const subscribeFixture = (listener: () => void) => { fixtureListeners.add(listener); return () => { fixtureListeners.delete(listener) } }
const fixtureSnapshot = () => fixtureRevision
const events = new Map<string, Set<(event: { payload: unknown }) => void>>()
const emit = (name: string, payload: unknown) => events.get(name)?.forEach(handler => handler({ payload }))
const closeWindow = (trayAvailable: boolean) => {
  pendingClose ??= { id: ++closeId, trayAvailable }
  emit('window-close-requested', pendingClose)
  repaint()
}
const finish = async (action: string) => {
  log = `模拟操作 ${action} 已受理；没有执行真实退出或隐藏`
  repaint()
  if (exitMode === 'failure') throw new Error('排空超时，已取消退出并恢复接单（隔离模拟，不强杀）。')
  if (exitMode === 'hold') await new Promise<void>((resolve, reject) => {
    finishExit = error => { finishExit = null; repaint(); if (error) reject(new Error(error)); else resolve() }
    repaint()
  })
  pendingClose = null
}
const connection = { saved: { upstream: 'https://fixture.invalid', port: 18787 }, active: null, restartRequired: false, endpoint: 'http://127.0.0.1:18787' }

Object.assign(globalThis, { __TAURI__: {
  app: { getVersion: async () => '0.3.3' },
  core: { invoke: async (command: string, args: Record<string, unknown> = {}) => {
    switch (command) {
      case 'get_update_state': updateReads++; repaint(); return structuredClone(update)
      case 'check_for_update': {
        update = makeView('checking')
        log = '仅模拟检查更新：确认前不下载'
        repaint()
        await new Promise(resolve => setTimeout(resolve, 1500))
        update = makeView('available')
        repaint()
        return structuredClone(update)
      }
      case 'confirm_update':
        if (args.candidateId !== update.release?.id || !['available', 'deferred'].includes(update.phase)) throw new Error('模拟候选已失效')
        log = `仅模拟 confirm_update：${String(args.candidateId)}`
        update = makeView('download0')
        repaint()
        return structuredClone(update)
      case 'get_close_request': return pendingClose ? { ...pendingClose } : null
      case 'respond_to_close':
        if (!pendingClose || args.requestId !== pendingClose.id) throw new Error('关闭请求已失效（隔离模拟）')
        log = `模拟 respond_to_close：${String(args.requestId)} / ${String(args.action)}`
        repaint()
        if (args.action === 'cancel') { pendingClose = null; return }
        if (args.action === 'tray' && !pendingClose.trayAvailable) throw new Error('托盘不可用（隔离模拟）')
        return finish(String(args.action))
      case 'quit_app': return finish('quit_app')
      case 'get_status': return { available: false, version: null, activeRequests: null, uptimeText: null, owner: 'none' }
      case 'get_connection_settings': return connection
      case 'get_autostart': return { enabled: false, exe_path: null, notes: ['隔离模拟，不写注册表'] }
      case 'get_recent_activity': return []
      case 'query_activity_events': return { events: [], page: 0, pageSize: 50, hasMore: false }
      case 'watch_activity_events': return { revision: 0, status: 'unavailable', message: '隔离模拟，不读取诊断文件' }
      case 'stop_activity_watch': return
      default: throw new Error(`隔离 fixture 禁止命令：${command}`)
    }
  } },
  event: { listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
    let listeners = events.get(name)
    if (!listeners) { listeners = new Set(); events.set(name, listeners) }
    listeners.add(handler)
    return () => { listeners.delete(handler) }
  } },
  window: { getCurrentWindow: () => ({
    minimize: async () => { log = '模拟最小化，不改变窗口'; repaint() },
    toggleMaximize: async () => { log = '模拟最大化，不改变窗口'; repaint() },
    isMaximized: async () => false,
    close: async () => closeWindow(true),
    onResized: async () => () => {},
  }) },
} })

export function Fixture() {
  useSyncExternalStore(subscribeFixture, fixtureSnapshot)
  const controlClass = 'rounded border border-amber-900/40 bg-white px-2 py-1 text-xs text-black'
  return <>
    <aside aria-label="隔离模拟控制台" className="border-b-2 border-amber-700 bg-amber-100 px-3 py-2 text-xs text-amber-950">
      <strong className="block">隔离模拟验收 · 真实 App / 组件 · 不访问网关、不下载、不安装、不退出</strong>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <label>更新场景 <select id="fixture-scenario" className={controlClass} value={selected} onChange={event => { selected = event.target.value as Scenario; update = makeView(selected); repaint() }}>
          {Object.entries(scenarios).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select></label>
        <button id="fixture-close" className={controlClass} onClick={() => closeWindow(true)}>关闭窗口（有托盘）</button>
        <button id="fixture-no-tray" className={controlClass} onClick={() => closeWindow(false)}>关闭窗口（无托盘）</button>
        <label>关闭结果 <select id="fixture-exit-mode" className={controlClass} value={exitMode} onChange={event => { exitMode = event.target.value; repaint() }}>
          <option value="failure">失败</option><option value="hold">保持进行中</option><option value="success">模拟完成</option>
        </select></label>
        <button id="fixture-exit-failed" className={controlClass} onClick={() => emit('app-exit-failed', { detail: '托盘退出失败，已唤回窗口（隔离模拟）' })}>托盘退出失败事件</button>
        <button id="fixture-resolve-failure" className={controlClass} disabled={!finishExit} onClick={() => finishExit?.('排空超时，已取消退出（隔离模拟）')}>结束等待为失败</button>
        <button id="fixture-resolve-success" className={controlClass} disabled={!finishExit} onClick={() => finishExit?.()}>结束等待为模拟完成</button>
      </div>
      <p className="mt-1 break-all"><span id="fixture-poll-count">更新状态读取 {updateReads} 次</span> · <span id="fixture-call-log">{log}</span></p>
    </aside>
    <App />
  </>
}

createRoot(document.getElementById('root')!).render(<StrictMode><Fixture /></StrictMode>)
