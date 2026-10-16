// 单实例隔离验证：只运行已编译的 example，不启动产品、不访问网关与用户数据。
import assert from 'node:assert/strict'
import test from 'node:test'
import { spawn } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import { randomUUID } from 'node:crypto'

const binary = resolve(process.env.AG_RELAY_SINGLE_INSTANCE_TEST_BINARY
  ?? 'src-tauri/target/debug/examples/verify-single-instance.exe')
const windowsOnly = { skip: process.platform !== 'win32', timeout: 60000 }
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function events(file) {
  try {
    // 最后的未完成行留给下次读取，避免把并发 append 的半行当成完整事件。
    return readFileSync(file, 'utf8').split(/\r?\n/).slice(0, -1).filter(Boolean)
  } catch (error) {
    if (error.code === 'ENOENT') return []
    throw error
  }
}

async function until(predicate, message, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (predicate()) return
    await delay(20)
  }
  assert.fail(typeof message === 'function' ? message() : message)
}

async function withFixture(run) {
  const tempRoot = resolve(tmpdir())
  const dir = mkdtempSync(join(tempRoot, 'ag-relay-single-instance-'))
  const evidence = join(dir, 'events.txt')
  const id = `ag-relay-single-instance-test-${randomUUID()}`
  const children = []
  const describe = () => children.map(({ child, code, signal, error, stderr }) =>
    `pid=${child.pid ?? '-'} code=${code} signal=${signal} error=${error?.message ?? '-'} ${stderr}`).join('\n')
  const launch = ({ holdBeforeBuild = false, executable = binary } = {}) => {
    const args = [id, evidence]
    if (holdBeforeBuild) args.push('--hold-before-build')
    const child = spawn(executable, args, { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] })
    const state = { child, code: null, signal: null, error: null, stderr: '', exited: false, closed: false }
    children.push(state)
    child.stderr.on('data', (bytes) => { state.stderr = (state.stderr + bytes).slice(-8192) })
    // error 不一定伴随 exit（例如 ENOENT），但 close 会结束生命周期。
    child.on('error', (error) => { state.error = error })
    child.once('exit', (code, signal) => { Object.assign(state, { code, signal, exited: true }) })
    child.once('close', () => { state.closed = true })
    return state
  }
  const wait = (predicate, message) => until(() => {
    for (const state of children) {
      if (state.error) throw state.error
      assert.ok(!state.exited || state.code === 0, `验证进程异常退出：${describe()}`)
    }
    return predicate()
  }, () => `${message}\n${describe()}`)
  const rows = (event) => events(evidence).filter((line) => line.startsWith(`${event} `))
  let failure
  try {
    await run({ dir, launch, wait, rows, release: () => writeFileSync(join(dir, 'events.release'), 'ready') })
  } catch (error) {
    failure = error
  }
  try {
    for (const { child, exited, closed } of children) {
      if (child.pid && !exited && !closed) child.kill()
    }
    await until(() => children.every((state) => state.closed),
      () => `子进程清理超时；保留证据目录 ${dir}\n${describe()}`, 5000)
    assert.equal(dirname(dir), tempRoot)
    assert.ok(basename(dir).startsWith('ag-relay-single-instance-'))
    rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 })
  } catch (error) {
    // 即使系统拒绝终止本测试子进程，管道/ChildProcess 也不能让测试运行器永久悬挂。
    // 上面的超时错误保留 pid 与证据目录；只撤销本进程等待，不按名字或端口扩大清理。
    for (const { child, closed } of children) {
      if (!closed) {
        child.stderr.destroy()
        child.unref()
      }
    }
    throw failure ? new AggregateError([failure, error], '验证与清理均失败') : error
  }
  if (failure) throw failure
}

function requireBinary() {
  assert.ok(existsSync(binary), `缺少 ${binary}；先编译隔离 example verify-single-instance`)
}

test('重复启动只保留一个实例并唤起已有实例', windowsOnly, async () => {
  requireBinary()
  await withFixture(async ({ launch, wait, rows }) => {
    const primary = launch()
    await wait(() => rows('ready').length === 1, '首实例未完成初始化')
    const secondary = launch()
    await wait(() => secondary.closed, '第二个实例未正常退出')
    await wait(() => rows('callback').length === 1 && rows('activate').length === 1, '已有实例未收到唤起')
    assert.equal(secondary.code, 0)
    assert.deepEqual(rows('setup'), [`setup ${primary.child.pid}`])
    assert.equal(primary.exited, false, '首实例不应退出')
  })
})

test('并发启动在通知窗口创建前等待同一 gate，始终只有一次 setup', windowsOnly, async () => {
  requireBinary()
  for (let round = 0; round < 3; round++) {
    await withFixture(async ({ launch, wait, rows, release }) => {
      const primary = launch({ holdBeforeBuild: true })
      await wait(() => rows('holding_before_build').length === 1, '首实例未进入持锁屏障')
      const secondaries = Array.from({ length: 4 }, () => launch())
      await wait(() => rows('gate_waiting').length === 5, '并发实例未全部进入启动入口')
      // 屏障固定竞争窗口；短暂让出 CPU，确认后来的进程无法越过 gate。
      await delay(200)
      assert.deepEqual(rows('gate_acquired'), [`gate_acquired ${primary.child.pid}`])
      assert.equal(rows('build_started').length, 0, '持锁屏障释放前不应开始插件初始化')
      assert.equal(rows('setup').length, 0)
      release()
      await wait(() => secondaries.every((state) => state.closed), '并发实例没有全部正常退出')
      await wait(() => rows('ready').length === 1 && rows('callback').length === 4
        && rows('activate').length > 0, '首实例未就绪或丢失并发唤起')
      assert.ok(secondaries.every((state) => state.code === 0))
      assert.deepEqual(rows('setup'), [`setup ${primary.child.pid}`])
      assert.deepEqual(rows('plugin_ready'), [`plugin_ready ${primary.child.pid}`])
      assert.equal(rows('gate_acquired').length, 5, '插件直接退出后的 abandoned gate 也应允许后续进程进入')
      assert.equal(primary.exited, false)
    })
  }
})

test('spawn 失败及时报告且清理不等待不存在的 exit', { timeout: 10000 }, async () => {
  await assert.rejects(withFixture(async ({ dir, launch, wait }) => {
    launch({ executable: join(dir, 'missing-verifier.exe') })
    await wait(() => false, 'spawn 错误未及时报告')
  }), { code: 'ENOENT' })
})
