import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { gatewayCore, startGatewayFixture } from './helpers/gateway-fixture.mjs'

const baseline = process.env.AG_RELAY_HISTORY_BASELINE
const error = (message, code, status = 400) => ({
  status, body: { error: { type: 'invalid_request_error', message, ...(code ? { code } : {}) } },
})
const azure = error('The requested item was created under a different Azure OpenAI resource')
const scenarios = [
  { name: '提前处理', profile: { historyMode: 'always' } },
  { name: '会话记忆', profile: { rememberSession: true }, errors: [azure], requests: 2 },
  { name: '会话绑定', errors: [error('session binding invalidated; use a new session id', 'session_binding_invalidated', 409)] },
  { name: '账号绑定', errors: [error('the upstream account bound to this conversation is unavailable; start a new conversation without previous encrypted reasoning items')] },
  { name: '跨资源', errors: [azure] },
  { name: '失效推理', errors: [error('encrypted content could not be verified')] },
  { name: '空回应一级', empty: 2 },
  { name: '空回应二级', empty: 6 },
  { name: '未覆盖错误', errors: [error('Opaque provider history rejected', 'opaque_history'), error('Opaque provider history rejected', 'opaque_history')] },
]

function history(client) {
  return {
    model: 'fixture-model', store: false, prompt_cache_key: 'parity-cache',
    client_metadata: { session_id: 'parity-session' },
    input: [
      { type: 'message', id: 'msg_parity', role: 'user', content: 'fixture question' },
      { type: 'reasoning', id: 'rs_parity', encrypted_content: 'fixture ciphertext', summary: [] },
      ...(client === 'codex' ? [{ type: 'web_search_call', id: 'ws_parity', status: 'completed',
        action: { type: 'search', query: 'fixture query', extension: { id: 'source_keep' } } }] : []),
      { type: 'agent_message', id: 'agent_parity', author: 'fixture_agent', recipient: 'all', content: 'fixture handoff', extension: { id: 'agent_keep' } },
      { type: 'function_call', id: 'fc_parity', call_id: 'call_parity', name: 'fixture', arguments: '{"id":"argument_keep"}' },
      { type: 'function_call_output', id: 'fco_parity', call_id: 'call_parity', output: 'fixture result' },
      { type: 'custom_tool_call', id: 'ct_parity', call_id: 'custom_parity', name: 'fixture_custom', input: 'fixture input' },
      { type: 'custom_tool_call_output', id: 'cto_parity', call_id: 'custom_parity', output: 'fixture custom result' },
    ],
  }
}

for (const client of ['claude', 'codex']) {
  for (const scenario of scenarios) {
    test(`历史处理旧新对照 ${client}/${scenario.name}`, { skip: !baseline }, async t => {
      const outcomes = []
      for (const script of [resolve(baseline), gatewayCore]) {
        const sent = []
        const f = await startGatewayFixture(t, {
          script,
          env: {
            PROXY_MAX_ATTEMPTS: '12', PROXY_DEPLOY_MAX_ATTEMPTS: '8',
            PROXY_BASE_DELAY_MS: '10', PROXY_MAX_DELAY_MS: '10', PROXY_DEPLOY_DELAY_MS: '10',
            PROXY_TOTAL_BUDGET_MS: '10000', PROXY_MIN_RETRY_WINDOW_MS: '100', PROXY_CONNECT_TIMEOUT_MS: '2000',
            PROXY_POLICY_JSON: JSON.stringify({ profiles: {
              codex: { stripItemIds: false, rememberSession: false, ...scenario.profile },
              general: { rememberSession: false, ...scenario.profile },
            } }),
          },
          respond: async (req, res) => {
            const chunks = []
            for await (const chunk of req) chunks.push(chunk)
            sent.push({ raw: Buffer.concat(chunks).toString('utf8'),
              authorization: req.headers.authorization, ua: req.headers['user-agent'], session: req.headers['session-id'] })
            if (sent.length <= (scenario.empty ?? 0)) {
              res.writeHead(200, { 'content-type': 'text/event-stream' }).end()
              return
            }
            const reply = scenario.errors?.[sent.length - 1] ?? { status: 200, body: { ok: true } }
            res.writeHead(reply.status, { 'content-type': 'application/json' })
            res.end(JSON.stringify(reply.body))
          },
        })
        const responses = []
        for (let i = 0; i < (scenario.requests ?? 1); i++) {
          const response = await fetch(`${f.url}/v1/responses`, {
            method: 'POST', signal: AbortSignal.timeout(12000),
            headers: {
              'content-type': 'application/json', authorization: 'Bearer parity-fixture-only',
              'x-relay-client': client, 'x-relay-session-id': 'parity-session', 'session-id': 'parity-session',
              'user-agent': client === 'codex' ? 'codex_cli_rs/fixture' : 'claude-cli/fixture',
            },
            body: ` \n${JSON.stringify(history(client))}\n `,
          })
          const body = await response.text()
          assert.equal(response.status, client === 'claude' && scenario.name === '会话绑定' ? 409 : 200, body)
          responses.push({ status: response.status, body })
        }
        const expectedSends = client === 'claude' && scenario.name === '会话绑定' ? 1
          : (scenario.empty ?? scenario.errors?.length ?? 0) + (scenario.requests ?? 1)
        assert.equal(sent.length, expectedSends, '场景必须实际走完预期的恢复发送')
        for (const request of sent) assert.doesNotThrow(() => JSON.parse(request.raw), '历史请求必须是有效 JSON')
        const repairs = readFileSync(f.diagnostics, 'utf8').trim().split('\n').map(JSON.parse)
          .filter(row => ['repair_applied', 'repair_refused', 'session_compat_applied', 'session_compat_refused', 'session_compat_enabled'].includes(row.event))
          .map(({ event, kind, source, removedIds, removedReasoning, removedTypes, refusal }) =>
            ({ event, kind, source, removedIds, removedReasoning, removedTypes, refusal }))
        outcomes.push({ sent, responses, repairs })
      }
      assert.deepEqual(outcomes[1], outcomes[0], '发送次数、请求字节、身份、返回结果与修复诊断必须相同')
    })
  }
}
