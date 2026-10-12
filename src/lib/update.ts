import { FALLBACK_APP_VERSION, invoke, isTauri } from './gateway'

export interface UpdateView {
  phase: 'disabled' | 'idle' | 'checking' | 'available' | 'downloading' | 'verified'
    | 'quiescing' | 'deferred' | 'install_pending' | 'failed' | 'needs_attention'
  currentVersion: string
  release: { id: string; version: string; notes: string } | null
  detail: string
  recovery: string | null
}

// 普通浏览器不冒充一次真实更新检查，更不模拟安装成功。
const unavailable: UpdateView = {
  phase: 'disabled', currentVersion: FALLBACK_APP_VERSION, release: null,
  detail: '浏览器预览不支持应用内更新，请在已安装的 AG Relay 中检查。', recovery: null,
}

export async function getUpdateView(): Promise<UpdateView> {
  return isTauri() ? invoke<UpdateView>('get_update_state') : unavailable
}

export async function checkForUpdate(): Promise<UpdateView> {
  return isTauri() ? invoke<UpdateView>('check_for_update') : unavailable
}

export async function confirmUpdate(candidateId: string): Promise<UpdateView> {
  // Rust 只接受候选编号，不接受下载 URL、安装路径、签名或包字节。
  if (!isTauri()) return unavailable
  return invoke<UpdateView>('confirm_update', { candidateId })
}
