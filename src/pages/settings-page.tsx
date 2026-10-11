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
import { DISPLAY_ENDPOINT, type SettingsPageProps } from '@/lib/types'
import {
  FALLBACK_APP_VERSION,
  getAppVersion,
  getAutostart,
  setAutostartEnabled,
} from '@/lib/gateway'

export function SettingsPage(props: SettingsPageProps) {
  const id = useId()
  const { onAction } = props

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
      className="min-w-0 space-y-6 text-sm text-foreground"
    >
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-2">
          <h1
            id={`${id}-heading`}
            className="text-[30px] font-semibold leading-9 tracking-[-0.025em]"
          >
            偏好设置
          </h1>
          <p className="text-sm leading-6 text-muted-foreground">
            管理启动偏好，查看连接与退出方式。
          </p>
        </div>
      </header>

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
              <p className="text-sm leading-6">先排空自有网关，再退出</p>
              <p className="text-xs leading-5 text-muted-foreground">
                排空等待预算为 10 秒，不代表整个退出流程的耗时上限。
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
                  value={DISPLAY_ENDPOINT}
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
              退出前会先优雅排空在途请求（最多等待 10 秒），再停止网关并退出。
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
