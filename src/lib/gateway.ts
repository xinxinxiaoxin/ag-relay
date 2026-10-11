// Tauri IPC 抽象层（T1 奠基，后续前端工单复用）。
//
// 职责：
//  - 在 Tauri 运行时内（存在 window.__TAURI__.core.invoke）走真实命令层；
//  - 在浏览器（无 window.__TAURI__，如 vite dev/preview）回退到 mock 数据，
//    使设计能在浏览器里渲染、截图（仅限开发留档，绝不进真机逻辑）。
//
// 现有壳命令层见 src-tauri（lib.rs 的 invoke_handler 注册 8 个命令）。
// T1 只接读取（get_status）用于状态轮询；start/stop/restart/open-logs/quit 等
// 变更型动作在 UI 层暂为占位（toast），真实接线在 T2/T3/T8。
import type { ActivityEvent, GatewaySnapshot } from './types'

// get_status 的真实返回形状（camelCase，见现有 gateway_status.rs / 原生 main.js）。
// owner 为 T2 新增：实例归属，壳侧推导（status 接口本身无此字段）。
export interface StatusSnapshot {
  available: boolean
  version: string | null
  activeRequests: number | null
  uptimeText: string | null
  owner: 'window' | 'external' | 'none'
}

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>

interface TauriGlobal {
  core?: { invoke?: InvokeFn }
  app?: { getVersion?: () => Promise<string> }
}

function tauriGlobal(): TauriGlobal | null {
  return (globalThis as { __TAURI__?: TauriGlobal }).__TAURI__ ?? null
}

function tauriInvoke(): InvokeFn | null {
  return tauriGlobal()?.core?.invoke ?? null
}

/** 是否运行在 Tauri 运行时内（供浏览器回退判断）。 */
export function isTauri(): boolean {
  return tauriInvoke() !== null
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const fn = tauriInvoke()
  if (!fn) throw new Error(`Tauri 运行时不可用，无法调用命令：${cmd}`)
  return fn<T>(cmd, args)
}

// 状态轮询节奏，沿用原生壳：3 秒（网关 status 有约 20 秒内部缓存，数值不秒变属正常）。
export const STATUS_POLL_MS = 3000

// 命令层异常时的兜底：整体按未运行显示（永不抛错、不闪退）。
const OFFLINE_STATUS: StatusSnapshot = {
  available: false, version: null, activeRequests: null, uptimeText: null, owner: 'none',
}

// 浏览器回退 mock：复用定稿设计的「运行中」语义，仅用于开发渲染/截图。
// version 用网关脚本短哈希样例（8 位十六进制），与应用版本 getAppVersion() 区分开，
// 便于截图核对「运行版本」主/次两行语义不同。
const MOCK_STATUS: StatusSnapshot = {
  available: true, version: 'e8395e0f', activeRequests: 3, uptimeText: '2 小时 18 分', owner: 'window',
}

// 浏览器回退 mock：译后活动事件样例（最新在前），混合 信息/警告/错误 三级并给够条数，
// 使诊断日志页在 vite 预览下可演示级别筛选 / 搜索 / 分页 / 空态。仅限开发渲染/截图。
const MOCK_EVENTS: ActivityEvent[] = [
  { time: '14:18:36', level: 'info', title: '运行状态已更新', detail: '网关保持运行，当前有 3 个在途请求正在处理。' },
  { time: '14:12:04', level: 'warn', title: '上游响应变慢', detail: '一次转发的首字节延迟超过预期，已记录以便观察。' },
  { time: '14:05:51', level: 'error', title: '上游连接失败', detail: '与上游建立连接超时，该请求已按重试策略处理。' },
  { time: '13:58:20', level: 'info', title: '请求流式结束', detail: '一次 SSE 响应正常收尾，连接已释放。' },
  { time: '13:40:12', level: 'warn', title: '请求被限流', detail: '上游返回限流，网关已按退避策略稍后重试。' },
  { time: '13:22:47', level: 'info', title: '安全重启完成', detail: '排空在途请求后已重新拉起网关。' },
  { time: '13:21:30', level: 'info', title: '开始优雅排空', detail: '重启前等待在途请求自然结束，预算 10 秒。' },
  { time: '12:47:15', level: 'error', title: '请求异常中止', detail: '一次响应在完成前中断，客户端需重试。' },
  { time: '12:30:08', level: 'warn', title: '账号额度紧张', detail: '某上游账号接近额度上限，已切换到备用账号。' },
  { time: '12:15:42', level: 'info', title: '配置已加载', detail: '网关脚本配置解析完成，开始受理请求。' },
  { time: '12:00:01', level: 'info', title: '本地端点就绪', detail: '网关开始监听 127.0.0.1:8787。' },
  { time: '12:00:00', level: 'info', title: '网关启动完成', detail: '由当前窗口创建并管理此实例。' },
  { time: '11:59:59', level: 'info', title: '工作空间已准备', detail: '本地运行环境初始化完成。' },
  { time: '11:58:30', level: 'info', title: '诊断读取器就绪', detail: '已开始增量读取诊断记录，提炼为活动事件。' },
]

