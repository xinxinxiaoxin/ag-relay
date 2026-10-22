// 仅验证自持隔离核心：临时目录、随机回环端口、假上游，不接触现用网关。
import assert from 'node:assert/strict'
import test from 'node:test'
import http from 'node:http'
import { spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { once } from 'node:events'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

const core = resolve('src-tauri/resources/gateway/relay-retry-proxy.mjs')
const runtime = process.platform === 'win32'
  ? resolve('src-tauri/binaries/node-x86_64-pc-windows-msvc.exe') : process.execPath
const token = () => randomBytes(16).toString('hex')
const options = { timeout: 12000 }

async function listen(server) {
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  return server.address().port
}

async function fixture(t, respond = async (_, res) => res.end('{"ok":true}')) {
  const dir = mkdtempSync(join(tmpdir(), 'ag-relay-drain-'))
  const diagnostics = join(dir, 'diagnostics.jsonl')
  let upstreamError
  const upstream = http.createServer(async (req, res) => {
    try {
      for await (const _ of req) { /* consume fixture request */ }
      res.setHeader('content-type', 'application/json')
      await respond(req, res)
    } catch (error) {
      upstreamError = error
      res.destroy()
    }
  })
  let child
  let closed
  let output = ''
  t.after(async () => {
    try {
      if (child && child.exitCode === null && child.signalCode === null) {
        if (child.connected) child.send({ type: 'shutdown' })
        await Promise.race([closed, delay(300)])
        // 清理只使用本测试亲自创建的进程句柄，绝不按端口或进程名操作。
        if (child.exitCode === null && child.signalCode === null) child.kill()
        await Promise.race([closed, delay(2000).then(() => { throw new Error('隔离核心未退出') })])
      }
    } finally {
      upstream.closeAllConnections()
      await new Promise(resolve => upstream.close(resolve))
      if (!child || child.exitCode !== null || child.signalCode !== null) {
        rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 })
      }
    }
    if (upstreamError) throw upstreamError
  })
  const upstreamPort = await listen(upstream)
  const reservation = http.createServer()
  const port = await listen(reservation)
  await new Promise(resolve => reservation.close(resolve))
  assert.notEqual(port, 8787)
  assert.notEqual(upstreamPort, 8787)
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => /^(SystemRoot|WINDIR|TEMP|TMP)$/i.test(key)))
  Object.assign(env, {
    PROXY_HOST: '127.0.0.1', PROXY_PORT: String(port), PROXY_UPSTREAM: `http://127.0.0.1:${upstreamPort}`,
    PROXY_DIAGNOSTICS_FILE: diagnostics, PROXY_SUPERVISED: '1', PROXY_VERBOSE: '1',
    PROXY_MAX_ATTEMPTS: '1', PROXY_TOTAL_BUDGET_MS: '5000', PROXY_CONNECT_TIMEOUT_MS: '5000',
  })
  child = spawn(runtime, [core], { cwd: dir, env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe', 'ipc'] })
  closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })))
  child.stdout.on('data', chunk => { output += chunk })
  child.stderr.on('data', chunk => { output += chunk })
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`隔离核心就绪超时：${output}`)), 5000)
    child.once('error', error => { clearTimeout(timer); reject(error) })
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`隔离核心退出 ${code}：${output}`)) })
    child.once('message', message => {
      clearTimeout(timer)
      if (message.type === 'ready') resolve()
      else reject(new Error('隔离核心未报告 ready'))
    })
  })
  // 本测试子进程 ready 以后才发 HTTP；绑定失败时不探测占位的实例。
  const url = `http://127.0.0.1:${port}`
  const request = async (path, headers = {}, method = 'POST') => {
    const response = await fetch(`${url}${path}`, {
      method, signal: AbortSignal.timeout(2000),
      headers: { 'content-type': 'application/json', ...headers },
      ...(method === 'POST' ? { body: '{}' } : {}),
    })
    return { status: response.status, body: await response.json() }
  }
  const status = async () => (await request('/_relay/status', {}, 'GET')).body
  assert.equal((await status()).pid, child.pid)
  return {
    child, closed, url, request, status,
    logs: () => output + readFileSync(diagnostics, 'utf8'),
    forward: () => request('/v1/responses', { authorization: 'Bearer fixture-only' }),
    drain: (action, operation, headers = {}) => request('/_relay/drain', {
      'x-relay-control': '1', 'x-relay-pid': String(child.pid),
      'x-relay-action': action, 'x-relay-operation': operation, ...headers,
    }),
    start: (operation, budget = 5000, lease = 1000) => request('/_relay/drain', {
      'x-relay-control': '1', 'x-relay-pid': String(child.pid), 'x-relay-action': 'start',
      'x-relay-operation': operation, 'x-relay-timeout-ms': String(budget), 'x-relay-lease-ms': String(lease),
    }),
  }
}

