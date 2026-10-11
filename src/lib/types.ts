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
  owner: 'window' | 'external' | null
}
// 活动事件（GLOSSARY）：壳侧把英文诊断记录提炼成的分级中文事件。
// T1 为占位结构；真实字段与来源在 T5/T6 落地。
export interface ActivityEvent {
  id: string
  time: string
  level: 'info' | 'warning' | 'error'
  title: string
  detail: string
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