// ── 开发态 mock 场景（T7；仅浏览器预览/截图，绝不进真机逻辑）────────────────
// 通过 URL 查询参数 ?mock=<scenario> 切换浏览器回退数据，便于对「外部实例」态与
// 「错误态」截图留档。isTauri() 为真时恒返回 ''，真机路径（走真实命令）完全不受影响。
// 取值：running(默认) | external | stopped | port_conflict | start_fail | stop_fail | restart_fail。
export type MockScenario =
  | '' | 'running' | 'external' | 'stopped'
  | 'port_conflict' | 'start_fail' | 'stop_fail' | 'restart_fail'
function mockScenario(): MockScenario {
  if (isTauri()) return ''
  try {
    const search = typeof window !== 'undefined' ? window.location.search : ''
    return (new URLSearchParams(search).get('mock') ?? '') as MockScenario
  } catch {
    return ''
  }
}
// 外部实例态：可达但归属为「外部进程」（接管显示真实指标）。
const MOCK_EXTERNAL_STATUS: StatusSnapshot = {
  available: true, version: 'a1b2c3d4', activeRequests: 2, uptimeText: '1 小时 02 分', owner: 'external',
}
function mockStatusFor(scenario: MockScenario): StatusSnapshot {
  switch (scenario) {
    case 'external': return MOCK_EXTERNAL_STATUS
    case 'port_conflict': // 端口被占且探测不可达 → 未启动，错误态由 start 结果驱动
    case 'start_fail':
    case 'stopped': return OFFLINE_STATUS
    default: return MOCK_STATUS // running / stop_fail / restart_fail → 受管运行中
  }
}

/** 读取网关状态：Tauri 内走真实 get_status，浏览器回退 mock（可经 ?mock= 切场景）。 */
export async function getStatus(): Promise<StatusSnapshot> {
  if (!isTauri()) return mockStatusFor(mockScenario())
  try {
    return await invoke<StatusSnapshot>('get_status')
  } catch {
    return OFFLINE_STATUS
  }
}

// 把命令层的 StatusSnapshot 派生成 UI 的 GatewaySnapshot（T2：接真归属 + 版本次显 + 外部接管）。
//  - 可达 + 归属=外部进程 → 外部实例态（external，设计稿只读文案留给 T7 按 ADR-0003 改写）；
//  - 可达 + 其余（当前窗口 / 无法核实）→ 运行中；
//  - 不可达 → 未启动。
// 端口冲突态（error）由命令结果驱动（start_gateway 返回 port_already_in_use 且随后仍不可达），
// 不在轮询派生里臆测——留给 T7 的错误态 UI。
export function deriveSnapshot(status: StatusSnapshot): GatewaySnapshot {
  if (status.available) {
    return {
      state: status.owner === 'external' ? 'external' : 'running',
      inflight: status.activeRequests,
      uptime: status.uptimeText,
      version: status.version,
      owner: status.owner,
    }
  }
  return { state: 'stopped', inflight: null, uptime: null, version: null, owner: status.owner }
}

// 应用版本（运行版本「主显」）：优先 Tauri app.getVersion()（取自 tauri.conf.json 的 version），
// 浏览器（无 window.__TAURI__）或权限/运行时异常时回退常量 "0.1.0"。返回裸语义版本（无 v 前缀）。
// SHARED CONTRACT（T3/合并者）：若将来必须走 Rust 命令，统一命名 get_app_version → String。
// settings-page 以此为初值，故导出。
export const FALLBACK_APP_VERSION = '0.1.0'
export async function getAppVersion(): Promise<string> {
  const getVersion = tauriGlobal()?.app?.getVersion
  if (typeof getVersion === 'function') {
    try {
      const v = await getVersion()
      if (typeof v === 'string' && v.trim()) return v.trim()
    } catch {
      // 权限缺失 / 运行时异常 → 回退常量
    }
  }
  return FALLBACK_APP_VERSION
}