async function eventually(check, message, budget = 2000) {
  const deadline = Date.now() + budget
  while (Date.now() < deadline) {
    if (await check()) return
    await delay(20)
  }
  assert.fail(message)
}

function deferred() {
  let resolve
  const promise = new Promise(done => { resolve = done })
  return { promise, resolve }
}

test('零请求开始等待不退出，暂停新请求，取消后恢复接单', options, async t => {
  const f = await fixture(t)
  const operation = token()
  assert.deepEqual(await f.start(operation), {
    status: 202, body: { operation, activeRequests: 0, draining: true },
  })
  const status = await f.status()
  assert.equal(status.safeShutdownVersion, 1)
  assert.equal(status.drainWaitVersion, 1)
  assert.equal(status.draining, true)
  assert.ok(!JSON.stringify(status).includes(operation), 'status 不暴露控制 token')
  assert.equal((await f.forward()).status, 503)
  assert.deepEqual(await f.drain('refresh', operation), {
    status: 200, body: { operation, activeRequests: 0, draining: true },
  })
  assert.deepEqual(await f.drain('cancel', operation), {
    status: 200, body: { operation, cancelled: true },
  })
  assert.equal((await f.status()).draining, false)
  assert.equal((await f.forward()).status, 200)
  assert.ok(!f.logs().includes(operation), '诊断与 verbose 不暴露控制 token')
})

test('等待不打断已有请求，在途归零后仍须 commit 才退出', options, async t => {
  const entered = deferred()
  const release = deferred()
  t.after(() => release.resolve())
  const f = await fixture(t, async (_, res) => {
    entered.resolve()
    await release.promise
    res.end('{"ok":true}')
  })
  const existing = f.forward()
  await entered.promise
  const operation = token()
  assert.deepEqual(await f.start(operation), {
    status: 202, body: { operation, activeRequests: 1, draining: true },
  })
  assert.equal((await f.forward()).status, 503)
  assert.equal((await f.drain('commit', operation)).status, 409, '忙时不能确认退出')
  release.resolve()
  assert.deepEqual(await existing, { status: 200, body: { ok: true } }, '已有请求完整返回')
  await eventually(async () => (await f.status()).activeRequests === 0, '在途请求未完成')
  assert.deepEqual(await f.drain('refresh', operation), {
    status: 200, body: { operation, activeRequests: 0, draining: true },
  })
  assert.equal(f.child.exitCode, null, '在途归零不能自动退出')
  assert.deepEqual(await f.drain('commit', operation), {
    status: 200, body: { operation, drained: true, pid: f.child.pid },
  })
  assert.deepEqual(await Promise.race([f.closed, delay(2000).then(() => 'timeout')]), { code: 0, signal: null })
  assert.ok(!f.logs().includes(operation), '提交也不泄露控制 token')
})

test('budget 到期后 cancel 可确认恢复且不影响新 op', options, async t => {
  const f = await fixture(t)
  const operation = token()
  assert.equal((await f.start(operation, 500, 10000)).status, 202)
  await delay(525)
  // 先直接访问操作接口，不靠 status 轮询触发到期处理。
  assert.equal((await f.drain('refresh', operation)).status, 409, '总截止已过不能续租')
  assert.equal((await f.drain('commit', operation)).status, 409, '总截止已过不能确认退出')
  assert.equal((await f.start(operation)).status, 409, '重复 start 不能重置已到期预算')
  assert.deepEqual(await f.drain('cancel', operation), {
    status: 200, body: { operation, cancelled: true },
  })
  assert.equal((await f.forward()).status, 200)
  const next = token()
  assert.equal((await f.start(next)).status, 202)
  assert.deepEqual(await f.drain('cancel', operation), {
    status: 200, body: { operation, cancelled: true },
  })
  assert.equal((await f.status()).draining, true, '旧操作的取消确认不能撤销新等待')
  assert.equal((await f.forward()).status, 503)
  for (const action of ['refresh', 'commit']) {
    assert.equal((await f.drain(action, operation)).status, 409)
  }
  assert.equal((await f.start(operation)).status, 409)
  assert.equal((await f.drain('cancel', token())).status, 409, '真正未知 token 仍拒绝')
  assert.equal((await f.drain('cancel', next)).status, 200)
  assert.deepEqual(await f.drain('cancel', next), {
    status: 200, body: { operation: next, cancelled: true },
  })
  assert.equal((await f.forward()).status, 200)
})

