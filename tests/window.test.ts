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
  await assert.rejects(quitApp(), /排空超时/)
  await assert.rejects(respondToClose(12, 'tray'), /排空超时/)
  assert.deepEqual(calls, [['quit_app', undefined], ['respond_to_close', { requestId: 12, action: 'tray' }]])
  Reflect.deleteProperty(globalThis, '__TAURI__')
  await assert.rejects(quitApp(), /桌面应用/)
  await assert.rejects(respondToClose(12, 'quit'), /桌面应用/)
})

test('关闭桥接先订阅后读未决请求，事件优先于迟到快照并去重', async () => {
  const handlers = new Map<string, (event: { payload: unknown }) => void>()
  let resolveInitial!: (value: CloseRequest) => void
  const initial = new Promise<CloseRequest>(done => { resolveInitial = done })
  const delivered: CloseRequest[] = []
  let subscribedAtRead = false
  Object.assign(globalThis, { __TAURI__: {
    core: { invoke: async () => { subscribedAtRead = handlers.has('window-close-requested') && handlers.has('app-exit-failed'); return initial } },
    event: { listen: async (name: string, callback: (event: { payload: unknown }) => void) => { handlers.set(name, callback); return () => { handlers.delete(name) } } },
  } })
  const stop = watchWindowLifecycle({ onClose: value => delivered.push(value), onExitFailed: () => {}, onError: error => { throw error } })
  await new Promise(resolve => setImmediate(resolve))
  handlers.get('window-close-requested')!({ payload: { id: 2, trayAvailable: true } })
  handlers.get('window-close-requested')!({ payload: { id: 2, trayAvailable: true } })
  resolveInitial({ id: 1, trayAvailable: false })
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(subscribedAtRead, true)
  assert.deepEqual(delivered, [{ id: 2, trayAvailable: true }])
  stop()
  assert.equal(handlers.size, 0)
})
