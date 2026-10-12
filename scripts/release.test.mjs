import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import { after, before } from 'node:test';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const cli = path.join(root, 'scripts/release.mjs');
const tauri = path.join(root, 'node_modules/@tauri-apps/cli/tauri.js');
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'ag-relay-signature-test-'));
const key = path.join(temp, 'TEST-ONLY.key');
const testConfig = path.join(temp, 'tauri.test.json');
const artifacts = path.join(temp, 'artifacts');
const manifest = {
  version: '0.2.0', notes: '仅隔离验签测试，不可发布', pub_date: '2026-10-12T00:00:00Z',
  platforms: {},
};

function signing(args) {
  const env = { ...process.env, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '' };
  delete env.TAURI_SIGNING_PRIVATE_KEY;
  delete env.TAURI_SIGNING_PRIVATE_KEY_PATH;
  const result = spawnSync(process.execPath, [tauri, 'signer', ...args], {
    cwd: root, encoding: 'utf8', env,
  });
  assert.equal(result.status, 0, '临时签名命令必须成功（不回显密钥输出）');
}

before(() => {
  signing(['generate', '--ci', '-p', '', '-w', key]);
  fs.mkdirSync(artifacts);
  const config = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json')));
  config.version = '0.2.0';
  config.plugins = { updater: { pubkey: fs.readFileSync(`${key}.pub`, 'utf8').trim(),
    endpoints: ['https://example.invalid/ag-relay-test/latest.json'], requireSignedVersion: true } };
  fs.writeFileSync(testConfig, JSON.stringify(config));
  for (const [target, name] of [['nsis', 'ag-relay_0.2.0_x64-setup.exe'], ['msi', 'ag-relay_0.2.0_x64_zh-CN.msi']]) {
    const file = path.join(artifacts, name);
    fs.writeFileSync(file, `这是 ${target} 验签测试字节，不是可运行的安装程序`);
    signing(['sign', '-f', key, '-p', '', '--app-version', '0.2.0', file]);
    manifest.platforms[`windows-x86_64-${target}`] = {
      url: `https://example.invalid/ag-relay-test/${name}`, signature: fs.readFileSync(`${file}.sig`, 'utf8').trim(),
    };
  }
  fs.writeFileSync(path.join(artifacts, 'latest.json'), JSON.stringify(manifest, null, 2));
});
after(() => fs.rmSync(temp, { recursive: true, force: true }));


function run(args, env = {}) {
  return spawnSync(process.execPath, [cli, ...args], {
    cwd: root,
    encoding: 'utf8',
    env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY: '', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '', ...env },
  });
}

test('正式发布未配置公钥时拒绝，不生成可发布更新清单', () => {
  const emptyConfig = path.join(temp, 'empty-config.json');
  fs.writeFileSync(emptyConfig, JSON.stringify({ plugins: { updater: { pubkey: '', endpoints: [] } } }));
  const result = run(['preflight', '--config', emptyConfig]);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /生产更新公钥尚未配置/);
  assert.doesNotMatch(result.stderr, /TAURI_SIGNING_PRIVATE_KEY=.*\S/);
});

test('正式构建在生产材料缺失时不创建输出目录', () => {
  const out = path.join(temp, 'must-not-exist');
  const result = run(['build', '--out', out]);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /生产更新公钥尚未配置|必须通过环境提供 TAURI_SIGNING_PRIVATE_KEY/);
  assert.equal(fs.existsSync(out), false);
});

test('生产公钥就绪但缺少环境私钥时仍拒绝发布', () => {
  const config = JSON.parse(fs.readFileSync(testConfig));
  config.plugins.updater.endpoints = ['https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json'];
  const file = path.join(temp, 'production-config.json');
  fs.writeFileSync(file, JSON.stringify(config));
  const result = run(['preflight', '--config', file]);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /必须通过环境提供 TAURI_SIGNING_PRIVATE_KEY/);
});

test('发布拒绝测试端点和不安全的签名策略', () => {
  for (const patch of [
    { endpoints: ['https://example.invalid/ag-relay-test/latest.json'] },
    { endpoints: ['http://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json'] },
    { requireSignedVersion: false },
    { dangerousAcceptInvalidCerts: true },
    { allowDowngrades: true },
  ]) {
    const config = JSON.parse(fs.readFileSync(testConfig));
    Object.assign(config.plugins.updater, { endpoints: ['https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json'] }, patch);
    const file = path.join(temp, 'bad-config.json');
    fs.writeFileSync(file, JSON.stringify(config));
    const result = run(['preflight', '--config', file], { TAURI_SIGNING_PRIVATE_KEY: key });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /生产更新配置不安全/);
  }
});

test('Tauri 清单可读且两种更新物的独立临时签名验证通过', () => {
  const result = run(['verify', '--config', testConfig, '--artifacts', artifacts]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /验签通过/);
});

test('测试产物生成可读清单并明确标记不可发布，不把私钥带入产物', () => {
  const out = path.join(temp, 'staged-test');
  const notes = path.join(temp, 'notes.txt');
  fs.writeFileSync(notes, '测试更新说明');
  const result = run(['test-manifest', '--config', testConfig, '--bundle-dir', artifacts, '--out', out, '--notes', notes]);
  assert.equal(result.status, 0, result.stderr);
  const text = fs.readFileSync(path.join(out, 'latest.json'), 'utf8');
  const staged = JSON.parse(text);
  assert.equal(staged.version, '0.2.0');
  assert.equal(staged.notes, '测试更新说明');
  assert.equal(staged.platforms['windows-x86_64-msi'].signature, manifest.platforms['windows-x86_64-msi'].signature);
  assert.match(staged.platforms['windows-x86_64-nsis'].url, /^https:\/\/example\.invalid\//);
  assert.match(text, /\n  "version"/);
  assert.ok(fs.existsSync(path.join(out, 'TEST-ONLY-NOT-FOR-RELEASE')));
  assert.ok(fs.existsSync(path.join(out, 'SHA256SUMS')));
  assert.ok(!fs.readdirSync(out).some(name => name.endsWith('.key')));
});

test('生产发布复核拒绝临时测试产物', () => {
  fs.writeFileSync(path.join(artifacts, 'TEST-ONLY-NOT-FOR-RELEASE'), '测试产物');
  try {
    const result = run(['verify', '--config', testConfig, '--artifacts', artifacts, '--production']);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /临时测试产物不能发布/);
  } finally {
    fs.unlinkSync(path.join(artifacts, 'TEST-ONLY-NOT-FOR-RELEASE'));
  }
});

test('删除测试标记也不能把测试端点和混杂文件冒充正式发布', () => {
  const result = run(['verify', '--config', testConfig, '--artifacts', artifacts, '--production']);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /生产更新配置不安全/);
});

test('有效旧签名不能冒充更高的清单版本', () => {
  fs.writeFileSync(path.join(artifacts, 'latest.json'), JSON.stringify({ ...manifest, version: '9.9.9' }));
  try {
    const result = run(['verify', '--config', testConfig, '--artifacts', artifacts]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /签名版本与清单版本不一致/);
  } finally {
    fs.writeFileSync(path.join(artifacts, 'latest.json'), JSON.stringify(manifest));
  }
});
