import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { prepareDistribution, verifyDistribution, verifySourceArchive } from './prepare-distribution.mjs';
import { gzipSync, gunzipSync } from 'node:zlib';
import { stage, verify } from './release.mjs';

const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');
const archiveName = 'third-party-sources.tar.gz';
const moved = ['licenses/installer/nsis/nsis-3.11-source.tar.gz', 'licenses/installer/wix/wix-3.14.1-source.tar.gz'];

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ag-relay-distribution-test-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const files = {
    'LICENSE': '项目许可\n', 'THIRD_PARTY_NOTICES.md': '第三方声明\n',
    'package.json': '{"version":"0.3.3"}\n',
    'licenses/COMPONENTS.md': '完整组件索引\n',
    'licenses/texts/license.txt': '原始版权与许可\n',
    'licenses/installer/nsis/COPYING': 'NSIS 原始许可\n',
    'licenses/installer/wix/LICENSE.TXT': 'WiX 原始许可\n',
    [moved[0]]: '未裁剪 NSIS 源码归档', [moved[1]]: '未裁剪 WiX 源码归档',
  };
  for (const [name, text] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), text);
  }
  const inventory = {
    schemaVersion: 1,
    inputs: [{ path: 'package.json', sha256: hash(files['package.json']), normalizeLf: true }],
    components: [{ name: 'fixture', version: '1.0.0', license: 'MIT', source: 'https://example.invalid/fixture',
      materials: Object.keys(files).filter(name => name.startsWith('licenses/') && !name.endsWith('COMPONENTS.md'))
        .map(name => ({ path: name, sha256: hash(files[name]) })) }],
    distribution: {
      sourceArchives: moved,
      indexes: [{ path: 'licenses/COMPONENTS.md', sha256: hash(files['licenses/COMPONENTS.md']), normalizeLf: true }],
    },
  };
  fs.writeFileSync(path.join(root, 'licenses/inventory.json'), JSON.stringify(inventory));
  return { root, out: path.join(root, 'src-tauri/generated/license-distribution'), files };
}

test('默认准备目录不占用正式发布目录，保留许可原文且不安装迁出的源码和索引', (t) => {
  const { root, out, files } = fixture(t);
  const result = prepareDistribution({ root });
  assert.equal(result.sourceArchive, path.join(out, archiveName));
  assert.equal(fs.existsSync(path.join(root, 'release-artifacts')), false);
  for (const name of moved) assert.equal(fs.existsSync(path.join(out, 'installer/third-party', name.slice('licenses/'.length))), false);
  assert.equal(fs.existsSync(path.join(out, 'installer/third-party/inventory.json')), false);
  assert.equal(fs.existsSync(path.join(out, 'installer/third-party/COMPONENTS.md')), false);
  assert.equal(fs.readFileSync(path.join(out, 'installer/third-party/texts/license.txt'), 'utf8'), files['licenses/texts/license.txt']);
  assert.match(fs.readFileSync(path.join(out, 'installer/来源.txt'), 'utf8'), /releases\/download\/v0\.3\.3\/third-party-sources\.tar\.gz/);
  assert.doesNotMatch(fs.readFileSync(path.join(out, 'installer/来源.txt'), 'utf8'), /latest/);
  assert.equal(verifyDistribution({ root, out }).version, '0.3.3');
  const first = fs.readFileSync(result.sourceArchive);
  const knownTime = new Date('2000-01-01T00:00:00Z');
  fs.utimesSync(result.sourceArchive, knownTime, knownTime);
  prepareDistribution({ root, out });
  assert.deepEqual(fs.readFileSync(result.sourceArchive), first, '同版同材料的附件必须可复现');
  assert.equal(fs.statSync(result.sourceArchive).mtimeMs, knownTime.getTime(), '同内容验证后复用，不触发Cargo输入变化');
});

test('许可检查能核对分发目录，漏许可或漏附件不能只靠仓库原文假通过', (t) => {
  const { root, out } = fixture(t);
  prepareDistribution({ root, out });
  const cli = fileURLToPath(new URL('./check-licenses.mjs', import.meta.url));
  const run = () => spawnSync(process.execPath, [cli, '--root', root, '--distribution', out], { encoding: 'utf8' });
  const complete = run();
  assert.equal(complete.status, 0, complete.stderr);
  const license = path.join(out, 'installer/third-party/texts/license.txt');
  const original = fs.readFileSync(license);
  fs.unlinkSync(license);
  const missingLicense = run();
  assert.notEqual(missingLicense.status, 0);
  assert.match(missingLicense.stderr, /分发文件清单不一致|未记录的空目录/);
  fs.writeFileSync(license, original);
  prepareDistribution({ root, out });
  fs.unlinkSync(path.join(out, archiveName));
  assert.notEqual(run().status, 0);
});

