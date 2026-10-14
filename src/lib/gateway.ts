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
import type { ActivityEvent, GatewaySnapshot, RequestProgress } from './types'

// get_status 的真实返回形状（camelCase，见现有 gateway_status.rs / 原生 main.js）。
// owner 为 T2 新增：实例归属，壳侧推导（status 接口本身无此字段）。
export interface StatusSnapshot {
  requests?: RequestProgress[] | null
  requestsTruncated?: boolean | null
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
  event?: {
    listen: <T>(name: string, handler: (event: { payload: T }) => void) => Promise<() => void>
  }
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

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const fn = tauriInvoke()
  if (!fn) throw new Error(`Tauri 运行时不可用，无法调用命令：${cmd}`)
  return fn<T>(cmd, args)
}

// 每 3 秒读取核心当前状态；不在前端推算请求进展或完成时间。
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
  requestsTruncated: false,
  requests: [
    { requestId: 'b8d74e30-demo-0001', phase: 'waiting_first_content', attempt: 3, elapsedMs: 65000, sinceLastProgressMs: null },
    { requestId: 'a2519c62-demo-0002', phase: 'reasoning', attempt: 1, elapsedMs: 24000, sinceLastProgressMs: 2000 },
    { requestId: 'f16b8a04-demo-0003', phase: 'text_output', attempt: 1, elapsedMs: 18000, sinceLastProgressMs: 500 },
  ],
}

// 浏览器回退 mock：译后活动事件样例（最新在前），混合 信息/警告/错误 三级并给够条数，
// 使诊断日志页在 vite 预览下可演示级别筛选 / 搜索 / 分页 / 空态。仅限开发渲染/截图。
const MOCK_EVENTS: ActivityEvent[] = [
  { time: '14:18:36', level: 'info', title: '等待首段内容', detail: '等待首段内容，上游已连接但还未返回可交付的内容；已用时 1分5秒；第 3 次尝试；尚未收到内容进展。', requestId: 'b8d74e30-demo-0001', summary: '尚未收到内容', attempt: 3, code: 'request_status', technicalDetail: '等待首个内容。' },
  { time: '14:18:31', level: 'info', title: '正在推理', detail: '最近收到推理内容，正文可能尚未开始；已用时 24秒；第 1 次尝试；距上次内容进展 2秒。', requestId: 'a2519c62-demo-0002', summary: '已用时 24秒', attempt: 1, code: 'request_status', technicalDetail: '正在输出推理。' },
  { time: '14:18:20', level: 'info', title: '回复已完整结束', detail: '已收到正常结束标记，本次流式回复完整结束。', requestId: 'd532abc1-demo-0004', summary: '已收到正常结束标记', attempt: 2, httpStatus: 200, code: 'sse_ended', technicalDetail: '流式响应结束：终止类型 正常完成，停滞 无停滞。' },
  { time: '14:18:16', level: 'warn', title: '尚未收到可用回复', detail: '回复交付前遇到问题：等待首个内容超时。是否继续尝试，请查看后续记录。', requestId: 'b8d74e30-demo-0001', summary: '首段等待超时', attempt: 2, code: 'precommit_failure', technicalDetail: '等待首个内容超时。' },
  { time: '14:18:10', level: 'info', title: '发起上游尝试', detail: '第 2 次向上游发起尝试。', requestId: 'b8d74e30-demo-0001', summary: '已发送至上游', attempt: 2, code: 'attempt_started' },
  { time: '14:18:02', level: 'warn', title: '客户端提前断开', detail: '客户端在回复完成前断开，可能是主动取消、超时或关闭了连接。', requestId: 'ea978c11-demo-0005', summary: '取消、超时或连接断开', attempt: 1, code: 'client_closed' },
  { time: '14:17:50', level: 'error', title: '上游未能完成回复', detail: '上游明确报告本次生成失败，可查看技术信息定位原因。', requestId: 'f49a8821-demo-0006', summary: '上游生成失败', attempt: 2, httpStatus: 200, code: 'sse_ended', technicalDetail: '流式响应结束：终止类型 失败终止，停滞 无停滞。' },
  { time: '14:17:31', level: 'info', title: '收到请求', detail: '客户端已把请求交给网关，开始处理。', technicalDetail: '收到客户端请求：/v1/responses。', requestId: 'b8d74e30-demo-0001', summary: '/v1/responses', attempt: 0, code: 'received' },
  { time: '14:17:20', level: 'info', title: '开始传回内容', detail: '已收到可交付的内容，开始向客户端传回回复。', requestId: 'd532abc1-demo-0004', summary: '开始返回内容', attempt: 2, httpStatus: 200, code: 'sse_committed' },
  { time: '14:17:10', level: 'info', title: '上游已响应', detail: '上游已接受本次连接，仍需等待内容；这还不代表回复完整结束。', requestId: 'd532abc1-demo-0004', summary: '已响应，等待内容', attempt: 2, httpStatus: 200, code: 'upstream_response', technicalDetail: '收到上游响应头：HTTP 200（成功）。' },
  { time: '14:17:05', level: 'info', title: '收到请求', detail: '客户端已把请求交给网关，开始处理。', technicalDetail: '收到客户端请求：/v1/responses。', requestId: 'd532abc1-demo-0004', summary: '/v1/responses', attempt: 0, code: 'received' },
  { time: '12:00:00', level: 'info', title: '网关已启动', detail: '网关已就绪，可以接收客户端请求。', summary: '就绪，可接收请求', code: 'gateway_started', technicalDetail: '网关监听就绪。' },
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
      requests: status.requests ?? null,
      requestsTruncated: status.requestsTruncated ?? null,
    }
  }
  return { state: 'stopped', inflight: null, uptime: null, version: null, owner: status.owner, requests: null, requestsTruncated: null }
}

