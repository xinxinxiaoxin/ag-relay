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
export interface StatusSnapshot {
  available: boolean
  version: string | null
  activeRequests: number | null
  uptimeText: string | null
}

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>

interface TauriGlobal {
  core?: { invoke?: InvokeFn }
}

function tauriInvoke(): InvokeFn | null {
  const g = (globalThis as { __TAURI__?: TauriGlobal }).__TAURI__
  return g?.core?.invoke ?? null
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
  available: false, version: null, activeRequests: null, uptimeText: null,
}

// 浏览器回退 mock：复用定稿设计的「运行中」语义，仅用于开发渲染/截图。
const MOCK_STATUS: StatusSnapshot = {
  available: true, version: 'v0.1.0', activeRequests: 3, uptimeText: '2 小时 18 分',
}

const MOCK_EVENTS: ActivityEvent[] = [
  { id: 'mock-1', time: '14:18:36', level: 'info', title: '运行状态已更新', detail: '网关保持运行，当前有在途请求正在处理。' },
  { id: 'mock-2', time: '12:00:01', level: 'info', title: '本地端点就绪', detail: '网关开始监听 127.0.0.1:8787。' },
  { id: 'mock-3', time: '12:00:00', level: 'info', title: '网关启动完成', detail: '由当前窗口创建并管理此实例。' },
  { id: 'mock-4', time: '11:59:59', level: 'info', title: '工作空间已准备', detail: '本地运行环境初始化完成。' },
]

/** 读取网关状态：Tauri 内走真实 get_status，浏览器回退 mock。 */
export async function getStatus(): Promise<StatusSnapshot> {
  if (!isTauri()) return MOCK_STATUS
  try {
    return await invoke<StatusSnapshot>('get_status')
  } catch {
    return OFFLINE_STATUS
  }
}

// 把命令层的 StatusSnapshot 派生成 UI 的 GatewaySnapshot。
// T1 占位派生：available → 运行中（当前窗口）；否则 未启动。
// 实例归属（external/无）、端口冲突态、版本主/次拆分留给 T2（见 frontend-map 警告），此处不实现。
export function deriveSnapshot(status: StatusSnapshot): GatewaySnapshot {
  if (status.available) {
    return {
      state: 'running',
      inflight: status.activeRequests,
      uptime: status.uptimeText,
      version: status.version,
      owner: 'window',
    }
  }
  return { state: 'stopped', inflight: null, uptime: null, version: null, owner: null }
}

/**
 * 最近活动事件（供总览「最近活动」与诊断日志页）。
 * T1 为占位：尚无对应命令（取/查活动事件是 T5 新增命令），两端均返回 mock。
 */
export async function getRecentEvents(): Promise<ActivityEvent[]> {
  // T5 落地后：Tauri 内改为 invoke 真实命令，浏览器仍回退 MOCK_EVENTS。
  return MOCK_EVENTS
}
