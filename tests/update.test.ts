import assert from 'node:assert/strict'
import test from 'node:test'
import { createUpdateController, downloadProgress, type UpdateView } from '../src/lib/update.ts'

const available: UpdateView = { phase: 'available', currentVersion: '0.3.3', release: { id: 'candidate-A', version: '0.3.4', notes: '<b>纯文本</b>' }, download: null, detail: '有新版', recovery: null }
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(done => { resolve = done })
  return { promise, resolve }
}
function mockInvoke(invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>) {
  Object.assign(globalThis, { __TAURI__: { core: { invoke } } })
}

test('晚到的轮询旧响应不能把下载状态退回候选可用', async () => {
  const first = deferred<UpdateView>()
  let reads = 0
  mockInvoke(async () => ++reads === 1 ? first.promise : { ...available, phase: 'downloading' })
  const updates = createUpdateController()
  const stale = updates.refresh()
  await updates.refresh()
  first.resolve(available)
  await stale
  assert.equal(updates.getSnapshot().view?.phase, 'downloading')
})

test('重查保留新版提醒但立即撤销旧候选安装权，旧轮询不能解锁', async () => {
  const check = deferred<UpdateView>()
  const oldRead = deferred<UpdateView>()
  let reads = 0
  let installed = false
  mockInvoke(async command => {
    if (command === 'check_for_update') return check.promise
    if (command === 'confirm_update') { installed = true; return available }
    return ++reads === 2 ? oldRead.promise : available
  })
  const updates = createUpdateController()
  await updates.refresh()
  const stale = updates.refresh()
  const checking = updates.check()
  assert.equal(updates.getSnapshot().hasUpdate, true)
  assert.equal(updates.getSnapshot().view?.release, null)
  oldRead.resolve(available)
  await stale
  await updates.confirm('candidate-A')
  assert.equal(installed, false)
  assert.equal(updates.getSnapshot().view?.phase, 'checking')
  check.resolve({ ...available, phase: 'idle', release: null })
  await checking
  assert.equal(updates.getSnapshot().hasUpdate, false)
})

test('操作完成后丢弃操作期间尚未返回的快照，不能退回 checking', async () => {
  const check = deferred<UpdateView>()
  const checkingRead = deferred<UpdateView>()
  let reads = 0
  mockInvoke(async command => command === 'check_for_update' ? check.promise : ++reads === 1 ? available : checkingRead.promise)
  const updates = createUpdateController()
  await updates.refresh()
  const action = updates.check()
  const poll = updates.refresh()
  check.resolve({ ...available, release: { ...available.release!, id: 'candidate-B' } })
  await action
  checkingRead.resolve({ ...available, phase: 'checking', release: null })
  await poll
  assert.equal(updates.getSnapshot().view?.phase, 'available')
  assert.equal(updates.getSnapshot().view?.release?.id, 'candidate-B')
})

test('重查失败的错误与未安装提醒持续可见，但旧候选不能再次安装', async () => {
  let current = available
  mockInvoke(async command => {
    if (command === 'check_for_update') { current = { ...available, phase: 'failed', release: null }; throw new Error('网络暂不可用') }
    return current
  })
  const updates = createUpdateController()
  await updates.refresh()
  await updates.check()
  await updates.refresh()
  assert.match(updates.getSnapshot().error ?? '', /网络暂不可用/)
  assert.equal(updates.getSnapshot().hasUpdate, true)
  assert.equal(updates.getSnapshot().view?.release, null)
})

test('下载 40 MB / 100 MB 显示真实 40% 而非模拟进度', () => {
  assert.deepEqual(downloadProgress({ downloadedBytes: 40_000_000, totalBytes: 100_000_000 }), {
    value: 40, text: '40% · 40.0 / 100.0 MB',
  })
})

test('未知、零或不可信总量只显示已下载字节并采用不定进度', () => {
  for (const totalBytes of [null, 0, -1, 20_000_000, Infinity, NaN]) {
    assert.deepEqual(downloadProgress({ downloadedBytes: 40_000_000, totalBytes }), {
      value: null, text: '已下载 40.0 MB · 总大小未知',
    })
  }
  assert.deepEqual(downloadProgress({ downloadedBytes: -1, totalBytes: 100 }), { value: null, text: '正在下载 · 进度暂不可用' })
  assert.deepEqual(downloadProgress(null), { value: null, text: '正在下载 · 进度暂不可用' })
})
