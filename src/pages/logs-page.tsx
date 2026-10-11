import { useState } from 'react'
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
import type { ActivityEvent, PageProps } from '@/lib/types'

const LEVEL_LABELS: Record<ActivityEvent['level'], string> = {
  info: '信息',
  warning: '警告',
  error: '错误',
}

const LEVEL_FILTERS = [
  { value: 'all', label: '全部' },
  { value: 'info', label: '信息' },
  { value: 'warning', label: '警告' },
  { value: 'error', label: '错误' },
] as const

type LevelFilter = (typeof LEVEL_FILTERS)[number]['value']

const PAGE_SIZE = 6

export function LogsPage(props: PageProps) {
  const [level, setLevel] = useState<LevelFilter>('all')
  const [query, setQuery] = useState('')
  const [page, setPage] = useState(1)
  const normalizedQuery = query.trim().toLocaleLowerCase()
  const hasFilters = level !== 'all' || normalizedQuery.length > 0

  const filteredEvents = props.events.filter((event) => {
    if (level !== 'all' && event.level !== level) return false

    return [event.time, event.level, LEVEL_LABELS[event.level], event.title, event.detail]
      .join(' ')
      .toLocaleLowerCase()
      .includes(normalizedQuery)
  })

  const pageCount = Math.max(1, Math.ceil(filteredEvents.length / PAGE_SIZE))
  const currentPage = Math.min(page, pageCount)
  const firstIndex = (currentPage - 1) * PAGE_SIZE
  const visibleEvents = filteredEvents.slice(firstIndex, firstIndex + PAGE_SIZE)

  function resetFilters() {
    setLevel('all')
    setQuery('')
    setPage(1)
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
            setLevel(value as LevelFilter)
            setPage(1)
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
                  setPage(1)
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
                    setPage(1)
                  }}
                >
                  <X aria-hidden="true" className="size-3.5" />
                </Button>
              )}
            </div>
          </div>

          <TabsContent value={level} className="min-w-0">
            {visibleEvents.length > 0 ? (
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
                  {visibleEvents.map((event) => (
                    <TableRow key={event.id} className="border-border">
                      <TableCell className="py-4 pl-5 align-top text-xs leading-5 whitespace-normal text-muted-foreground tabular-nums">
                        {event.time}
                      </TableCell>
                      <TableCell className="py-4 align-top">
                        <Badge
                          variant={event.level === 'error' ? 'destructive' : event.level === 'warning' ? 'outline' : 'secondary'}
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
            {filteredEvents.length === 0
              ? '0 条记录'
              : `显示 ${firstIndex + 1}–${firstIndex + visibleEvents.length} 条，共 ${filteredEvents.length} 条`}
          </p>
          {pageCount > 1 && (
            <nav aria-label="日志分页" className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={currentPage === 1}
                onClick={() => setPage(currentPage - 1)}
              >
                <ChevronLeft aria-hidden="true" className="size-4" />
                上一页
              </Button>
              <span className="text-xs text-muted-foreground tabular-nums">
                {currentPage} / {pageCount}
              </span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={currentPage === pageCount}
                onClick={() => setPage(currentPage + 1)}
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