test('发布暂存与正式复核必须一并交付同版来源附件及其校验和', (t) => {
  const { root, out } = fixture(t);
  prepareDistribution({ root, out });
  const config = path.join(root, 'tauri.json');
  fs.writeFileSync(config, JSON.stringify({ version: '0.3.3', plugins: { updater: {
    pubkey: 'fixture-public-key', requireSignedVersion: true,
    endpoints: ['https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json'],
  } } }));
  const bundle = path.join(root, 'bundle');
  fs.mkdirSync(bundle);
  for (const name of ['ag-relay_0.3.3_x64-setup.exe', 'ag-relay_0.3.3_x64_zh-CN.msi']) {
    fs.writeFileSync(path.join(bundle, name), '非可执行安装器夹具');
    fs.writeFileSync(path.join(bundle, `${name}.sig`), '外部验签边界夹具');
  }
  const notes = path.join(root, 'notes.txt');
  fs.writeFileSync(notes, '本次发布');
  const staged = path.join(root, 'staged');
  const options = { sourceRoot: root, verifySignatures: () => {} };
  stage(config, bundle, staged, notes, false, options);
  assert.deepEqual(fs.readFileSync(path.join(staged, archiveName)), fs.readFileSync(path.join(out, archiveName)));
  assert.match(fs.readFileSync(path.join(staged, 'SHA256SUMS'), 'utf8'), /[a-f0-9]{64}  third-party-sources\.tar\.gz\n/);
  assert.doesNotThrow(() => verify(config, staged, true, options));
  const archive = path.join(staged, archiveName);
  const tampered = gunzipSync(fs.readFileSync(archive));
  tarEntry(tampered, 'licenses/installer/nsis/COPYING').data[0] ^= 1;
  fs.writeFileSync(archive, gzipSync(tampered));
  const sums = fs.readdirSync(staged).filter(name => name !== 'SHA256SUMS').sort()
    .map(name => `${hash(fs.readFileSync(path.join(staged, name)))}  ${name}`).join('\n');
  fs.writeFileSync(path.join(staged, 'SHA256SUMS'), `${sums}\n`);
  assert.throws(() => verify(config, staged, true, options), /分发材料摘要不符/, '攻击者重算外层SHA256SUMS也不能绕过内部核验');
  fs.unlinkSync(path.join(staged, archiveName));
  assert.throws(() => verify(config, staged, true, options), /发布目录缺少文件/);
  fs.unlinkSync(path.join(out, archiveName));
  const incomplete = path.join(root, 'incomplete');
  assert.throws(() => stage(config, bundle, incomplete, notes, false, options));
  assert.equal(fs.existsSync(incomplete), false, '附件缺失不能留下看似可发布的目录');
});

function tarEntry(bytes, name) {
  let offset = 0;
  while (offset < bytes.length) {
    const header = bytes.subarray(offset, offset + 512);
    const size = parseInt(header.subarray(124, 136).toString().replace(/\0/g, ''), 8);
    const end = offset + 512 + Math.ceil(size / 512) * 512;
    if (header.subarray(0, 100).toString().replace(/\0/g, '') === name) {
      return { header, offset, end, data: bytes.subarray(offset + 512, offset + 512 + size) };
    }
    offset = end;
  }
  throw new Error(`测试源归档缺少 ${name}`);
}

function changeHeader(header, name, type = '0') {
  header.fill(0, 0, 100);
  header.write(name);
  header[156] = type.charCodeAt(0);
  header.fill(32, 148, 156);
  header.write(`${header.reduce((sum, value) => sum + value, 0).toString(8).padStart(6, '0')}\0 `, 148, 8);
}