test('重复 start 与 refresh 只续租约，绝不延长该操作的总预算', options, async t => {
  const f = await fixture(t)
  const operation = token()
  assert.equal((await f.start(operation, 650, 500)).status, 202)
  await delay(150)
  assert.equal((await f.start(operation, 600000, 10000)).status, 202, '同 token 重试 start 幂等')
  assert.equal((await f.start(token())).status, 409, '等待期间拒绝新 token')
  while ((await f.status()).draining) {
    const response = await f.drain('refresh', operation)
    assert.ok([200, 409].includes(response.status))
    await delay(100)
  }
  assert.equal((await f.forward()).status, 200)
  for (const action of ['refresh', 'commit']) {
    assert.equal((await f.drain(action, operation)).status, 409, `${action} 不能恢复过期操作`)
  }
  assert.deepEqual(await f.drain('cancel', operation), {
    status: 200, body: { operation, cancelled: true },
  })
  assert.equal((await f.start(operation)).status, 409, '过期 token 不得重新开始总预算')
  const next = token()
  assert.equal((await f.start(next)).status, 202, '使用新 token 可开始下一次等待')
  assert.equal((await f.drain('cancel', next)).status, 200)
})

test('租约失联与总预算到期都恢复接单，不中断在途请求', options, async t => {
  const entered = deferred()
  const release = deferred()
  t.after(() => release.resolve())
  const f = await fixture(t, async (req, res) => {
    if (req.url === '/v1/responses' && !entered.done) {
      entered.done = true
      entered.resolve()
      await release.promise
    }
    res.end('{"ok":true}')
  })
  const existing = f.forward()
  await entered.promise
  const operation = token()
  assert.equal((await f.start(operation, 5000, 250)).status, 202)
  await delay(150)
  assert.equal((await f.drain('refresh', operation)).status, 200)
  await delay(150)
  assert.equal((await f.status()).draining, true, 'refresh 应推迟租约过期')
  await eventually(async () => !(await f.status()).draining, '租约到期未恢复接单')
  assert.equal((await f.status()).activeRequests, 1, '租约到期不能中断已有请求')
  assert.equal((await f.forward()).status, 200)
  const budgetOperation = token()
  assert.equal((await f.start(budgetOperation, 200, 10000)).status, 202)
  await eventually(async () => !(await f.status()).draining, '总预算到期未恢复接单')
  assert.equal((await f.status()).activeRequests, 1, '总预算到期不能中断已有请求')
  assert.equal((await f.drain('commit', budgetOperation)).status, 409)
  release.resolve()
  assert.deepEqual(await existing, { status: 200, body: { ok: true } })
  assert.equal((await f.forward()).status, 200)
})

test('控制头、实例与 token 校验失败不能改变等待状态', options, async t => {
  const f = await fixture(t)
  const operation = token()
  const headers = {
    'x-relay-control': '1', 'x-relay-pid': String(f.child.pid), 'x-relay-operation': operation,
    'x-relay-action': 'start', 'x-relay-timeout-ms': '600000', 'x-relay-lease-ms': '10000',
  }
  const invalid = [
    ['x-relay-control', '', 403], ['x-relay-pid', String(f.child.pid + 1), 409],
    ['x-relay-operation', operation.toUpperCase(), 400], ['x-relay-operation', 'a'.repeat(31), 400],
    ['x-relay-action', 'finish', 400],
    ...['0', '600001', '1.5', '1e3', 'Infinity', ''].map(value => ['x-relay-timeout-ms', value, 400]),
    ...['99', '10001', '100.5', '1e3', ''].map(value => ['x-relay-lease-ms', value, 400]),
  ]
  for (const [header, value, status] of invalid) {
    assert.equal((await f.request('/_relay/drain', { ...headers, [header]: value })).status, status, `${header}=${value}`)
    assert.equal((await f.status()).draining, false)
  }
  assert.equal((await f.request('/_relay/drain', headers)).status, 202, '上限 10 分钟与 10 秒租约可用')
  const unknown = token()
  for (const action of ['refresh', 'cancel', 'commit']) {
    assert.equal((await f.drain(action, unknown)).status, 409)
    assert.equal((await f.status()).draining, true)
  }
  assert.equal((await f.drain('cancel', operation, { 'x-relay-pid': String(f.child.pid + 1) })).status, 409)
  assert.equal((await f.status()).draining, true)
  assert.equal((await f.drain('cancel', operation)).status, 200)
  assert.equal((await f.drain('commit', operation)).status, 409, '取消与提交互斥')
  await delay(150)
  assert.equal((await f.forward()).status, 200, '取消后不留下延迟退出')
  assert.ok(!f.logs().includes(operation) && !f.logs().includes(unknown))
})

