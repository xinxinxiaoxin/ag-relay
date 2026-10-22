import assert from 'node:assert/strict'
import test from 'node:test'
import { quitApp } from '../src/lib/gateway.ts'
import { respondToClose, watchWindowLifecycle, type CloseRequest } from '../src/lib/window.ts'

test('退出与关闭响应透传后端失败，普通浏览器不伪造成功', async () => {
  const calls: unknown[] = []
  Object.assign(globalThis, { __TAURI__: { core: { invoke: async (command: string, args: unknown) => {
    calls.push([command, args])
    throw new Error('排空超时，已取消退出')
  } } } })
  try {
    await assert.rejects(quitApp(), { message: '排空超时，已取消退出' })
    await assert.rejects(respondToClose(12, 'tray'), { message: '排空超时，已取消退出' })
    assert.deepEqual(calls, [['quit_app', undefined], ['respond_to_close', { requestId: 12, action: 'tray' }]])
  } finally {
    Reflect.deleteProperty(globalThis, '__TAURI__')
  }
  await assert.rejects(quitApp(), /桌面应用/)
  await assert.rejects(respondToClose(12, 'quit'), /桌面应用/)
})

test('退出与关闭响应完整透传具体中文拒绝原因，失败后仍可重试', async () => {
  const detail = '无法核实当前网关状态，未执行停机'
  const calls: unknown[] = []
  let rejectExit = true
  Object.assign(globalThis, { __TAURI__: { core: { invoke: async (command: string, args: unknown) => {
    calls.push([command, args])
    if (rejectExit) throw detail
  } } } })
  try {
    await assert.rejects(quitApp(), error => { assert.equal(error, detail); return true })
    await assert.rejects(respondToClose(13, 'quit'), error => { assert.equal(error, detail); return true })
    rejectExit = false
    await quitApp()
    await respondToClose(13, 'quit')
    assert.deepEqual(calls, [
      ['quit_app', undefined],
      ['respond_to_close', { requestId: 13, action: 'quit' }],
      ['quit_app', undefined],
      ['respond_to_close', { requestId: 13, action: 'quit' }],
    ])
  } finally {
    Reflect.deleteProperty(globalThis, '__TAURI__')
  }
})

test('关闭桥接完整传递失败原因后仍处理事件，迟到快照与重复请求不重开对话框', async () => {
  const handlers = new Map<string, (event: { payload: unknown }) => void>()
  let resolveInitial!: (value: CloseRequest) => void
  const initial = new Promise<CloseRequest>(done => { resolveInitial = done })
  const delivered: CloseRequest[] = []
  const failures: string[] = []
  const errors: unknown[] = []
  const detail = '排空已完成，但未能确认整个网关进程链退出；未强杀，请稍后核实'
  let subscribedAtRead = false
  Object.assign(globalThis, { __TAURI__: {
    core: { invoke: async () => { subscribedAtRead = handlers.has('window-close-requested') && handlers.has('app-exit-failed'); return initial } },
    event: { listen: async (name: string, callback: (event: { payload: unknown }) => void) => { handlers.set(name, callback); return () => { handlers.delete(name) } } },
  } })
  const stop = watchWindowLifecycle({ onClose: value => delivered.push(value), onExitFailed: value => failures.push(value), onError: error => errors.push(error) })
  try {
    await new Promise(resolve => setImmediate(resolve))
    handlers.get('app-exit-failed')!({ payload: { detail } })
    handlers.get('window-close-requested')!({ payload: { id: 2, trayAvailable: true } })
    handlers.get('window-close-requested')!({ payload: { id: 2, trayAvailable: true } })
    resolveInitial({ id: 1, trayAvailable: false })
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(subscribedAtRead, true)
    assert.deepEqual(failures, [detail])
    assert.deepEqual(delivered, [{ id: 2, trayAvailable: true }])
    handlers.get('app-exit-failed')!({ payload: { detail: '无法核实 supervisor 父链，未执行停机' } })
    handlers.get('window-close-requested')!({ payload: { id: 3, trayAvailable: false } })
    assert.deepEqual(failures, [detail, '无法核实 supervisor 父链，未执行停机'])
    assert.deepEqual(delivered, [{ id: 2, trayAvailable: true }, { id: 3, trayAvailable: false }])
    assert.deepEqual(errors, [])
  } finally {
    stop()
    Reflect.deleteProperty(globalThis, '__TAURI__')
  }
  assert.equal(handlers.size, 0)
})