test('来源附件逐项核验：内部篡改、错版本、漏许可和额外条目均被拒绝', (t) => {
  const { root, out } = fixture(t);
  const { sourceArchive: file } = prepareDistribution({ root, out });
  const original = fs.readFileSync(file);
  const mutations = [
    [bytes => { tarEntry(bytes, 'licenses/installer/nsis/COPYING').data[0] ^= 1; return bytes; }, /分发材料摘要不符/],
    [bytes => { const data = tarEntry(bytes, 'manifest.json').data; data.write('9.3.3', data.indexOf('0.3.3')); return bytes; }, /版本或发布通道不一致/],
    [bytes => { const entry = tarEntry(bytes, 'licenses/installer/nsis/COPYING'); return Buffer.concat([bytes.subarray(0, entry.offset), bytes.subarray(entry.end)]); }, /缺失或额外文件/],
    [bytes => { const entry = tarEntry(bytes, 'LICENSE'); const extra = Buffer.from(bytes.subarray(entry.offset, entry.end)); changeHeader(extra.subarray(0, 512), 'extra.txt'); return Buffer.concat([extra, bytes]); }, /缺失或额外文件/],
  ];
  for (const [mutate, error] of mutations) {
    fs.writeFileSync(file, gzipSync(mutate(gunzipSync(original))));
    assert.throws(() => verifySourceArchive({ root, file }), error);
  }
  fs.writeFileSync(file, original);
  assert.equal(verifySourceArchive({ root, file }).version, '0.3.3');
});

test('来源附件拒绝路径穿越、绝对路径、链接、重复项、大小写冲突和超限输入', (t) => {
  const { root, out } = fixture(t);
  const { sourceArchive: file } = prepareDistribution({ root, out });
  const original = fs.readFileSync(file);
  for (const [name, type] of [
    ['../outside', '0'], ['/absolute', '0'], ['C:/outside', '0'], ['dir\\outside', '0'],
    ['CON.txt', '0'], ['bad?.txt', '0'], ['LICENSE', '2'], ['LICENSE', '1'], ['LICENSE', '5'],
  ]) {
    const bytes = gunzipSync(original);
    changeHeader(tarEntry(bytes, 'LICENSE').header, name, type);
    fs.writeFileSync(file, gzipSync(bytes));
    assert.throws(() => verifySourceArchive({ root, file }), /不安全的分发路径|只允许普通文件/);
  }
  for (const name of ['LICENSE', 'license']) {
    const bytes = gunzipSync(original);
    const entry = tarEntry(bytes, 'LICENSE');
    const duplicate = Buffer.from(bytes.subarray(entry.offset, entry.end));
    changeHeader(duplicate.subarray(0, 512), name);
    fs.writeFileSync(file, gzipSync(Buffer.concat([duplicate, bytes])));
    assert.throws(() => verifySourceArchive({ root, file }), /重复项或大小写冲突/);
  }
  fs.writeFileSync(file, Buffer.alloc(0));
  fs.truncateSync(file, 64 * 1024 * 1024 + 1);
  assert.throws(() => verifySourceArchive({ root, file }), /超出大小限制/);
  fs.writeFileSync(file, gzipSync(Buffer.alloc(128 * 1024 * 1024 + 512)));
  assert.throws(() => verifySourceArchive({ root, file }), /larger than|超出/);
  const unsafeMode = gunzipSync(original);
  const header = tarEntry(unsafeMode, 'LICENSE').header;
  header.write('0004755\0', 100, 8);
  changeHeader(header, 'LICENSE');
  fs.writeFileSync(file, gzipSync(unsafeMode));
  assert.throws(() => verifySourceArchive({ root, file }), /元数据/);
  fs.writeFileSync(file, gzipSync(gunzipSync(original).subarray(0, -1024)));
  assert.throws(() => verifySourceArchive({ root, file }), /尾部无效/);
});

test('测试材料支持独立新暂存目录，禁止覆盖非专用目录或冒充正式来源附件', (t) => {
  const { root } = fixture(t);
  const out = path.join(root, 'isolated-test');
  const prepared = prepareDistribution({ root, out, version: '0.2.0', testOnly: true });
  assert.equal(verifySourceArchive({ root, file: prepared.sourceArchive, version: '0.2.0', testOnly: true }).version, '0.2.0');
  assert.throws(() => verifySourceArchive({ root, file: prepared.sourceArchive }), /版本或发布通道不一致/);
  assert.throws(() => prepareDistribution({ root, out, testOnly: true }), /新目录/);
  assert.ok(fs.existsSync(prepared.sourceArchive), '拒绝时不得删除来源不明的已有目录');
});

