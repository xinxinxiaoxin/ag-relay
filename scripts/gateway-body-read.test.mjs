import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { setTimeout as delay } from 'node:timers/promises'
import { startGatewayFixture } from './helpers/gateway-fixture.mjs'

const defaults = JSON.parse(readFileSync(new URL('../tests/fixtures/policy-settings.json', import.meta.url), 'utf8'))
const success = '{"ok":true,"text":"complete fixture response"}'
const body = {
  model: 'fixture-model', stream: false, prompt_cache_key: 'body-read-cache',
  input: [
    { type: 'message', id: 'msg_body', role: 'user', content: 'fixture request' },
    { type: 'reasoning', id: 'rs_body', encrypted_content: 'fixture ciphertext', summary: [] },
    { type: 'function_call', call_id: 'call_body', name: 'fixture', arguments: '{"keep":true}' },
    { type: 'function_call_output', call_id: 'call_body', output: 'fixture tool output' },
  ],
}
const raw = JSON.stringify(body, null, 2)

async function fixture(t, respond, { profile = {}, env = {} } = {}) {
  const settings = structuredClone(defaults)
  for (const value of Object.values(settings.profiles)) Object.assign(value, {
    maxAttempts: 5, baseDelayMs: 10, maxDelayMs: 10, totalBudgetMs: 5000, connectTimeoutMs: 1000,
    stripItemIds: false, rememberSession: false, ...profile,
  })
  const captured = []
  const f = await startGatewayFixture(t, {
    env: {
      PROXY_MAX_ATTEMPTS: '5', PROXY_DEPLOY_MAX_ATTEMPTS: '5', PROXY_DEPLOY_DELAY_MS: '10',
      PROXY_MIN_RETRY_WINDOW_MS: '10', PROXY_POLICY_JSON: JSON.stringify(settings), ...env,
    },
    respond: async (req, res) => {
      const chunks = []
      for await (const chunk of req) chunks.push(chunk)
      captured.push({ raw: Buffer.concat(chunks).toString('utf8'), headers: { ...req.headers } })
      await respond(res, captured.length)
    },
  })
  return {
    ...f, captured,
    async request(client = 'claude', { signal = AbortSignal.timeout(8000), requestBody = raw } = {}) {
      const response = await fetch(`${f.url}/v1/responses`, {
        method: 'POST', signal,
        headers: { 'content-type': 'application/json', authorization: 'Bearer fixture-only-not-a-key',
          'x-relay-client': client, 'user-agent': `${client}-fixture`, 'session-id': 'body-read-session' },
        body: requestBody,
      })
      return { status: response.status, body: await response.text() }
    },
  }
}

async function truncate(res, status = 200, waitMs = 50) {
  res.writeHead(status, { 'content-type': 'application/json', 'content-length': '10000' })
  res.write(status === 200 ? '{"output":[' : '{"error":{"code":"server_error",')
  await delay(waitMs)
  res.destroy()
}

function complete(res, status = 200, value = success) {
  res.writeHead(status, { 'content-type': 'application/json' })
  res.end(value)
}

for (const client of ['claude', 'codex']) {
  for (const status of [200, 503]) {
    test(`正文断线 ${client}/HTTP ${status}：第二次成功，保留完整请求与身份`, { timeout: 10000 }, async t => {
      const f = await fixture(t, (res, count) => count === 1 ? truncate(res, status) : complete(res))
      assert.deepEqual(await f.request(client), { status: 200, body: success })
      assert.equal(f.captured.length, 2)
      for (const request of f.captured) {
        assert.equal(request.raw, raw)
        assert.equal(request.headers.authorization, 'Bearer fixture-only-not-a-key')
        assert.equal(request.headers['user-agent'], `${client}-fixture`)
        assert.equal(request.headers['session-id'], 'body-read-session')
      }
    })
  }
}

for (const client of ['claude', 'codex']) {
  test(`正文断线 ${client}：次数耗尽按实际两次报告`, { timeout: 10000 }, async t => {
    const f = await fixture(t, res => truncate(res), {
      profile: { maxAttempts: 2 }, env: { PROXY_DEPLOY_MAX_ATTEMPTS: '2' },
    })
    const response = await f.request(client)
    assert.equal(response.status, 502)
    assert.match(JSON.parse(response.body).error.message, /after 2 attempts/)
    assert.equal(f.captured.length, 2)
    assert.ok(f.captured.every(request => request.raw === raw))
  })
}

test('正文断线：常规110次不被部署最低8次截断，第九次仍可成功', { timeout: 10000 }, async t => {
  const f = await fixture(t, (res, count) => count <= 8 ? truncate(res, 503) : complete(res), {
    profile: { maxAttempts: 110 }, env: { PROXY_DEPLOY_MAX_ATTEMPTS: '8' },
  })
  assert.deepEqual(await f.request('codex'), { status: 200, body: success })
  assert.equal(f.captured.length, 9)
  assert.ok(f.captured.every(request => request.raw === raw))
})