test('v1 shutdown/restart 与两阶段等待互斥，取消后 v1 停机仍可用', options, async t => {
  const f = await fixture(t)
  const operation = token()
  assert.equal((await f.start(operation)).status, 202)
  const control = { 'x-relay-control': '1', 'x-relay-pid': String(f.child.pid), 'x-relay-timeout-ms': '1000' }
  assert.equal((await f.request('/_relay/shutdown', control)).status, 409)
  assert.equal((await f.request('/_relay/restart', control)).status, 409)
  assert.equal((await f.status()).draining, true)
  assert.equal((await f.drain('cancel', operation)).status, 200)
  assert.deepEqual(await f.request('/_relay/shutdown', control), {
    status: 200, body: { drained: true, pid: f.child.pid },
  })
  assert.deepEqual(await Promise.race([f.closed, delay(2000).then(() => 'timeout')]), { code: 0, signal: null })
})

test('v1 安全停机正在等待时拒绝 start，原有超时恢复协议不变', options, async t => {
  const entered = deferred()
  const release = deferred()
  t.after(() => release.resolve())
  const f = await fixture(t, async (_, res) => {
    entered.resolve()
    await release.promise
    res.end('{"ok":true}')
  })
  const existing = f.forward()
  await entered.promise
  const shutdown = f.request('/_relay/shutdown', {
    'x-relay-control': '1', 'x-relay-pid': String(f.child.pid), 'x-relay-timeout-ms': '350',
  })
  await eventually(async () => (await f.status()).draining, 'v1 安全停机未进入排空')
  assert.equal((await f.start(token())).status, 409)
  assert.deepEqual(await shutdown, { status: 408, body: { error: 'drain_timeout' } })
  assert.equal((await f.status()).draining, false)
  release.resolve()
  assert.equal((await existing).status, 200)
  assert.equal((await f.forward()).status, 200)
})

test('v1 restart 接受与 IPC drain 退出码保持兼容，并拒绝并行等待', options, async t => {
  const f = await fixture(t)
  const restartMessage = once(f.child, 'message')
  const response = await f.request('/_relay/restart', { 'x-relay-control': '1' })
  assert.deepEqual(response, {
    status: 202, body: { accepted: true, restarting: true, draining: false, activeRequests: 0 },
  })
  assert.deepEqual((await restartMessage)[0], { type: 'restart_request' })
  assert.equal((await f.start(token())).status, 409)
  f.child.send({ type: 'drain' })
  assert.deepEqual(await Promise.race([f.closed, delay(2000).then(() => 'timeout')]), { code: 75, signal: null })
})

test('旧 IPC shutdown 清除等待态并自然等已有请求完成', options, async t => {
  const entered = deferred()
  const release = deferred()
  t.after(() => release.resolve())
  const f = await fixture(t, async (_, res) => {
    entered.resolve()
    await release.promise
    res.end('{"ok":true}')
  })
  const existing = f.forward()
  await entered.promise
  const operation = token()
  assert.equal((await f.start(operation, 5000, 100)).status, 202)
  f.child.send({ type: 'shutdown' })
  await eventually(async () => (await f.drain('refresh', operation)).status === 409, '实际停机未清除等待操作')
  await delay(150)
  assert.equal((await f.forward()).status, 503, '等待租约不得撤销已开始的实际停机')
  release.resolve()
  assert.equal((await existing).status, 200)
  assert.deepEqual(await Promise.race([f.closed, delay(2000).then(() => 'timeout')]), { code: 0, signal: null })
})