test('校验和重建都拒绝目录链接，不读取或删除链接后的外部材料', (t) => {
  const { root, out } = fixture(t);
  prepareDistribution({ root, out });
  const external = path.join(root, 'external-installer');
  fs.renameSync(path.join(out, 'installer'), external);
  fs.symlinkSync(external, path.join(out, 'installer'), process.platform === 'win32' ? 'junction' : 'dir');
  assert.throws(() => verifyDistribution({ root, out }), /符号链接/);
  fs.unlinkSync(path.join(out, 'installer'));
  fs.rmSync(out, { recursive: true });
  fs.symlinkSync(external, out, process.platform === 'win32' ? 'junction' : 'dir');
  assert.throws(() => prepareDistribution({ root, out }), /符号链接/);
  assert.ok(fs.existsSync(path.join(external, 'LICENSE')));
});

test('验签子进程延续已核验的分发通道，不继承外部通道或生产私钥', (t) => {
  const { root, out } = fixture(t);
  const config = path.join(root, 'tauri.json');
  const saved = Object.fromEntries(['AG_RELAY_TEST_DISTRIBUTION', 'TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PATH', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD']
    .map(name => [name, process.env[name]]));
  t.after(() => {
    for (const [name, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  });
  Object.assign(process.env, { TAURI_SIGNING_PRIVATE_KEY: 'not-a-real-key', TAURI_SIGNING_PRIVATE_KEY_PATH: 'not-a-real-path', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'not-a-real-password' });
  for (const testOnly of [true, false]) {
    prepareDistribution({ root, testOnly });
    process.env.AG_RELAY_TEST_DISTRIBUTION = testOnly ? '0' : '1';
    fs.writeFileSync(config, JSON.stringify({ version: '0.3.3', plugins: { updater: {
      endpoints: [testOnly ? 'https://example.invalid/ag-relay-test/latest.json' : 'https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json'],
    } } }));
    let called = false;
    verify(config, out, false, { sourceRoot: root, verifySignatures: (_config, _artifacts, { env }) => {
      called = true;
      assert.equal(env.AG_RELAY_TEST_DISTRIBUTION, testOnly ? '1' : '0');
      for (const name of ['TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PATH', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD']) assert.equal(env[name], undefined);
    } });
    assert.equal(called, true);
  }
});

test('无效输入或未经核验的旧目录不能触发删除，人工文件原样保留', (t) => {
  const { root, out } = fixture(t);
  fs.mkdirSync(out, { recursive: true });
  const manual = path.join(out, '人工材料.txt');
  fs.writeFileSync(manual, '不可删除');
  fs.writeFileSync(path.join(root, 'package.json'), '{"version":"9.9.9"}');
  assert.throws(() => prepareDistribution({ root }), /依赖或二进制已变化/);
  assert.equal(fs.readFileSync(manual, 'utf8'), '不可删除');
  fs.writeFileSync(path.join(root, 'package.json'), '{"version":"0.3.3"}\n');
  assert.throws(() => prepareDistribution({ root }), /非生成目录|生成记录/);
  assert.equal(fs.readFileSync(manual, 'utf8'), '不可删除');
});

test('已经生成的目录中人工追加或修改文件会拒绝替换，恢复后可安全重建', (t) => {
  const { root, out } = fixture(t);
  prepareDistribution({ root });
  const manual = path.join(out, 'installer/人工材料.txt');
  fs.writeFileSync(manual, '人工补充');
  assert.throws(() => prepareDistribution({ root }), /文件清单不一致/);
  assert.equal(fs.readFileSync(manual, 'utf8'), '人工补充');
  fs.unlinkSync(manual);
  const license = path.join(out, 'installer/LICENSE');
  const original = fs.readFileSync(license);
  fs.writeFileSync(license, '人工修改');
  assert.throws(() => prepareDistribution({ root }), /摘要不符/);
  assert.equal(fs.readFileSync(license, 'utf8'), '人工修改');
  fs.writeFileSync(license, original);
  assert.doesNotThrow(() => prepareDistribution({ root }));
  const empty = path.join(out, 'installer/人工空目录');
  fs.mkdirSync(empty);
  fs.appendFileSync(path.join(root, 'THIRD_PARTY_NOTICES.md'), '合法声明更新');
  assert.throws(() => prepareDistribution({ root }), /空目录/);
  assert.ok(fs.existsSync(empty));
});
