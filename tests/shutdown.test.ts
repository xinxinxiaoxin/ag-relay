import assert from 'node:assert/strict'
import test from 'node:test'
import {
  cancelShutdown, createShutdownController, getShutdownProgress, shutdownControls, shutdownProgressText,
  type ShutdownProgress,
} from '../src/lib/shutdown.ts'

const waiting: ShutdownProgress = { operationId: 12, kind: 'exit', phase: 'waiting', elapsedMs: 12_999, remainingMs: 587_001, remainingRequests: 3 }
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}
function mockInvoke(invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>) {
  Object.assign(globalThis, { __TAURI__: { core: { invoke } } })
}

test('公开命令只读后端快照，取消严格传 operationId；普通浏览器不伪造取消', async () => {
  const calls: unknown[] = []
  mockInvoke(async (command, args) => { calls.push([command, args]); return command === 'get_shutdown_progress' ? waiting : undefined })
  try {
    assert.deepEqual(await getShutdownProgress(), waiting)
    await cancelShutdown(12)
    assert.deepEqual(calls, [['get_shutdown_progress', undefined], ['cancel_shutdown', { operationId: 12 }]])
    await assert.rejects(cancelShutdown(NaN), /编号无效/)
    assert.equal(calls.length, 2)
  } finally { Reflect.deleteProperty(globalThis, '__TAURI__') }
  assert.equal(await getShutdownProgress(), null)
  await assert.rejects(cancelShutdown(12), /桌面应用/)
})

test('仅展示真实请求数和秒数，未知值不伪造零或百分比', () => {
  assert.equal(shutdownProgressText(waiting), '暂停接收新请求 · 剩余3个 · 已等待12秒 · 最长10分钟')
  assert.equal(shutdownProgressText(null), '暂停接收新请求 · 数量未知 · 已等待时间读取中 · 最长10分钟')
  assert.match(shutdownProgressText({ ...waiting, remainingRequests: null }), /数量未知/)
  assert.match(shutdownProgressText({ ...waiting, remainingRequests: 0 }), /剩余0个/)
  assert.doesNotMatch(shutdownProgressText(waiting), /%/)
})

test('取消受理保持锁定，直到后端取消完成；重复点击与旧编号不能取消', async () => {
  let progress: ShutdownProgress | null = waiting
  const calls: unknown[] = []
  mockInvoke(async (command, args) => {
    if (command === 'get_shutdown_progress') return progress
    calls.push([command, args])
  })
  try {
    const controller = createShutdownController()
    await controller.refresh()
    await controller.cancel(11)
    assert.equal(calls.length, 0)
    await controller.cancel(12)
    await controller.cancel(12)
    assert.equal(calls.length, 1)
    assert.equal(controller.getSnapshot().progress?.operationId, 12)
    assert.equal(controller.getSnapshot().cancelRequestedId, 12)
    await controller.refresh() // 后端还在 waiting，也不能把“取消已受理”解锁。
    assert.equal(shutdownControls(controller.getSnapshot()).canCancel, false)
    assert.match(shutdownControls(controller.getSnapshot()).detail, /确认取消完成前/)
    progress = { ...waiting, phase: 'cancelling' }
    await controller.refresh()
    assert.equal(shutdownControls(controller.getSnapshot()).canCancel, false)
    progress = null
    await controller.refresh()
    assert.equal(controller.getSnapshot().progress, null)
    assert.equal(controller.getSnapshot().cancelRequestedId, null)
  } finally { Reflect.deleteProperty(globalThis, '__TAURI__') }
})

test('取消与完成阶段不能取消；取消失败保留错误并允许重试', async () => {
  let progress = { ...waiting, phase: 'committing' } as ShutdownProgress
  let calls = 0
  mockInvoke(async command => {
    if (command === 'get_shutdown_progress') return progress
    calls++
    throw new Error('模拟取消失败')
  })
  try {
    const controller = createShutdownController()
    await controller.refresh()
    await controller.cancel(12)
    assert.equal(calls, 0)
    assert.match(shutdownControls(controller.getSnapshot()).detail, /不能再取消/)
    progress = waiting
    await controller.refresh()
    await controller.cancel(12)
    assert.match(controller.getSnapshot().error ?? '', /模拟取消失败/)
    await controller.refresh()
    assert.match(controller.getSnapshot().error ?? '', /模拟取消失败/)
    assert.equal(shutdownControls(controller.getSnapshot()).canCancel, true)
  } finally { Reflect.deleteProperty(globalThis, '__TAURI__') }
})

test('旧取消响应和旧轮询不能覆盖新操作或污染新错误', async () => {
  const oldCancel = deferred<void>()
  const oldRead = deferred<ShutdownProgress | null>()
  const next = { ...waiting, operationId: 13, kind: 'update' as const }
  let reads = 0
  const calls: unknown[] = []
  mockInvoke(async (command, args) => {
    if (command === 'cancel_shutdown') { calls.push(args); return oldCancel.promise }
    reads++
    return reads === 1 ? waiting : reads === 2 ? oldRead.promise : next
  })
  try {
    const controller = createShutdownController()
    await controller.refresh()
    const cancelling = controller.cancel(12)
    const polling = controller.refresh()
    await controller.refresh()
    oldCancel.reject(new Error('旧操作取消失败'))
    oldRead.resolve(waiting)
    await Promise.all([cancelling, polling])
    assert.equal(controller.getSnapshot().progress?.operationId, 13)
    assert.equal(controller.getSnapshot().error, null)
    assert.equal(controller.getSnapshot().cancelRequestedId, null)
    await controller.cancel(12)
    assert.deepEqual(calls, [{ operationId: 12 }])
    assert.equal(shutdownControls(controller.getSnapshot()).canCancel, true)
  } finally { Reflect.deleteProperty(globalThis, '__TAURI__') }
})