// 受管/外部网关 变更型动作（T2 接真；浏览器回退占位结果，使 vite 预览可点按截图）。
// 返回形状逐字对齐 Rust 命令枚举（serde tag="kind", snake_case），前端按 kind 分支给中文吐司。
export type GatewayActionResult =
  | { kind: 'started'; supervisor_pid: number }
  | { kind: 'stopped'; graceful: boolean; exit_code: number | null }
  | { kind: 'stopped_external'; graceful: boolean }
  | { kind: 'external_stop_abandoned' }
  | { kind: 'not_running' }
  | { kind: 'failed'; reason: string }

export type RestartResult =
  | { kind: 'restarted' }
  | { kind: 'not_running' }
  | { kind: 'timeout' }
  | { kind: 'failed'; detail: string }

export type LogsDirResult =
  | { kind: 'opened'; path: string }
  | { kind: 'failed'; detail: string }

/** 启动受管网关（start_gateway）。浏览器回退：默认「已启动」，?mock=port_conflict|start_fail 时回退失败结果。 */
export async function startGateway(): Promise<GatewayActionResult> {
  if (!isTauri()) {
    const s = mockScenario()
    if (s === 'port_conflict') return { kind: 'failed', reason: 'port_already_in_use' }
    if (s === 'start_fail') return { kind: 'failed', reason: 'node_missing' }
    return { kind: 'started', supervisor_pid: 0 }
  }
  try { return await invoke<GatewayActionResult>('start_gateway') }
  catch (e) { return { kind: 'failed', reason: `invoke_error: ${String(e)}` } }
}

/** 停止网关（stop_gateway；受管 + 外部 F1）。浏览器回退：默认「已优雅停止」，?mock=external|stop_fail 另有结果。 */
export async function stopGateway(): Promise<GatewayActionResult> {
  if (!isTauri()) {
    const s = mockScenario()
    if (s === 'stop_fail') return { kind: 'failed', reason: 'external_stop_failed' }
    if (s === 'external') return { kind: 'stopped_external', graceful: true }
    return { kind: 'stopped', graceful: true, exit_code: 0 }
  }
  try { return await invoke<GatewayActionResult>('stop_gateway') }
  catch (e) { return { kind: 'failed', reason: `invoke_error: ${String(e)}` } }
}

/** 安全重启（restart_gateway，60 秒上限）。浏览器回退：默认「已受理」，?mock=restart_fail 时回退失败。 */
export async function restartGateway(): Promise<RestartResult> {
  if (!isTauri()) {
    if (mockScenario() === 'restart_fail') return { kind: 'failed', detail: '排空超时后无法重新拉起（模拟）' }
    return { kind: 'restarted' }
  }
  try { return await invoke<RestartResult>('restart_gateway') }
  catch (e) { return { kind: 'failed', detail: String(e) } }
}

/** 打开日志文件夹（open_logs_dir）。浏览器回退：占位「已打开」。 */
export async function openLogsDir(): Promise<LogsDirResult> {
  if (!isTauri()) return { kind: 'opened', path: '（浏览器预览）日志文件夹' }
  try { return await invoke<LogsDirResult>('open_logs_dir') }
  catch (e) { return { kind: 'failed', detail: String(e) } }
}

// ── T6 活动事件查询（additive；绑定 T5 的两条命令，浏览器回退 mock） ──
// 形状逐字对齐 integration-notes 的 T5 契约：ActivityEvent(camelCase，最新在前) / ActivityPage。

/** 级别筛选 token（命令入参）：all + 三级（info|warn|error）。 */
export type ActivityLevelFilter = 'all' | ActivityEvent['level']

/** query_activity_events 入参（全可选；page 为 0 基，pageSize 取值 1..=500）。 */
export interface ActivityQuery {
  level?: ActivityLevelFilter
  keyword?: string
  page?: number
  pageSize?: number
}

/** query_activity_events 返回：当前页事件（最新在前）+ 分页游标。 */
export interface ActivityPage {
  events: ActivityEvent[]
  page: number
  pageSize: number
  hasMore: boolean
}

const DEFAULT_PAGE_SIZE = 50

/**
 * 最近活动事件（供总览「最近活动」）：Tauri 内走真实 get_recent_activity（最新在前，最多 50 条），
 * 浏览器回退 MOCK_EVENTS。命令虽为 Result（IO/路径错误返回 Ok([]) 即「暂无活动」），仍兜 promise
 * 失败 → 空列表，绝不以 mock 混入真机。
 */