test('正文断线：沿用较小常规预算的部署最低尝试机会', { timeout: 10000 }, async t => {
  const f = await fixture(t, (res, count) => count < 3 ? truncate(res) : complete(res), {
    profile: { maxAttempts: 2 }, env: { PROXY_DEPLOY_MAX_ATTEMPTS: '3' },
  })
  assert.deepEqual(await f.request(), { status: 200, body: success })
  assert.equal(f.captured.length, 3)
})

test('正文断线：读取失败时已过总预算，不再发送并报告实际一次', { timeout: 10000 }, async t => {
  const f = await fixture(t, res => truncate(res, 200, 1200), { profile: { totalBudgetMs: 1000 } })
  const response = await f.request()
  assert.equal(response.status, 503)
  assert.match(JSON.parse(response.body).error.message, /after 1 attempts/)
  assert.equal(f.captured.length, 1)
})

test('正文读取：已到响应头的慢成功正文不被响应头或重试预算截断', { timeout: 10000 }, async t => {
  const f = await fixture(t, async res => {
    res.writeHead(200, { 'content-type': 'application/json' })
    res.write(success.slice(0, 5))
    await delay(1200)
    res.end(success.slice(5))
  }, { profile: { totalBudgetMs: 1000 } })
  assert.deepEqual(await f.request(), { status: 200, body: success })
  assert.equal(f.captured.length, 1)
})

async function waitForStatus(f, accepts) {
  const deadline = Date.now() + 3000
  while (Date.now() < deadline) {
    const status = await f.status()
    if (accepts(status)) return
    await delay(10)
  }
  assert.fail('隔离网关未到达预期请求阶段')
}

for (const phase of ['reading_body', 'retry_waiting']) {
  test(`正文断线：${phase} 阶段取消立即结束且不再发送`, { timeout: 10000 }, async t => {
    const f = await fixture(t, res => {
      if (phase === 'retry_waiting') return truncate(res)
      res.writeHead(200, { 'content-type': 'application/json' })
      res.write('{"output":[')
    }, { env: { PROXY_DEPLOY_DELAY_MS: '500' } })
    const controller = new AbortController()
    t.after(() => controller.abort())
    const rejected = assert.rejects(f.request('claude', { signal: controller.signal }), { name: 'AbortError' })
    await waitForStatus(f, status => status.requests.some(request => request.phase === phase))
    controller.abort()
    await rejected
    await waitForStatus(f, status => status.activeRequests === 0)
    assert.equal(f.captured.length, 1)
  })
}

for (const status of [401, 402, 403]) {
  test(`正文断线：真实HTTP ${status} 不新增重试，错误次数如实报告`, { timeout: 10000 }, async t => {
    const f = await fixture(t, res => truncate(res, status))
    const response = await f.request()
    assert.equal(response.status, 502)
    assert.match(JSON.parse(response.body).error.message, /after 1 attempts/)
    assert.equal(f.captured.length, 1)
  })
}

const unknownError = '{"error":{"code":"opaque_history","message":"Opaque provider history rejected"}}'
const historyError = '{"error":{"message":"encrypted content could not be verified"}}'

test('正文断线：未覆盖错误的连续确认被传输失败打断', { timeout: 10000 }, async t => {
  const f = await fixture(t, (res, count) => {
    if (count === 2) return truncate(res)
    complete(res, count < 5 ? 400 : 200, count < 5 ? unknownError : success)
  })
  assert.deepEqual(await f.request('codex'), { status: 200, body: success })
  assert.equal(f.captured.length, 5)
  assert.ok(f.captured.slice(0, 4).every(request => request.raw === raw))
  assert.ok(!JSON.parse(f.captured[4].raw).input.some(item => item.type === 'reasoning'))
})

test('正文断线：保底候选仍最多实际发送两次且保持同一请求体', { timeout: 10000 }, async t => {
  const f = await fixture(t, (res, count) => count <= 2 ? complete(res, 400, unknownError) : truncate(res))
  const response = await f.request('codex')
  assert.equal(response.status, 502)
  assert.match(JSON.parse(response.body).error.message, /after 4 attempts/)
  assert.equal(f.captured.length, 4)
  assert.ok(f.captured.slice(0, 2).every(request => request.raw === raw))
  assert.ok(!JSON.parse(f.captured[2].raw).input.some(item => item.type === 'reasoning'))
  assert.equal(f.captured[3].raw, f.captured[2].raw)
})

test('正文断线：明确历史降级后的三次实际发送上限继续生效', { timeout: 10000 }, async t => {
  const f = await fixture(t, (res, count) => count === 1 ? complete(res, 400, historyError) : truncate(res))
  const response = await f.request('codex')
  assert.equal(response.status, 503)
  assert.match(JSON.parse(response.body).error.message, /after 4 attempts/)
  assert.equal(f.captured.length, 4)
  assert.equal(f.captured[0].raw, raw)
  assert.ok(!JSON.parse(f.captured[1].raw).input.some(item => item.type === 'reasoning'))
  assert.ok(f.captured.slice(1).every(request => request.raw === f.captured[1].raw))
})
