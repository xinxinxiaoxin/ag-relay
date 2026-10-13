import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const script = fileURLToPath(new URL('./bump-version.mjs', import.meta.url));

// 五处清单 + 干扰项：依赖声明里的 version、另一个包的 lock 条目，都不该被改到。
async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'ag-relay-bump-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'src-tauri'));
  await writeFile(join(root, 'package.json'), JSON.stringify({ name: 'ag-relay', version: '0.3.2' }, null, 2) + '\n');
  await writeFile(join(root, 'package-lock.json'), JSON.stringify({
    name: 'ag-relay', version: '0.3.2', lockfileVersion: 3, packages: { '': { name: 'ag-relay', version: '0.3.2' } },
  }, null, 2) + '\n');
  await writeFile(join(root, 'src-tauri/Cargo.toml'), [
    '[package]', 'name = "ag-relay"', 'version = "0.3.2"', '',
    '[dependencies]', 'serde = { version = "1.0.219", features = ["derive"] }', '',
  ].join('\n'));
  await writeFile(join(root, 'src-tauri/Cargo.lock'), [
    'version = 4', '',
    '[[package]]', 'name = "ag-relay"', 'version = "0.3.2"', 'dependencies = [', ' "serde",', ']', '',
    '[[package]]', 'name = "serde"', 'version = "1.0.219"', '',
  ].join('\n'));
  await writeFile(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify({ productName: 'ag-relay', version: '0.3.2' }, null, 2) + '\n');
  return root;
}

const run = (root, version) => spawnSync(process.execPath, [script, version, '--root', root], { encoding: 'utf8' });

test('五处版本号同步更新，且不误改依赖声明与其他包的锁条目', async (t) => {
  const root = await fixture(t);
  const result = run(root, '0.4.0');
  assert.equal(result.status, 0, result.stderr);

  const pkg = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  const npmLock = JSON.parse(await readFile(join(root, 'package-lock.json'), 'utf8'));
  const conf = JSON.parse(await readFile(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  const cargo = await readFile(join(root, 'src-tauri/Cargo.toml'), 'utf8');
  const cargoLock = await readFile(join(root, 'src-tauri/Cargo.lock'), 'utf8');

  assert.equal(pkg.version, '0.4.0');
  assert.equal(npmLock.version, '0.4.0');
  assert.equal(npmLock.packages[''].version, '0.4.0');
  assert.equal(conf.version, '0.4.0');
  assert.match(cargo, /^version = "0\.4\.0"$/m, 'Cargo.toml 包版本应更新');
  assert.match(cargo, /serde = \{ version = "1\.0\.219"/, '依赖声明的 version 不能被改');
  assert.match(cargoLock, /name = "ag-relay"\nversion = "0\.4\.0"/, 'Cargo.lock 中 ag-relay 条目应更新');
  assert.match(cargoLock, /name = "serde"\nversion = "1\.0\.219"/, 'Cargo.lock 中其他包不能被改');
});

test('非法版本号被拒绝且不写入任何文件', async (t) => {
  const root = await fixture(t);
  for (const bad of ['0.4', 'v0.4.0', '0.4.0-beta.1', '']) {
    const result = run(root, bad);
    assert.notEqual(result.status, 0, `应拒绝版本号 ${JSON.stringify(bad)}`);
  }
  const pkg = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  assert.equal(pkg.version, '0.3.2', '拒绝后原版本号必须保持不动');
});

test('锁文件缺少本包条目时中止，且不留下半套版本号', async (t) => {
  const root = await fixture(t);
  await writeFile(join(root, 'src-tauri/Cargo.lock'), 'version = 4\n\n[[package]]\nname = "serde"\nversion = "1.0.219"\n');
  const result = run(root, '0.4.0');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /找不到 ag-relay 包条目/);

  const pkg = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  const conf = JSON.parse(await readFile(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  assert.equal(pkg.version, '0.3.2', 'package.json 不应被改');
  assert.equal(conf.version, '0.3.2', 'tauri.conf.json 不应被改');
});
