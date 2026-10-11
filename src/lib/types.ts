// 壳前端共享类型与常量。
// 术语对齐 GLOSSARY.md：活动事件 / 实例归属 / 受管网关 / 外部网关。
export type PageId = 'overview' | 'logs' | 'settings'
export type GatewayState = 'running' | 'stopped' | 'external' | 'error' | 'starting' | 'draining'
export type StableState = 'running' | 'stopped' | 'external' | 'error'
export type GatewayAction = 'start' | 'stop' | 'restart' | 'copy' | 'open-logs' | 'exit'
export interface GatewaySnapshot {
  state: GatewayState
  inflight: number | null
  uptime: string | null
  version: string | null
  // 实例归属（GLOSSARY）：当前窗口 | 外部进程 | 无。壳侧推导，始终有值。
  owner: 'window' | 'external' | 'none'
}
// 活动事件（GLOSSARY）：壳侧把英文诊断记录提炼成的分级中文事件。
// T6 定型：逐字对齐 T5 命令的 ActivityEvent（serde camelCase，最新在前，无 id）。
// 级别 token 用 info|warn|error；中文级别（信息/警告/错误）由展示层映射。
export interface ActivityEvent {
  level: 'info' | 'warn' | 'error'
  title: string
  detail: string
  time: string
}
export interface PageProps {
  snapshot: GatewaySnapshot
  events: ActivityEvent[]
  onAction: (action: GatewayAction) => void
  onNavigate: (page: PageId) => void
}
export interface SettingsPageProps extends PageProps {
  autostart: boolean
  onAutostartChange: (value: boolean) => void
}
export const ENDPOINT = 'http://127.0.0.1:8787'
export const DISPLAY_ENDPOINT = '127.0.0.1:8787'
export const APP_VERSION = 'v0.1.0'
export const stateNames: Record<GatewayState, string> = {
  running: '运行中', stopped: '未启动', external: '已有实例 · 只读',
  error: '端口冲突', starting: '正在启动', draining: '正在排空',
}
