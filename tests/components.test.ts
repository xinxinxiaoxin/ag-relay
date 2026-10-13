import assert from 'node:assert/strict'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { Progress } from '../src/components/ui/progress.tsx'
import { UpdatePanel } from '../src/components/update-panel.tsx'
import type { UpdateState } from '../src/lib/update.ts'

const state: UpdateState = {
  view: { phase: 'downloading', currentVersion: '0.3.3', release: { id: 'a', version: '0.3.4', notes: '<script>不执行</script>' }, detail: '真实字节', recovery: null, download: { downloadedBytes: 40_000_000, totalBytes: 100_000_000 } },
  error: null, working: false, hasUpdate: true,
}

test('受控更新卡展示真实下载状态和纯文本说明，原生交接不冒充安装成功', () => {
  const render = (next: UpdateState) => renderToStaticMarkup(createElement(UpdatePanel, { state: next, onCheck: async () => {}, onConfirm: async () => {} }))
  const html = render(state)
  assert.match(html, /40% · 40.0 \/ 100.0 MB/)
  assert.match(html, /aria-valuenow="40"/)
  assert.match(html, /&lt;script&gt;不执行&lt;\/script&gt;/)
  const handingOff = render({ ...state, view: { ...state.view!, phase: 'install_pending' } })
  assert.match(handingOff, /原生安装器/)
  assert.doesNotMatch(handingOff, /安装成功|aria-valuenow="100"/)
})


test('真实 Progress 将 0/40/100 传给无障碍 Root，未知值不声称为 0%', () => {
  for (const value of [0, 40, 100]) {
    const html = renderToStaticMarkup(createElement(Progress, { value, 'aria-label': '下载进度' }))
    assert.match(html, new RegExp(`aria-valuenow="${value}"`))
    assert.match(html, new RegExp(`translateX\\(-${100 - value}%\\)`))
  }
  const unknown = renderToStaticMarkup(createElement(Progress, { value: null }))
  assert.doesNotMatch(unknown, /aria-valuenow/)
  assert.match(unknown, /data-state="indeterminate"/)
  assert.match(unknown, /animate-pulse/)
})
