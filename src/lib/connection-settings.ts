import { invoke, isTauri } from './gateway'

export interface ConnectionSettings { upstream: string; port: number }
export interface ConnectionSettingsView {
  saved: ConnectionSettings
  active: ConnectionSettings | null
  restartRequired: boolean
  endpoint: string
}

export const DEFAULT_CONNECTION: ConnectionSettings = { upstream: 'https://agentrouter.org', port: 8787 }
let previewSaved = { ...DEFAULT_CONNECTION }

/** 浏览器只作编辑演示，不落盘，不以 mock 结果代表真实配置。 */
export async function getConnectionSettings(): Promise<ConnectionSettingsView> {
  if (isTauri()) return invoke<ConnectionSettingsView>('get_connection_settings')
  return { saved: previewSaved, active: DEFAULT_CONNECTION, restartRequired: previewSaved.upstream !== DEFAULT_CONNECTION.upstream || previewSaved.port !== DEFAULT_CONNECTION.port, endpoint: 'http://127.0.0.1:8787' }
}

/** 前端单一校验实现：返回错误文案，合法返回 null。settings 页的即时校验与保存共用它，不再各写一份。 */
export function connectionSettingsError(upstream: string, port: number): string | null {
  let url: URL
  try {
    url = new URL(upstream)
  } catch {
    return '上游地址须为完整的 http:// 或 https:// 地址'
  }
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
    return '地址须为 HTTP(S)，不能包含账号、密钥、查询参数或片段'
  }
  if (!Number.isInteger(port) || port < 1 || port > 65535) return '本机端口须为 1 到 65535 的整数'
  return null
}

export async function saveConnectionSettings(settings: ConnectionSettings): Promise<ConnectionSettingsView> {
  if (isTauri()) return invoke<ConnectionSettingsView>('save_connection_settings', { upstream: settings.upstream, port: settings.port })
  const error = connectionSettingsError(settings.upstream, settings.port)
  if (error) throw new Error(error)
  const url = new URL(settings.upstream)
  previewSaved = { upstream: url.href.replace(/\/+$/, ''), port: settings.port }
  return getConnectionSettings()
}