// 应用版本（运行版本「主显」）：优先 Tauri app.getVersion()（取自 tauri.conf.json 的 version），
// 浏览器（无 window.__TAURI__）或权限/运行时异常时回退常量 "0.2.0"。返回裸语义版本（无 v 前缀）。
// SHARED CONTRACT（T3/合并者）：若将来必须走 Rust 命令，统一命名 get_app_version → String。
// settings-page 以此为初值，故导出。
export const FALLBACK_APP_VERSION = '0.2.0'
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

/** 安全重启（restart_gateway）：排空预算 10 秒，超时撤销不强杀；成功已确认新实例就绪。 */
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

/** 最近活动与历史分页使用同一读取服务；失败交给界面展示，不伪装成空列表。 */
export async function getRecentActivity(): Promise<ActivityEvent[]> {
  if (!isTauri()) return MOCK_EVENTS
  return invoke<ActivityEvent[]>('get_recent_activity')
}

/** 分页懒读：保留调用方筛选、页码；Tauri 内不以 mock 或空页掩盖失败。 */
export async function queryActivityEvents(query: ActivityQuery = {}): Promise<ActivityPage> {
  const page = query.page ?? 0
  const pageSize = query.pageSize ?? DEFAULT_PAGE_SIZE
  if (!isTauri()) return mockQueryActivityEvents(query, page, pageSize)
  return invoke<ActivityPage>('query_activity_events', { query })
}

export interface ActivityNotice {
  revision: number
  status: 'watching' | 'unavailable' | 'error'
  message: string | null
}

/** 先建立事件通道再取服务端快照；注销亦涵盖异步注册尚未完成的 StrictMode 卸载。 */
export function watchActivity(onNotice: (notice: ActivityNotice) => void): () => void {
  if (!isTauri()) {
    onNotice({ revision: 0, status: 'unavailable', message: '浏览器演示数据，不代表真实诊断文件。' })
    return () => {}
  }
  const subscription = crypto.randomUUID()
  let disposed = false
  let received = false
  let unlisten: (() => void) | undefined
  const stop = () => { void invoke('stop_activity_watch', { subscription }).catch(() => {}) }
  void (async () => {
    const listen = tauriGlobal()?.event?.listen
    if (!listen) throw new Error('活动事件通知通道不可用，请重试。')
    unlisten = await listen<ActivityNotice & { subscription: string }>('activity-changed', ({ payload }) => {
      if (!disposed && payload.subscription === subscription) {
        received = true
        onNotice(payload)
      }
    })
    if (disposed) { unlisten(); return }
    const initial = await invoke<ActivityNotice>('watch_activity_events', { subscription })
    if (disposed) stop()
    else if (!received) onNotice(initial) // 通知已到达时，迟到的首响应不能回滚错误/恢复状态。
  })().catch(error => {
    if (!disposed) onNotice({ revision: 0, status: 'error', message: String(error) })
  })
  return () => { disposed = true; unlisten?.(); stop() }
}

// 浏览器回退：对 MOCK_EVENTS 按级别 + 关键词过滤，再按 page/pageSize 切片（仅供开发渲染/截图）。
function mockQueryActivityEvents(query: ActivityQuery, page: number, pageSize: number): ActivityPage {
  const level = query.level ?? 'all'
  const keyword = (query.keyword ?? '').trim().toLocaleLowerCase()
  const filtered = MOCK_EVENTS.filter((event) => {
    if (level !== 'all' && event.level !== level) return false
    if (!keyword) return true
    return `${event.title} ${event.summary ?? ''} ${event.detail} ${event.technicalDetail ?? ''} ${event.requestId ?? ''} ${event.code ?? ''}`.toLocaleLowerCase().includes(keyword)
  })
  const start = page * pageSize
  const events = filtered.slice(start, start + pageSize)
  return { events, page, pageSize, hasMore: start + events.length < filtered.length }
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

/** 显式退出：排空超时取消退出；错误交给确认框原地展示，不吞掉 IPC 失败。 */
export async function quitApp(): Promise<void> {
  if (!isTauri()) throw new Error('退出操作仅在桌面应用中可用。')
  await invoke<void>('quit_app')
}
