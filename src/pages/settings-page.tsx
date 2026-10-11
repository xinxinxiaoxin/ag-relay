import { useId } from 'react'
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
import {
  APP_VERSION,
  DISPLAY_ENDPOINT,
  type SettingsPageProps,
} from '@/lib/types'

export function SettingsPage(props: SettingsPageProps) {
  const id = useId()

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
                checked={props.autostart}
                onCheckedChange={props.onAutostartChange}
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
                  onClick={() => props.onAction('copy')}
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
                <dd className="text-sm font-medium tabular-nums">{APP_VERSION}</dd>
              </dl>
              <div className="space-y-1.5">
                <h3 className="text-sm font-medium leading-5">外部实例</h3>
                <p className="text-xs leading-5 text-muted-foreground">
                  只读查看，不接管、不停止、不重启。
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
              退出前先排空自有网关，外部实例不受此操作影响。
            </CardDescription>
          </div>
          <Button
            type="button"
            variant="outline"
            className="text-destructive hover:text-destructive"
            onClick={() => props.onAction('exit')}
          >
            <LogOut data-icon="inline-start" aria-hidden="true" />
            退出应用
          </Button>
        </CardContent>
      </Card>
    </section>
  )
}
