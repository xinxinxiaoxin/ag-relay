import assert from 'node:assert/strict'
import http from 'node:http'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'

export const gatewayCore = fileURLToPath(new URL('../../src-tauri/resources/gateway/relay-retry-proxy.mjs', import.meta.url))
const runtime = process.platform === 'win32'
  ? fileURLToPath(new URL('../../src-tauri/binaries/node-x86_64-pc-windows-msvc.exe', import.meta.url)) : process.execPath

async function listen(server) {
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const port = server.address().port
  assert.notEqual(port, 8787, '隔离测试不得使用现用网关默认端口')
  return port
}

// 仅管理自持核心与假上游。请求内容、返回场景和业务断言由各测试定义。
export async function startGatewayFixture(t, { respond, env: overrides = {}, script = gatewayCore, startupOnly = false }) {
  const dir = mkdtempSync(join(tmpdir(), 'ag-relay-fixture-'))
  const diagnostics = join(dir, 'diagnostics.jsonl')
  let upstreamError
  const upstream = http.createServer(async (req, res) => {
    try { await respond(req, res) }
    catch (error) { upstreamError = error; res.destroy() }
  })
  let child
  let closed
  let output = ''
  t.after(async () => {
    try {
      if (child && child.exitCode === null && child.signalCode === null) {
        if (child.connected) child.send({ type: 'shutdown' })
        await Promise.race([closed, delay(1500)])
        // 只终止本测试创建并持有的子进程，不按端口或进程名清理。
        if (child.exitCode === null && child.signalCode === null) child.kill()
        await Promise.race([closed, delay(3000).then(() => { throw new Error('隔离核心未退出') })])
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
  // 不继承用户凭据、NODE_OPTIONS 或现用网关配置。
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => /^(SystemRoot|WINDIR|TEMP|TMP)$/i.test(key)))
  Object.assign(env, {
    PROXY_HOST: '127.0.0.1', PROXY_PORT: String(port), PROXY_UPSTREAM: `http://127.0.0.1:${upstreamPort}`,
    PROXY_DIAGNOSTICS_FILE: diagnostics, PROXY_VERBOSE: '0', PROXY_SUPERVISED: '1',
    ...overrides,
  })
  child = spawn(runtime, [script], { cwd: dir, env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe', 'ipc'] })
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
  // 无效配置只观察启动结果，即使意外 ready 也不能发 HTTP 探测。
  if (startupOnly) assert.fail('无效配置意外到达 ready：必须在监听前拒绝')
  const url = `http://127.0.0.1:${port}`
  const status = async () => {
    const response = await fetch(`${url}/_relay/status`, { signal: AbortSignal.timeout(2000) })
    assert.equal(response.status, 200)
    const value = await response.json()
    assert.equal(value.pid, child.pid, '必须是本测试启动的核心')
    return value
  }
  // IPC ready 与自身 PID 核实完成后，调用方才可发送业务请求。
  await status()
  return { child, closed, url, diagnostics, status, output: () => output }
}
