import { useCallback, useEffect, useState } from 'react'
import { getRecentActivity, watchActivity } from '@/lib/gateway'
import type { ActivityEvent, ActivityFeed, GatewaySnapshot } from '@/lib/types'

/** 总览与诊断页共用一条通知流；只有网关状态继续轮询，活动事件不轮询。 */
export function useActivity(owner: GatewaySnapshot['owner']) {
  const [events, setEvents] = useState<ActivityEvent[]>([])
  const [revision, setRevision] = useState(0)
  const [refreshKey, setRefreshKey] = useState(0)
  const [retryKey, setRetryKey] = useState(0)
  const [status, setStatus] = useState<ActivityFeed['status']>('connecting')
  const [message, setMessage] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const retry = useCallback(() => setRetryKey(value => value + 1), [])

  useEffect(() => {
    let latest = 0
    setStatus('connecting')
    setMessage(null)
    return watchActivity(notice => {
      // 错误是通道状态而非数据版本；IPC 失败不能被较新的数据版本过滤掉。
      if (notice.status === 'error') {
        setStatus('error')
        setMessage(notice.message)
        return
      }
      // 每个订阅从0计数，历史快照不增加revision；非零首响应只可能是快照后真正新增。
      if (notice.revision < latest) return
      const added = notice.revision - latest
      if (added > 0) setRevision(value => value + added)
      latest = notice.revision
      setStatus(notice.status)
      setMessage(notice.message)
      setRefreshKey(value => value + 1)
    })
  }, [owner, retryKey])

  useEffect(() => {
    let ignore = false
    setLoading(true)
    void getRecentActivity().then(list => {
      if (ignore) return
      setEvents(list)
      setError(null)
    }).catch(reason => {
      if (!ignore) setError(String(reason)) // 保留最后成功结果，不能误显示「暂无活动」。
    }).finally(() => {
      if (!ignore) setLoading(false)
    })
    return () => { ignore = true }
  }, [refreshKey, retryKey])

  const activity: ActivityFeed = { revision, refreshVersion: refreshKey, status, message, error, loading, retry }
  return { events, activity }
}
