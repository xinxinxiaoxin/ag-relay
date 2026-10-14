import { Fragment, useState } from 'react'
import { ChevronDown } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { formatEventTime, type ActivityEvent } from '@/lib/types'

const levels = { info: '信息', warn: '警告', error: '错误' }

/** 只渲染已读到的事件；展开不触发网络请求，日志页可同时冻结自动更新。 */
export function ActivityTable({ events, onRequest, onExpand }: { events: ActivityEvent[]; onRequest?: (id: string) => void; onExpand?: () => void }) {
  const [expanded, setExpanded] = useState<string | null>(null)
  return <Table data-density="compact" aria-label="请求与网关活动" className="min-w-[700px] text-xs">
    <TableHeader className="bg-muted/35"><TableRow>
      <TableHead scope="col" className="h-9 w-24 pl-4 text-xs">时间</TableHead>
      <TableHead scope="col" className="h-9 w-24 text-xs">请求</TableHead>
      <TableHead scope="col" className="h-9 w-16 text-xs">级别</TableHead>
      <TableHead scope="col" className="h-9 text-xs">事件</TableHead>
      <TableHead scope="col" className="h-9 w-14 text-right text-xs">尝试</TableHead>
      <TableHead scope="col" className="h-9 w-16 text-right text-xs">HTTP</TableHead>
      <TableHead scope="col" className="h-9 text-xs">原因 / 结果</TableHead>
      <TableHead scope="col" className="h-9 w-12 text-xs"><span className="sr-only">详情</span></TableHead>
    </TableRow></TableHeader>
    <TableBody>{events.map((event, index) => {
      const key = `${event.requestId ?? ''}-${event.code ?? event.title}-${event.time}-${index}`
      const open = expanded === key
      return <Fragment key={key}>
        <TableRow>
          <TableCell className="py-2 pl-4 tabular-nums text-muted-foreground"><time dateTime={event.time}>{formatEventTime(event.time)}</time></TableCell>
          <TableCell className="py-2 font-mono text-muted-foreground">{event.requestId ? onRequest
            ? <button type="button" className="cursor-pointer underline decoration-border underline-offset-4 hover:text-foreground" title={event.requestId} aria-label={`查看请求 ${event.requestId} 的记录`} onClick={() => onRequest(event.requestId!)}>{event.requestId.slice(0, 8)}</button>
            : <span title={event.requestId}>{event.requestId.slice(0, 8)}</span> : '—'}</TableCell>
          <TableCell className="py-2"><Badge variant={event.level === 'error' ? 'destructive' : 'secondary'} className="text-[11px] font-normal">{levels[event.level]}</Badge></TableCell>
          <TableCell className="py-2 font-medium">{event.title}</TableCell>
          <TableCell className="py-2 text-right tabular-nums">{event.attempt ?? '—'}</TableCell>
          <TableCell className="py-2 text-right tabular-nums text-muted-foreground">{event.httpStatus ?? '—'}</TableCell>
          <TableCell className="py-2 text-muted-foreground">{event.summary || '—'}</TableCell>
          <TableCell className="py-1 pr-3 text-right"><Button variant="ghost" size="icon-xs" aria-expanded={open} aria-label={`查看${event.title}详情`} onClick={() => { setExpanded(open ? null : key); if (!open) onExpand?.() }}><ChevronDown className={open ? 'rotate-180' : ''} aria-hidden="true" /></Button></TableCell>
        </TableRow>
        {open && <TableRow className="bg-muted/20"><TableCell colSpan={8} className="space-y-1 whitespace-normal px-4 py-3 text-xs leading-5 text-muted-foreground">
          <p className="text-foreground/85">{event.detail}</p>
          {event.technicalDetail && <p>{event.technicalDetail}</p>}
          <p className="break-all font-mono text-[11px]">{event.code}{event.requestId ? ` · ${event.requestId}` : ''}</p>
        </TableCell></TableRow>}
      </Fragment>
    })}</TableBody>
  </Table>
}
