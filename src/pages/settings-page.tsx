import { useEffect, useId, useState } from 'react'
import { Copy, LogOut } from 'lucide-react'

import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import { UpdatePanel, type UpdatePanelProps } from '@/components/update-panel'
import { type SettingsPageProps } from '@/lib/types'
import { connectionSettingsError, DEFAULT_CONNECTION, getConnectionSettings, saveConnectionSettings, type ConnectionSettingsView } from '@/lib/connection-settings'
import {
  FALLBACK_APP_VERSION,
  getAppVersion,
  getAutostart,
  setAutostartEnabled,
} from '@/lib/gateway'

export function SettingsPage(props: SettingsPageProps & { update: UpdatePanelProps }) {
  const id = useId()
  const { onAction } = props
  const [connection, setConnection] = useState<ConnectionSettingsView | null>(null)
  const [upstream, setUpstream] = useState(DEFAULT_CONNECTION.upstream)
  const [port, setPort] = useState(String(DEFAULT_CONNECTION.port))
  const [connectionBusy, setConnectionBusy] = useState(false)
  const [connectionError, setConnectionError] = useState('')
  const [savedNotice, setSavedNotice] = useState(false)
  const gatewayBusy = props.snapshot.state === 'starting' || props.snapshot.state === 'draining'

  useEffect(() => {
    let active = true
    void getConnectionSettings().then(view => {
      if (!active) return
      setConnection(view)
      setUpstream(view.saved.upstream)
      setPort(String(view.saved.port))
    }).catch(() => { if (active) setConnectionError('无法读取连接设置。请检查设置文件，或重新填写并保存。') })
    return () => { active = false }
  }, [])

  // 重启完成只刷新生效信息，不覆盖尚未保存的编辑内容。
  useEffect(() => {
    let active = true
    if (!gatewayBusy) void getConnectionSettings().then(view => {
      if (active) setConnection(view)
    }).catch(() => {})
    return () => { active = false }
  }, [gatewayBusy])

  const saveConnection = async () => {
    setConnectionError('')
    setSavedNotice(false)
    // 校验复用 lib 里的单一实现，避免文案与规则在页面与保存层重复。
    const draftError = connectionSettingsError(upstream, Number(port))
    if (draftError) {
      setConnectionError(draftError)
      return
    }
    setConnectionBusy(true)
    try {
      const view = await saveConnectionSettings({ upstream, port: Number(port) })
      setConnection(view)
      setUpstream(view.saved.upstream)
      setPort(String(view.saved.port))
      setSavedNotice(true)
    } catch (error) {
      setConnectionError(typeof error === 'string' ? error : '保存失败，请检查上游地址和端口，或确认设置目录可写。')
    } finally { setConnectionBusy(false) }
  }

  // 开机自启：挂载读 get_autostart 实况；切换写 set_autostart_enabled 后回读，往返与注册表一致（失败回弹）。
  const [autostart, setAutostart] = useState(false)
  const [autostartBusy, setAutostartBusy] = useState(false)

  useEffect(() => {
    let active = true
    void getAutostart().then((status) => {
      if (active) setAutostart(status.enabled)
    })
    return () => {
      active = false
    }
  }, [])

  const handleAutostartChange = async (next: boolean) => {
    setAutostartBusy(true)
    setAutostart(next) // 乐观反馈，随后以命令结果校正
    const result = await setAutostartEnabled(next)
    if (result.kind === 'ok') {
      setAutostart(result.status.enabled) // 以注册表实况为准
    } else {
      const status = await getAutostart() // 失败：回读真值，开关回弹
      setAutostart(status.enabled)
    }
    setAutostartBusy(false)
  }

  // 应用版本：Tauri 取 window.__TAURI__.app.getVersion()，浏览器回退常量；显示加 "v" 前缀。
  const [appVersion, setAppVersion] = useState(FALLBACK_APP_VERSION)
  useEffect(() => {
    let active = true
    void getAppVersion().then((version) => {
      if (active) setAppVersion(version)
    })
    return () => {
      active = false
    }
  }, [])

  return (
    <section
      aria-labelledby={`${id}-heading`}
      className="min-w-0 space-y-4 text-sm text-foreground"
    >
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-2">
          <h1
            id={`${id}-heading`}
            className="text-xl font-semibold tracking-tight"
          >
            偏好设置
          </h1>
          <p className="text-xs leading-5 text-muted-foreground">
            管理启动偏好，查看连接与退出方式。
          </p>
        </div>
      </header>

      <UpdatePanel {...props.update} />

      <Card className="min-w-0 rounded-[24px] ring-border [--card-spacing:1.25rem]">
        <CardHeader>
          <CardTitle><h2>网关连接设置</h2></CardTitle>
          <CardDescription>仅监听本机 127.0.0.1。密钥由客户端提供，本软件不保存账号或密钥。</CardDescription>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={event => { event.preventDefault(); void saveConnection() }}>
            <div className="space-y-2">
              <Label htmlFor={`${id}-upstream`}>上游服务地址</Label>
              <Input id={`${id}-upstream`} type="url" required value={upstream} onChange={event => setUpstream(event.target.value)} disabled={connectionBusy || gatewayBusy} spellCheck={false} autoComplete="off" aria-describedby={`${id}-connection-hint`} />
            </div>
            <div className="space-y-2">
              <Label htmlFor={`${id}-port`}>本机监听端口</Label>
              <Input id={`${id}-port`} inputMode="numeric" required value={port} onChange={event => setPort(event.target.value)} disabled={connectionBusy || gatewayBusy} className="max-w-52" />
            </div>
            <p id={`${id}-connection-hint`} className="text-xs leading-5 text-muted-foreground">上游地址支持 HTTP(S) 和路径前缀，不接受账号、查询参数或片段。保存只写入设置，不会自动重启或中断请求。</p>
            {connectionError && <p role="alert" className="text-sm text-destructive">{connectionError}</p>}
            <div role="status" className="space-y-2 text-sm">
              {connection?.restartRequired ? <p>连接设置已保存，需重启网关才生效；网关未运行时将在下次启动时应用。优雅排空最多等待 10 秒，超时不强杀，请稍后重试。</p> : savedNotice && <p>连接设置已保存，将在网关下次启动时使用。</p>}
              {connection?.restartRequired && <p className="text-xs text-muted-foreground">当前连接地址：{connection.endpoint}；应用后：http://127.0.0.1:{connection.saved.port}</p>}
            </div>
            <div className="flex flex-wrap gap-2">
              <Button type="submit" disabled={connectionBusy || gatewayBusy}>{connectionBusy ? '正在保存…' : '保存连接设置'}</Button>
              <Button type="button" variant="outline" disabled={connectionBusy || gatewayBusy} onClick={() => { setUpstream(DEFAULT_CONNECTION.upstream); setPort(String(DEFAULT_CONNECTION.port)) }}>填入出厂默认值</Button>
              {connection?.restartRequired && <Button type="button" variant="outline" disabled={connectionBusy || gatewayBusy || !['running', 'external'].includes(props.snapshot.state)} onClick={() => onAction('restart')}>安全重启并应用</Button>}
            </div>
          </form>
        </CardContent>
      </Card>

      <div className="grid min-w-0 grid-cols-[repeat(auto-fit,minmax(min(100%,20rem),1fr))] items-stretch gap-4">
        <Card className="min-w-0 rounded-[24px] ring-border [--card-spacing:1.25rem]">
          <CardHeader>
            <CardTitle className="text-base font-semibold leading-6">
              <h2>启动与关闭</h2>
            </CardTitle>
          </CardHeader>
          <CardContent className="min-w-0 space-y-5">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 space-y-1.5">
                <Label htmlFor={`${id}-autostart`} className="text-sm leading-5">
                  Windows 登录后启动
                </Label>
                <p
                  id={`${id}-autostart-hint`}
                  className="text-xs leading-5 text-muted-foreground"
                >
                  开启后随 Windows 登录自动启动 AG Relay
                </p>
              </div>
              <Switch
                id={`${id}-autostart`}
                checked={autostart}
                onCheckedChange={(value) => void handleAutostartChange(value)}
                disabled={autostartBusy}
                aria-describedby={`${id}-autostart-hint`}
                className="mt-0.5 shrink-0"
              />
            </div>

            <Separator />

            <div className="space-y-2">
              <h3 className="text-sm font-medium leading-5">关闭行为</h3>
              <p className="text-sm leading-6">关闭窗口时选择保留在托盘或彻底退出</p>
              <p className="text-xs leading-5 text-muted-foreground">
                保留在托盘不停止网关。彻底退出会暂停接收新请求，最多等待 10 分钟，支持取消等待且不强制中断已有请求；托盘不可用时不能隐藏窗口。
              </p>
            </div>
          </CardContent>
        </Card>

        <Card className="min-w-0 rounded-[24px] ring-border [--card-spacing:1.25rem]">
          <CardHeader>
            <CardTitle className="text-base font-semibold leading-6">
              <h2>连接与版本</h2>
            </CardTitle>
          </CardHeader>
          <CardContent className="min-w-0 space-y-5">
            <div className="space-y-2">
              <Label htmlFor={`${id}-endpoint`} className="text-sm leading-5">
                连接地址
              </Label>
              <div className="flex min-w-0 items-center gap-2">
                <Input
                  id={`${id}-endpoint`}
                  type="text"
                  value={props.endpoint}
                  readOnly
                  spellCheck={false}
                  className="min-w-0 flex-1 text-sm tabular-nums"
                />
                <Button
                  type="button"
                  variant="outline"
                  aria-label="复制连接地址"
                  onClick={() => onAction('copy')}
                >
                  <Copy data-icon="inline-start" aria-hidden="true" />
                  复制
                </Button>
              </div>
            </div>

            <Separator />

            <div className="space-y-4">
              <dl className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
                <dt className="text-sm text-muted-foreground">应用版本</dt>
                <dd className="text-sm font-medium tabular-nums">v{appVersion}</dd>
              </dl>
              <div className="space-y-1.5">
                <h3 className="text-sm font-medium leading-5">外部实例</h3>
                <p className="text-xs leading-5 text-muted-foreground">
                  端口上的外部网关会被接管显示状态，并保留停止与重启能力；破坏性操作前会二次确认，且仅在经 supervisor 父链核实归属后执行（宁可不杀，不可误杀）。
                </p>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>

      <Card className="min-w-0 rounded-[24px] ring-border [--card-spacing:1.25rem]">
        <CardContent className="flex flex-wrap items-center justify-between gap-x-6 gap-y-4">
          <div className="min-w-0 flex-1 basis-64 space-y-1.5">
            <CardTitle className="text-base font-semibold leading-6">
              <h2>退出应用</h2>
            </CardTitle>
            <CardDescription className="text-xs leading-5">
              退出时暂停接收新请求，显示等待进度；已有请求完成后才停止网关，可取消等待。
            </CardDescription>
          </div>
          <Button
            type="button"
            variant="outline"
            className="text-destructive hover:text-destructive"
            onClick={() => onAction('exit')}
          >
            <LogOut data-icon="inline-start" aria-hidden="true" />
            退出应用
          </Button>
        </CardContent>
      </Card>
    </section>
  )
}
