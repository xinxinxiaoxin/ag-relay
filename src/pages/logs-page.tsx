import { useEffect, useState } from 'react'
import { ChevronLeft, ChevronRight, FolderOpen, Search, X } from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { Separator } from '@/components/ui/separator'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { queryActivityEvents } from '@/lib/gateway'
import type { ActivityLevelFilter, ActivityPage } from '@/lib/gateway'
import { formatEventTime } from '@/lib/types'
import type { ActivityEvent, PageProps } from '@/lib/types'

const LEVEL_LABELS: Record<ActivityEvent['level'], string> = {
  info: '信息',
  warn: '警告',
  error: '错误',
}

const LEVEL_FILTERS: { value: ActivityLevelFilter; label: string }[] = [
  { value: 'all', label: '全部' },
  { value: 'info', label: '信息' },
  { value: 'warn', label: '警告' },
  { value: 'error', label: '错误' },
]

// 诊断日志页用服务端分页（T5 query_activity_events）：page 为 0 基，pageSize 1..=500。
const PAGE_SIZE = 10

export function LogsPage(props: PageProps) {
  const [level, setLevel] = useState<ActivityLevelFilter>('all')
  const [query, setQuery] = useState('')
  const [page, setPage] = useState(0)
  const [data, setData] = useState<ActivityPage | null>(null)
  const [loading, setLoading] = useState(true)

  const normalizedQuery = query.trim()
  const hasFilters = level !== 'all' || normalizedQuery.length > 0

  // 级别 / 关键词 / 页码变化即查询（服务端筛选分页）。cleanup 的 ignore 丢弃过期响应，防乱序。
  useEffect(() => {
    let ignore = false
    setLoading(true)
    void queryActivityEvents({ level, keyword: normalizedQuery, page, pageSize: PAGE_SIZE }).then(
      (result) => {
        if (!ignore) {
          setData(result)
          setLoading(false)
        }
      },
    )
    return () => {
      ignore = true
    }
  }, [level, normalizedQuery, page])

  const events = data?.events ?? []
  const hasMore = data?.hasMore ?? false
  const rangeStart = events.length > 0 ? page * PAGE_SIZE + 1 : 0
  const rangeEnd = page * PAGE_SIZE + events.length
  // 游标分页无总数：有上一页（page>0）或有下一页（hasMore）才显示分页条。
  const showPagination = page > 0 || hasMore

  function resetFilters() {
    setLevel('all')
    setQuery('')
    setPage(0)
  }

  return (
    <section aria-labelledby="logs-page-title" className="min-w-0 space-y-6 text-sm text-foreground">
      <header className="flex min-w-0 flex-wrap items-start justify-between gap-4">
        <div className="min-w-0 space-y-2">
          <h1 id="logs-page-title" className="text-[30px] leading-9 font-semibold tracking-[-0.025em]">
            诊断日志
          </h1>
          <p className="text-sm leading-6 text-muted-foreground">查看连接和生命周期记录。</p>
        </div>
        <Button type="button" variant="outline" onClick={() => props.onAction('open-logs')}>
          <FolderOpen aria-hidden="true" className="size-4" />
          日志文件夹
        </Button>
      </header>

      <Card className="min-w-0 gap-0 rounded-[24px] border border-border py-0 ring-0">
        <CardHeader className="gap-1 px-5 py-5">
          <CardTitle role="heading" aria-level={2} className="text-base font-semibold">
            事件记录
          </CardTitle>
          <CardDescription className="text-xs leading-5">
            按级别筛选、搜索并分页浏览活动事件。
          </CardDescription>
        </CardHeader>

        <Tabs
          value={level}
          onValueChange={(value) => {
            setLevel(value as ActivityLevelFilter)
            setPage(0)
          }}
          className="min-w-0 gap-0"
        >
          <div className="flex min-w-0 flex-wrap items-center gap-3 px-5 pb-5">
            <TabsList aria-label="按日志级别筛选" className="max-w-full shrink-0">
              {LEVEL_FILTERS.map((filter) => (
                <TabsTrigger key={filter.value} value={filter.value} className="px-3 text-sm">
                  {filter.label}
                </TabsTrigger>
              ))}
            </TabsList>

            <div className="relative min-w-0 flex-1 basis-52">
              <Search
                aria-hidden="true"
                className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
              />
              <Input
                type="text"
                inputMode="search"
                aria-label="搜索日志"
                placeholder="搜索事件或详情…"
                autoComplete="off"
                spellCheck={false}
                value={query}
                onChange={(event) => {
                  setQuery(event.target.value)
                  setPage(0)
                }}
                className="pl-9 pr-10 text-sm"
              />
              {query.length > 0 && (
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label="清除搜索"
                  className="absolute top-1/2 right-0.5 -translate-y-1/2 text-muted-foreground"
                  onClick={() => {
                    setQuery('')
                    setPage(0)
                  }}
                >
                  <X aria-hidden="true" className="size-3.5" />
                </Button>
              )}
            </div>
          </div>

          <TabsContent value={level} className="min-w-0">
            {events.length > 0 ? (
              <Table aria-label="连接和生命周期日志" className="min-w-[640px] table-fixed">
                <TableHeader className="border-t border-border bg-[#fafafa]">
                  <TableRow className="border-border hover:bg-transparent">
                    <TableHead scope="col" className="h-10 w-28 pl-5 text-xs text-muted-foreground">
                      时间
                    </TableHead>
                    <TableHead scope="col" className="h-10 w-24 text-xs text-muted-foreground">
                      级别
                    </TableHead>
                    <TableHead scope="col" className="h-10 w-40 text-xs text-muted-foreground">
                      事件
                    </TableHead>
                    <TableHead scope="col" className="h-10 pr-5 text-xs text-muted-foreground">
                      详情
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {events.map((event, index) => (
                    <TableRow key={`${event.time}-${index}`} className="border-border">
                      <TableCell className="py-4 pl-5 align-top text-xs leading-5 whitespace-normal text-muted-foreground tabular-nums">
                        <time dateTime={event.time}>{formatEventTime(event.time)}</time>
                      </TableCell>
                      <TableCell className="py-4 align-top">
                        <Badge
                          variant={event.level === 'error' ? 'destructive' : event.level === 'warn' ? 'outline' : 'secondary'}
                          className={event.level === 'info' ? 'text-xs text-muted-foreground' : 'text-xs'}
                        >
                          {LEVEL_LABELS[event.level]}
                        </Badge>
                      </TableCell>
                      <TableCell className="py-4 align-top text-sm leading-5 font-medium whitespace-normal wrap-break-word">
                        {event.title}
                      </TableCell>
                      <TableCell className="py-4 pr-5 align-top text-xs leading-5 whitespace-normal text-muted-foreground wrap-break-word">
                        {event.detail}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : loading ? (
              <div className="flex min-h-72 items-center justify-center px-5 py-12 text-sm text-muted-foreground">
                正在加载活动事件…
              </div>
            ) : (
              <Empty className="min-h-72 px-5 py-12">
                <EmptyHeader>
                  <EmptyTitle className="text-base">
                    {hasFilters ? '没有匹配的日志' : '暂无日志'}
                  </EmptyTitle>
                  <EmptyDescription className="text-sm leading-6">
                    {hasFilters ? '换个关键词，或查看全部级别。' : '连接和生命周期记录会显示在这里。'}
                  </EmptyDescription>
                </EmptyHeader>
                {hasFilters && (
                  <EmptyContent>
                    <Button type="button" variant="outline" size="sm" onClick={resetFilters}>
                      重置筛选
                    </Button>
                  </EmptyContent>
                )}
              </Empty>
            )}
          </TabsContent>
        </Tabs>

        <Separator />
        <CardFooter className="min-w-0 flex-wrap justify-between gap-3 px-5 py-4">
          <p role="status" aria-live="polite" aria-atomic="true" className="text-xs leading-5 text-muted-foreground tabular-nums">
            {events.length === 0 ? '0 条记录' : `显示第 ${rangeStart}–${rangeEnd} 条`}
          </p>
          {showPagination && (
            <nav aria-label="日志分页" className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={page === 0}
                onClick={() => setPage((current) => Math.max(0, current - 1))}
              >
                <ChevronLeft aria-hidden="true" className="size-4" />
                上一页
              </Button>
              <span className="text-xs text-muted-foreground tabular-nums">第 {page + 1} 页</span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={!hasMore}
                onClick={() => setPage((current) => current + 1)}
              >
                下一页
                <ChevronRight aria-hidden="true" className="size-4" />
              </Button>
            </nav>
          )}
        </CardFooter>
      </Card>
    </section>
  )
}
