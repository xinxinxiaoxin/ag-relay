import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import * as release from './release.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));

function fixture(t) {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'ag-relay-release-flow-'));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  return home;
}

test('正式打包默认复用当前版本的产物目录与说明，显式参数仍可覆盖', () => {
  assert.deepEqual(release.releasePaths('1.2.3'), {
    out: path.join(root, 'release-artifacts/v1.2.3'),
    notes: path.join(root, 'docs/releases/v1.2.3.md'),
  });
  assert.deepEqual(release.releasePaths('1.2.3', { out: 'custom-out', notes: 'custom.md' }), {
    out: 'custom-out', notes: 'custom.md',
  });
});

test('原签名目录可直接使用，不修改调用者环境或读取私钥正文', t => {
  const home = fixture(t);
  const dir = path.join(home, '.ag-relay-signing');
  fs.mkdirSync(dir);
  const key = path.join(dir, 'ag-relay.key');
  fs.writeFileSync(key, 'fixture-only-private-material');
  fs.writeFileSync(`${key}.pub`, 'fixture-public\n');
  const original = { PATH: 'fixture-path' };
  const env = release.signingEnvironment('fixture-public', { env: original, home });
  assert.equal(env.TAURI_SIGNING_PRIVATE_KEY, key);
  assert.equal(env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD, '');
  assert.deepEqual(original, { PATH: 'fixture-path' });
  assert.ok(!Object.values(env).includes('fixture-only-private-material'));
});

test('显式签名环境优先；无原签名时交由既有预检报告', t => {
  const home = fixture(t);
  const explicit = { TAURI_SIGNING_PRIVATE_KEY: 'explicit-key', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'explicit-password' };
  assert.deepEqual(release.signingEnvironment('unused', { env: explicit, home }), explicit);
  assert.deepEqual(release.signingEnvironment('unused', { env: {}, home }), {});
});

test('原签名公钥不匹配时提前停止，不回显私钥', t => {
  const home = fixture(t);
  const dir = path.join(home, '.ag-relay-signing');
  fs.mkdirSync(dir);
  fs.writeFileSync(path.join(dir, 'ag-relay.key'), 'fixture-secret-not-for-output');
  fs.writeFileSync(path.join(dir, 'ag-relay.key.pub'), 'other-public');
  assert.throws(() => release.signingEnvironment('expected-public', { env: {}, home }), error => {
    assert.match(error.message, /公钥.*不一致/);
    assert.ok(!error.message.includes('fixture-secret-not-for-output'));
    return true;
  });
});

function uploadFixture(t) {
  const dir = path.join(fixture(t), 'artifacts with spaces');
  fs.mkdirSync(dir);
  const files = ['ag-relay_1.2.3_x64-setup.exe', 'ag-relay_1.2.3_x64-setup.exe.sig',
    'ag-relay_1.2.3_x64_zh-CN.msi', 'ag-relay_1.2.3_x64_zh-CN.msi.sig',
    'third-party-sources.tar.gz', 'SHA256SUMS'];
  for (const file of files) fs.writeFileSync(path.join(dir, file), `fixture ${file}`);
  fs.writeFileSync(path.join(dir, 'latest.json'), JSON.stringify({ version: '1.2.3', notes: '打包时的说明\n第二行' }));
  const names = fs.readdirSync(dir).sort();
  const asset = name => ({ name, digest: `sha256:${createHash('sha256').update(fs.readFileSync(path.join(dir, name))).digest('hex')}` });
  return { dir, names, asset };
}

test('上传先复用正式验签，再从已打包说明创建草稿并自动带齐七个文件', t => {
  const { dir, names } = uploadFixture(t);
  const calls = [];
  let verified = false;
  const result = release.upload('fixture-config', dir, {
    verifyArtifacts(config, artifacts, production) {
      assert.deepEqual([config, artifacts, production], ['fixture-config', dir, true]);
      verified = true;
    },
    runGh(args, options) {
      assert.equal(verified, true);
      calls.push({ args, options });
      if (args[1] === 'view') return null;
      if (args[1] === 'create') return 'https://github.com/xinxinxiaoxin/ag-relay/releases/tag/v1.2.3\n';
      return '';
    },
  });
  const create = calls.find(call => call.args[1] === 'create');
  assert.ok(create.args.includes('--draft'));
  assert.ok(create.args.includes('--verify-tag'));
  assert.ok(!create.args.includes('--target'));
  assert.deepEqual(create.args.slice(-2), ['--notes-file', '-']);
  assert.equal(create.options.input, '打包时的说明\n第二行');
  const uploaded = calls.find(call => call.args[1] === 'upload');
  assert.deepEqual(uploaded.args.slice(5).sort(), names.map(name => path.join(dir, name)).sort());
  assert.ok(!uploaded.args.includes('--clobber'));
  assert.equal(result.uploaded, 7);
  assert.equal(result.tag, 'v1.2.3');
  assert.ok(!calls.some(call => call.args[1] === 'edit'));
});

test('重跑上传只补传草稿缺少的文件，不重建或覆盖已有文件', t => {
  const { dir, names, asset } = uploadFixture(t);
  const calls = [];
  const result = release.upload('fixture-config', dir, {
    verifyArtifacts() {},
    runGh(args) {
      calls.push(args);
      if (args[1] === 'view') return JSON.stringify({ isDraft: true, url: 'https://example.test/draft', assets: names.slice(0, 5).map(asset) });
      return '';
    },
  });
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[1].slice(5), names.slice(5).map(name => path.join(dir, name)));
  assert.equal(result.uploaded, 2);
});

test('草稿已有全部同内容文件时不重复上传', t => {
  const { dir, names, asset } = uploadFixture(t);
  const result = release.upload('fixture-config', dir, {
    verifyArtifacts() {},
    runGh(args) {
      assert.equal(args[1], 'view');
      return JSON.stringify({ isDraft: true, assets: names.map(asset) });
    },
  });
  assert.equal(result.uploaded, 0);
});

test('已公开版本和同名但不同内容的草稿文件均不覆盖', t => {
  const { dir, names } = uploadFixture(t);
  for (const existing of [
    { isDraft: false, assets: [] },
    { isDraft: true, assets: [{ name: names[0], digest: 'sha256:different' }] },
  ]) {
    assert.throws(() => release.upload('fixture-config', dir, {
      verifyArtifacts() {},
      runGh(args) {
        assert.equal(args[1], 'view');
        return JSON.stringify(existing);
      },
    }), /已公开|不一致/);
  }
});

test('本地验签失败时不调用 GitHub', t => {
  const { dir } = uploadFixture(t);
  assert.throws(() => release.upload('fixture-config', dir, {
    verifyArtifacts() { throw new Error('fixture verification failed'); },
    runGh() { assert.fail('不能访问 GitHub'); },
  }), /fixture verification failed/);
});
