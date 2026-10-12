import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
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

test('正式构建必须同时保有 NSIS 和 WiX 运行时的非空许可材料', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'licenses/fixture-LICENSE'), '原始许可\n');
  const inventoryPath = join(root, 'licenses/inventory.json');
  const inventory = JSON.parse(await readFile(inventoryPath, 'utf8'));
  const installers = ['NSIS installer runtime', 'WiX installer runtime'].map((name) => ({
    name, ecosystem: 'installer-runtime', version: '1.0.0', license: 'MIT', source: 'https://example.org/installer/1.0.0',
    materials: [{ path: 'licenses/fixture-LICENSE', sha256: hash('原始许可\n') }],
  }));
  assert.equal(run(root).status, 0, '普通文件检查不依赖尚未探测的安装器');
  const missingBoth = run(root, ['--installer-runtime']);
  assert.notEqual(missingBoth.status, 0);
  assert.match(missingBoth.stderr, /缺少安装器运行时许可.*NSIS installer runtime.*WiX installer runtime/);
  inventory.components.push(installers[0]);
  await writeFile(inventoryPath, JSON.stringify(inventory));
  const missingWix = run(root, ['--installer-runtime']);
  assert.notEqual(missingWix.status, 0);
  assert.match(missingWix.stderr, /缺少安装器运行时许可.*WiX installer runtime/);
  inventory.components.push(installers[1]);
  await writeFile(inventoryPath, JSON.stringify(inventory));
  const complete = run(root, ['--installer-runtime', '--rust-version', '1.99.0']);
  assert.equal(complete.status, 0, complete.stderr);
  installers[0].materials = [];
  await writeFile(inventoryPath, JSON.stringify(inventory));
  const empty = run(root, ['--installer-runtime']);
  assert.notEqual(empty.status, 0);
  assert.match(empty.stderr, /缺少安装器运行时许可.*NSIS installer runtime/);
});

test('材料已齐全但仍有未决来源时正式检查不能假通过', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'licenses/fixture-LICENSE'), '原始许可\n');
  const path = join(root, 'licenses/inventory.json');
  const inventory = JSON.parse(await readFile(path, 'utf8'));
  inventory.components.push(...['NSIS installer runtime', 'WiX installer runtime'].map((name) => ({
    name, ecosystem: 'installer-runtime', version: '1.0.0', license: 'MIT', source: 'https://example.org/installer/1.0.0',
    materials: [{ path: 'licenses/fixture-LICENSE', sha256: hash('原始许可\n') }],
  })));
  inventory.releaseBlockers = [{ component: 'nsis_tauri_utils', reason: '预编译依赖版本未能证实' }];
  await writeFile(path, JSON.stringify(inventory));
  assert.equal(run(root).status, 0, '保留普通材料核查能力，不把它当成正式验收');
  const blocked = run(root, ['--installer-runtime']);
  assert.notEqual(blocked.status, 0);
  assert.match(blocked.stderr, /正式分发许可来源未闭合.*nsis_tauri_utils/);
});

test('未决来源只有在维护者留下完整接受决定后才允许正式放行', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'licenses/fixture-LICENSE'), '原始许可\n');
  const path = join(root, 'licenses/inventory.json');
  const inventory = JSON.parse(await readFile(path, 'utf8'));
  inventory.components.push(...['NSIS installer runtime', 'WiX installer runtime'].map((name) => ({
    name, ecosystem: 'installer-runtime', version: '1.0.0', license: 'MIT', source: 'https://example.org/installer/1.0.0',
    materials: [{ path: 'licenses/fixture-LICENSE', sha256: hash('原始许可\n') }],
  })));
  inventory.releaseBlockers = [{
    component: 'nsis_tauri_utils', reason: '预编译依赖版本未能证实',
    decision: { decidedBy: '', decidedAt: '2026-10-12', rationale: '接受该来源限制' },
  }];
  await writeFile(path, JSON.stringify(inventory));
  assert.notEqual(run(root, ['--installer-runtime']).status, 0, '缺少决策人仍应拒绝');

  inventory.releaseBlockers[0].decision.decidedBy = '维护者';
  inventory.releaseBlockers[0].decision.decidedAt = '2026/10/12';
  await writeFile(path, JSON.stringify(inventory));
  assert.notEqual(run(root, ['--installer-runtime']).status, 0, '日期格式不合规仍应拒绝');

  inventory.releaseBlockers[0].decision.decidedAt = '2026-10-12';
  inventory.releaseBlockers[0].decision.rationale = '   ';
  await writeFile(path, JSON.stringify(inventory));
  assert.notEqual(run(root, ['--installer-runtime']).status, 0, '空理由仍应拒绝');

  inventory.releaseBlockers[0].decision.rationale = '接受官方预编译来源及其不可重复性限制';
  await writeFile(path, JSON.stringify(inventory));
  const accepted = run(root, ['--installer-runtime', '--rust-version', '1.99.0']);
  assert.equal(accepted.status, 0, accepted.stderr);
  assert.match(accepted.stdout, /已记录维护者接受决定.*nsis_tauri_utils/);
});
