import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const cli = fileURLToPath(new URL('./check-licenses.mjs', import.meta.url));
const hash = (text) => createHash('sha256').update(text).digest('hex');

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'ag-relay-license-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'licenses'));
  await writeFile(join(root, 'LICENSE'), '项目许可\n');
  await writeFile(join(root, 'THIRD_PARTY_NOTICES.md'), '第三方声明\n');
  await writeFile(join(root, 'package-lock.json'), '{}\n');
  await writeFile(join(root, 'licenses/inventory.json'), JSON.stringify({
    schemaVersion: 1,
    inputs: [{ path: 'package-lock.json', sha256: hash('{}\n'), normalizeLf: true }],
    components: [{ name: 'Rust standard library', ecosystem: 'toolchain-runtime', version: '1.99.0', license: 'MIT', source: 'https://example.org/fixture/1.99.0',
      materials: [{ path: 'licenses/fixture-LICENSE', sha256: hash('原始许可\n') }] }],
  }));
  return root;
}

const run = (root, args = []) => spawnSync(process.execPath, [cli, '--root', root, ...args], { encoding: 'utf8' });

test('打包前缺少清单引用的许可原文时拒绝通过并指出路径', async (t) => {
  const root = await fixture(t);
  const result = run(root);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /缺少许可材料.*licenses\/fixture-LICENSE/);
});

test('原文被截断时拒绝通过，恢复完整原文后通过', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'licenses/fixture-LICENSE'), '删节许可\n');
  const changed = run(root);
  assert.notEqual(changed.status, 0);
  assert.match(changed.stderr, /许可原文校验不符.*fixture-LICENSE/);
  await writeFile(join(root, 'licenses/fixture-LICENSE'), '原始许可\n');
  const restored = run(root);
  assert.equal(restored.status, 0, restored.stderr);
  assert.match(restored.stdout, /许可校验通过/);
});

test('锁文件实际变更要求重新核验，但允许 Windows 换行差异', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'licenses/fixture-LICENSE'), '原始许可\n');
  await writeFile(join(root, 'package-lock.json'), '{}\r\n');
  assert.equal(run(root).status, 0);
  await writeFile(join(root, 'package-lock.json'), '{"dependencies":{"new":"1.0.0"}}\n');
  const changed = run(root);
  assert.notEqual(changed.status, 0);
  assert.match(changed.stderr, /依赖或二进制已变化.*package-lock.json/);
});

test('发布调用者提供的实测 Rust 版本必须与标准库许可快照一致', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'licenses/fixture-LICENSE'), '原始许可\n');
  const changed = run(root, ['--rust-version', '1.100.0']);
  assert.notEqual(changed.status, 0);
  assert.match(changed.stderr, /Rust 工具链与许可快照不一致.*1\.100\.0.*1\.99\.0/);
  const matched = run(root, ['--rust-version', '1.99.0']);
  assert.equal(matched.status, 0, matched.stderr);
  assert.match(matched.stdout, /许可校验通过/);
});