export async function getRecentActivity(): Promise<ActivityEvent[]> {
  if (!isTauri()) return MOCK_EVENTS
  try {
    return await invoke<ActivityEvent[]>('get_recent_activity')
  } catch {
    return []
  }
}

/**
 * 分页查询活动事件（供诊断日志页）：Tauri 内走真实 query_activity_events（级别筛选 + 关键词 + 分页），
 * 浏览器回退对 MOCK_EVENTS 做等价的过滤/分页。失败兜底空页（保留请求的 page/pageSize）。
 */
export async function queryActivityEvents(query: ActivityQuery = {}): Promise<ActivityPage> {
  const page = query.page ?? 0
  const pageSize = query.pageSize ?? DEFAULT_PAGE_SIZE
  if (!isTauri()) return mockQueryActivityEvents(query, page, pageSize)
  try {
    return await invoke<ActivityPage>('query_activity_events', { query })
  } catch {
    return { events: [], page, pageSize, hasMore: false }
  }
}

// 浏览器回退：对 MOCK_EVENTS 按级别 + 关键词过滤，再按 page/pageSize 切片（仅供开发渲染/截图）。
function mockQueryActivityEvents(query: ActivityQuery, page: number, pageSize: number): ActivityPage {
  const level = query.level ?? 'all'
  const keyword = (query.keyword ?? '').trim().toLocaleLowerCase()
  const filtered = MOCK_EVENTS.filter((event) => {
    if (level !== 'all' && event.level !== level) return false
    if (!keyword) return true
    return `${event.time} ${event.title} ${event.detail}`.toLocaleLowerCase().includes(keyword)
  })
  const start = page * pageSize
  const events = filtered.slice(start, start + pageSize)
  return { events, page, pageSize, hasMore: start + events.length < filtered.length }
}

/**
 * 最近活动事件（兼容既有 App.tsx 调用方；委托 getRecentActivity）。
 * Tauri 内走真实 get_recent_activity，浏览器回退 MOCK_EVENTS。
 */
export async function getRecentEvents(): Promise<ActivityEvent[]> {
  return getRecentActivity()
}

// ── T3 偏好设置接线（additive；getAppVersion 与 T2 共享，合并时去重） ──

/** 开机自启状态（get_autostart 返回；字段原样 snake_case）。 */
export interface AutostartStatus {
  enabled: boolean
  exe_path: string | null
  notes: string[]
}

/** set_autostart_enabled 返回（serde tag=kind）。 */
export type AutostartSetResult =
  | { kind: 'ok'; status: AutostartStatus }
  | { kind: 'failed'; reason: string }

// 浏览器回退：默认未开启（仅供开发渲染/截图）。
const MOCK_AUTOSTART: AutostartStatus = { enabled: false, exe_path: null, notes: [] }

/** 读取开机自启实况：Tauri 内走真实 get_autostart，浏览器回退 mock。 */
export async function getAutostart(): Promise<AutostartStatus> {
  if (!isTauri()) return MOCK_AUTOSTART
  try {
    return await invoke<AutostartStatus>('get_autostart')
  } catch {
    return MOCK_AUTOSTART
  }
}

/**
 * 写开机自启（即时写注册表 Run 键）。调用方据结果回读实况：
 * ok → 用 status.enabled；failed → 另行 getAutostart() 回读真值并回弹。
 * 浏览器回退：回显 ok，使开关可在截图里切换。
 */
export async function setAutostartEnabled(
  enabled: boolean,
): Promise<AutostartSetResult> {
  if (!isTauri()) return { kind: 'ok', status: { enabled, exe_path: null, notes: [] } }
  try {
    return await invoke<AutostartSetResult>('set_autostart_enabled', { enabled })
  } catch (error) {
    return { kind: 'failed', reason: String(error) }
  }
}
// getAppVersion / FALLBACK_APP_VERSION 已与 T2 版本去重（见上方 getAppVersion 定义）。

/**
 * 面板退出：调用现有 quit_app（触发 RunEvent::Exit 优雅停网关 ≤10s——
 * 受管网关与经核实的外部网关一视同仁，再退出）。浏览器回退为空操作。
 */
export async function quitApp(): Promise<void> {
  if (!isTauri()) return
  try {
    await invoke<void>('quit_app')
  } catch {
    // 退出收尾时 IPC 可能断开，静默即可。
  }
}
