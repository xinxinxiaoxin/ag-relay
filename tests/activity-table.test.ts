import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { ActivityTable } from '../src/components/activity-table.tsx'

test('活动表格默认一条事件一行，解释按需展开，不猜测用量与速率', () => {
  const html = renderToStaticMarkup(createElement(ActivityTable, { events: [{ time: '14:18:36', level: 'info', title: '等待首段内容', detail: '很长的解释仅按需显示', requestId: 'fixture-001' }] }))
  assert.match(html, /<table/)
  assert.match(html, /等待首段内容/)
  assert.match(html, /aria-expanded="false"/)
  assert.doesNotMatch(html, /很长的解释|tok\/s|供应商|成本/)
  assert.equal((html.match(/<tr /g) ?? []).length, 2)
})


test('事件行直接显示既有尝试次数、HTTP 状态和简短结果', () => {
  const html = renderToStaticMarkup(createElement(ActivityTable, { events: [{ time: '14:18:36', level: 'warn', title: '尚未收到可用回复', detail: '隐藏的长解释', summary: '首段等待超时', attempt: 3, httpStatus: 200, requestId: 'fixture-001' }] }))
  for (const text of ['尝试', 'HTTP', '原因 / 结果', '首段等待超时', '200']) assert.match(html, new RegExp(text))
  assert.match(html, />3<\/td>/)
  assert.doesNotMatch(html, /隐藏的长解释/)
})
