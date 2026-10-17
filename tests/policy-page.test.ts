import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { PolicyPage } from '../src/pages/policy-page.tsx'
import { createPolicyEditor } from '../src/lib/policy-settings.ts'
import type { GatewaySnapshot, PolicySettings } from '../src/lib/types.ts'

const defaults: PolicySettings = JSON.parse(readFileSync(new URL('./fixtures/policy-settings.json', import.meta.url), 'utf8'))
const snapshot: GatewaySnapshot = { state: 'running', owner: 'window', inflight: 1, uptime: '2 分钟', version: 'fixture', requests: [], requestsTruncated: false }

test('旧核心运行中不冒充策略已生效；停机显示下次启动；页面区分两组真实模块', async () => {
  Object.assign(globalThis, { __TAURI__: { core: { invoke: async () => ({ defaults, saved: defaults, active: null, activeRevision: null, activeSupported: false, restartRequired: false }) } } })
  try {
    const editor = createPolicyEditor()
    await editor.refresh()
    const render = (current = snapshot) => renderToStaticMarkup(createElement(PolicyPage, { editor, snapshot: current, onAction: () => {} }))
    const codex = render()
    assert.match(codex, /运行策略未核实/)
    assert.doesNotMatch(codex, /data-policy-status="applied"/)
    assert.match(codex, /首发可选编号清理/)
    assert.match(codex, /包含首次发送/)
    assert.match(codex, /关掉首发编号清理不等于禁止历史兼容降级删除编号/)
    assert.match(codex, /当前请求同类累计，不要求连续/)
    editor.editProfile('codex', { historyMode: 'always' })
    assert.match(render(), /每次首发完整预处理，不读写会话记忆/)
    assert.match(render(), /仅报错后模式参与/)
    editor.selectGroup('general')
    const general = render()
    assert.match(general, /空回应分级兼容/)
    assert.doesNotMatch(general, /data-policy-module="stripItemIds"/)
    assert.match(render({ ...snapshot, state: 'stopped', owner: 'none' }), /下次启动时应用/)
  } finally { Reflect.deleteProperty(globalThis, '__TAURI__') }
})
