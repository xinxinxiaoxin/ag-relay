import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { gzipSync, gunzipSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { checkLicenses } from './check-licenses.mjs';

export const sourceArchiveName = 'third-party-sources.tar.gz';
const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sourceArchives = ['licenses/installer/nsis/nsis-3.11-source.tar.gz', 'licenses/installer/wix/wix-3.14.1-source.tar.gz'];
const maxArchiveBytes = 64 * 1024 * 1024;
const maxTarBytes = 128 * 1024 * 1024;
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const json = value => Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
export const distributionPath = root => path.join(root, 'src-tauri/generated/license-distribution');

function safeName(name) {
  if (typeof name !== 'string' || !name || /[\\:<>"|?*\x00-\x1f\x7f]/.test(name)
      || name.split('/').some(part => !part || part === '.' || part === '..' || /[. ]$/.test(part)
        || /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part))) {
    throw new Error(`不安全的分发路径：${name}`);
  }
  return name;
}

function regularPath(root, name, missing = false) {
  safeName(name);
  let current = root;
  for (const part of name.split('/')) {
    current = path.join(current, part);
    const stat = fs.lstatSync(current, { throwIfNoEntry: false });
    if (missing && !stat) continue;
    if (!stat) throw new Error(`缺少分发材料：${name}`);
    if (stat.isSymbolicLink()) throw new Error(`分发材料不允许符号链接：${name}`);
  }
  return current;
}

function noLinks(file, missing = false) {
  const absolute = path.resolve(file);
  const base = path.parse(absolute).root;
  return regularPath(base, path.relative(base, absolute).split(path.sep).join('/'), missing);
}

function read(root, name) {
  const file = regularPath(root, name);
  if (!fs.statSync(file).isFile()) throw new Error(`分发材料不是普通文件：${name}`);
  return fs.readFileSync(file);
}

function filesUnder(root, prefix = '') {
  const files = [];
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const name = safeName(`${prefix}${entry.name}`);
    if (entry.isSymbolicLink()) throw new Error(`分发材料不允许符号链接：${name}`);
    if (entry.isDirectory()) {
      const nested = filesUnder(path.join(root, entry.name), `${name}/`);
      if (!nested.length) throw new Error(`分发目录包含未记录的空目录：${name}`);
      files.push(...nested);
    } else if (entry.isFile()) files.push(name);
    else throw new Error(`分发材料不是普通文件：${name}`);
  }
  return files.sort();
}

function sameFiles(actual, expected) {
  if (JSON.stringify([...actual].sort()) !== JSON.stringify([...expected].sort())) {
    throw new Error('分发文件清单不一致：存在缺失或额外文件');
  }
}

function distributionContents(root, version, testOnly) {
  noLinks(root);
  const inventory = checkLicenses({ root });
  const productVersion = JSON.parse(read(root, 'package.json')).version;
  version ??= productVersion;
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)
      || (!testOnly && version !== productVersion)) throw new Error('来源附件版本与产品版本不一致');
  if (JSON.stringify(inventory.distribution?.sourceArchives) !== JSON.stringify(sourceArchives)
      || inventory.distribution?.indexes?.length !== 1
      || inventory.distribution.indexes[0].path !== 'licenses/COMPONENTS.md') {
    throw new Error('许可分发策略缺失或包含未审核的迁出材料');
  }
  const contents = new Map();
  const names = new Map();
  const add = (name, bytes) => {
    safeName(name);
    const previous = names.get(name.toLowerCase());
    if (previous && previous !== name) throw new Error(`分发路径大小写冲突：${name}`);
    if (contents.has(name) && !contents.get(name).equals(bytes)) throw new Error(`许可清单摘要冲突：${name}`);
    names.set(name.toLowerCase(), name);
    contents.set(name, bytes);
  };
  const materials = [...inventory.components.flatMap(component => component.materials), ...inventory.distribution.indexes];
  for (const material of materials) {
    if (!material.path.startsWith('licenses/')) throw new Error('许可材料必须位于 licenses 目录');
    const original = read(root, material.path);
    const bytes = material.normalizeLf ? Buffer.from(original.toString('utf8').replace(/\r\n/g, '\n')) : original;
    if (hash(bytes) !== material.sha256) throw new Error(`许可原文校验不符：${material.path}`);
    add(material.path, bytes);
  }
  for (const name of sourceArchives) if (!contents.has(name)) throw new Error(`缺少固定源码：${name}`);
  add('licenses/inventory.json', json(inventory));
  regularPath(root, 'licenses');
  sameFiles(filesUnder(path.join(root, 'licenses')).map(name => `licenses/${name}`), contents.keys());
  for (const input of inventory.inputs.filter(input => input.normalizeLf)) {
    add(input.path, Buffer.from(read(root, input.path).toString('utf8').replace(/\r\n/g, '\n')));
  }
  for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) add(name, read(root, name));
  const sorted = new Map([...contents].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0));
  const manifest = { schemaVersion: 1, product: 'ag-relay', version, testOnly,
    binaryInputs: inventory.inputs.filter(input => !input.normalizeLf),
    files: [...sorted].map(([name, bytes]) => ({ path: name, size: bytes.length, sha256: hash(bytes) })) };
  sorted.set('manifest.json', json(manifest));
  const excluded = new Set([...sourceArchives, 'licenses/inventory.json', 'licenses/COMPONENTS.md']);
  const installer = new Map([...contents].filter(([name]) => name.startsWith('licenses/') && !excluded.has(name))
    .map(([name, bytes]) => [`third-party/${name.slice('licenses/'.length)}`, bytes]));
  for (const name of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) installer.set(name, contents.get(name));
  installer.set('来源.txt', Buffer.from(`AG Relay ${version} 第三方材料\n${testOnly ? '仅测试材料，不可正式发布。\n' : ''}版权与许可原文保留在本目录。NSIS 3.11、WiX 3.14.1 的完整未修改源码（含文件级许可、构建材料）及完整取证清单由本项目同版附件免费提供：\nhttps://github.com/xinxinxiaoxin/ag-relay/releases/download/v${version}/${sourceArchiveName}\n附件包含完整 licenses/、项目许可声明、已核验的文本构建输入及 manifest.json；Node 二进制只记录摘要。请同时保留和分发该同版附件，不用项目 MIT 替代第三方条款。\n`));
  return { archive: sorted, installer, manifest };
}

function tarGzip(files) {
  const blocks = [];
  let total = 1024;
  for (const [name, bytes] of files) {
    const header = Buffer.alloc(512);
    let leaf = name;
    let prefix = '';
    if (Buffer.byteLength(name) > 100) {
      const slash = name.lastIndexOf('/');
      prefix = name.slice(0, slash);
      leaf = name.slice(slash + 1);
    }
    if (Buffer.byteLength(leaf) > 100 || Buffer.byteLength(prefix) > 155) throw new Error('分发路径超出 USTAR 长度限制');
    header.write(leaf, 0, 100);
    header.write(prefix, 345, 155);
    for (const [offset, width, value] of [[100, 8, 0o644], [108, 8, 0], [116, 8, 0], [124, 12, bytes.length], [136, 12, 0]]) {
      header.write(`${value.toString(8).padStart(width - 1, '0')}\0`, offset, width);
    }
    header.fill(32, 148, 156);
    header[156] = 48;
    header.write('ustar\0', 257, 6);
    header.write('00', 263, 2);
    header.write(`${header.reduce((sum, value) => sum + value, 0).toString(8).padStart(6, '0')}\0 `, 148, 8);
    const padding = Buffer.alloc((512 - bytes.length % 512) % 512);
    blocks.push(header, bytes, padding);
    total += 512 + bytes.length + padding.length;
    if (total > maxTarBytes) throw new Error('来源附件超出大小限制');
  }
  blocks.push(Buffer.alloc(1024));
  const compressed = gzipSync(Buffer.concat(blocks), { level: 9 });
  compressed[9] = 255;
  if (compressed.length > maxArchiveBytes) throw new Error('来源附件超出大小限制');
  return compressed;
}

function archiveFiles(file) {
  noLinks(file);
  if (!fs.statSync(file).isFile()) throw new Error('来源附件必须是普通文件');
  if (fs.statSync(file).size > maxArchiveBytes) throw new Error('来源附件超出大小限制');
  const bytes = gunzipSync(fs.readFileSync(file), { maxOutputLength: maxTarBytes });
  const files = new Map();
  const names = new Set();
  let offset = 0;
  const text = (header, start, length) => {
    const field = header.subarray(start, start + length);
    const zero = field.indexOf(0);
    if (zero >= 0 && field.subarray(zero).some(byte => byte !== 0)) throw new Error('无效的 USTAR 字段');
    return new TextDecoder('utf-8', { fatal: true }).decode(zero < 0 ? field : field.subarray(0, zero));
  };
  while (offset + 512 <= bytes.length && bytes.subarray(offset, offset + 512).some(byte => byte !== 0)) {
    const header = bytes.subarray(offset, offset + 512);
    const octal = (start, length) => {
      const value = header.subarray(start, start + length).toString('latin1').replace(/[\0 ]+$/, '');
      if (!/^[0-7]+$/.test(value)) throw new Error('无效的 USTAR 数字字段');
      return parseInt(value, 8);
    };
    const checksum = header.reduce((sum, byte, index) => sum + (index >= 148 && index < 156 ? 32 : byte), 0);
    if (octal(148, 8) !== checksum || text(header, 257, 6) !== 'ustar' || text(header, 263, 2) !== '00') {
      throw new Error('来源附件 USTAR 头无效');
    }
    if (header[156] !== 48 || header.subarray(157, 257).some(byte => byte !== 0)) throw new Error('来源附件只允许普通文件，禁止链接和特殊条目');
    if (octal(100, 8) !== 0o644 || octal(108, 8) !== 0 || octal(116, 8) !== 0 || octal(136, 12) !== 0
        || header.subarray(265, 345).some(byte => byte !== 0) || header.subarray(500).some(byte => byte !== 0)) {
      throw new Error('来源附件权限或归属元数据不符合固定格式');
    }
    const prefix = text(header, 345, 155);
    const name = safeName(`${prefix ? `${prefix}/` : ''}${text(header, 0, 100)}`);
    if (names.has(name.toLowerCase())) throw new Error(`来源附件有重复项或大小写冲突：${name}`);
    names.add(name.toLowerCase());
    const size = octal(124, 12);
    const end = offset + 512 + size;
    const next = Math.ceil(end / 512) * 512;
    if (end > bytes.length || bytes.subarray(end, next).some(byte => byte !== 0)) throw new Error('来源附件截断或填充无效');
    files.set(name, bytes.subarray(offset + 512, end));
    offset = next;
  }
  if (bytes.length - offset < 1024 || bytes.length % 512 || bytes.subarray(offset).some(byte => byte !== 0)) throw new Error('来源附件尾部无效');
  return files;
}

function compareContents(actual, expected) {
  sameFiles(actual.keys(), expected.keys());
  for (const [name, bytes] of expected) {
    if (hash(actual.get(name)) !== hash(bytes)) throw new Error(`分发材料摘要不符：${name}`);
  }
}

export function verifySourceArchive({ root = projectRoot, file, version, testOnly = false }) {
  const expected = distributionContents(root, version, testOnly);
  const actual = archiveFiles(file);
  const manifest = JSON.parse(actual.get('manifest.json') ?? '{}');
  if (manifest.version !== expected.manifest.version || manifest.testOnly !== testOnly) throw new Error('来源附件版本或发布通道不一致');
  compareContents(actual, expected.archive);
  return manifest;
}

export function verifyDistribution({ root = projectRoot, out = distributionPath(root), version, testOnly = false } = {}) {
  const expected = distributionContents(root, version, testOnly);
  const installer = noLinks(path.join(out, 'installer'));
  const files = new Map(filesUnder(installer).map(name => [name, read(installer, name)]));
  compareContents(files, expected.installer);
  return verifySourceArchive({ root, file: path.join(out, sourceArchiveName), version, testOnly });
}

const generationRecord = '.ag-relay-generated.json';

function verifyGeneratedDirectory(out) {
  noLinks(out);
  if (!fs.existsSync(path.join(out, generationRecord))) throw new Error('拒绝替换非生成目录：缺少生成记录');
  const record = JSON.parse(read(out, generationRecord));
  if (record.schemaVersion !== 1 || record.generator !== 'ag-relay-license-distribution'
      || !Array.isArray(record.files) || record.files.length === 0) throw new Error('许可目录生成记录无效');
  const names = new Set();
  for (const entry of record.files) {
    safeName(entry.path);
    if (names.has(entry.path.toLowerCase()) || entry.path === generationRecord
        || !Number.isSafeInteger(entry.size) || entry.size < 0 || !/^[a-f0-9]{64}$/.test(entry.sha256)) {
      throw new Error('许可目录生成记录无效');
    }
    names.add(entry.path.toLowerCase());
  }
  sameFiles(filesUnder(out), [...record.files.map(entry => entry.path), generationRecord]);
  for (const entry of record.files) {
    const bytes = read(out, entry.path);
    if (bytes.length !== entry.size || hash(bytes) !== entry.sha256) throw new Error(`生成材料摘要不符：${entry.path}`);
  }
  return read(out, generationRecord);
}

export function prepareDistribution({ root = projectRoot, out = distributionPath(root), version, testOnly = false } = {}) {
  out = path.resolve(out);
  noLinks(out, true);
  const contents = distributionContents(root, version, testOnly);
  const archive = tarGzip(contents.archive);
  const installerFiles = [...contents.installer].map(([name, bytes]) => [`installer/${name}`, bytes]);
  const generated = new Map([...installerFiles, [sourceArchiveName, archive]]);
  const record = json({ schemaVersion: 1, generator: 'ag-relay-license-distribution',
    files: [...generated].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
      .map(([name, bytes]) => ({ path: name, size: bytes.length, sha256: hash(bytes) })) });
  const result = { installerDir: path.join(out, 'installer'), sourceArchive: path.join(out, sourceArchiveName), version: contents.manifest.version };
  let previous;
  if (fs.existsSync(out)) {
    if (out !== path.resolve(distributionPath(root))) throw new Error('独立许可暂存必须使用尚不存在的新目录');
    previous = verifyGeneratedDirectory(out);
    if (previous.equals(record)) return result;
  }
  fs.mkdirSync(path.dirname(out), { recursive: true });
  const temporary = fs.mkdtempSync(path.join(path.dirname(out), '.license-distribution-'));
  const next = path.join(temporary, 'next');
  const backup = path.join(temporary, 'previous');
  let replaced = false;
  try {
    for (const [name, bytes] of generated) {
      const file = path.join(next, name);
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, bytes);
    }
    fs.writeFileSync(path.join(next, generationRecord), record);
    verifyGeneratedDirectory(next);
    verifyDistribution({ root, out: next, version, testOnly });
    if (previous) {
      if (!verifyGeneratedDirectory(out).equals(previous)) throw new Error('生成目录已被其他任务修改，拒绝替换');
      fs.renameSync(out, backup);
      try {
        verifyGeneratedDirectory(backup);
        fs.renameSync(next, out);
        replaced = true;
      } catch (error) {
        fs.renameSync(backup, out);
        throw error;
      }
    } else {
      if (fs.existsSync(out)) throw new Error('目标目录已被其他任务创建，拒绝替换');
      fs.renameSync(next, out);
      replaced = true;
    }
    return result;
  } finally {
    if (!fs.existsSync(backup) || replaced) fs.rmSync(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { values } = parseArgs({ options: { root: { type: 'string' } } });
    const result = prepareDistribution({ root: values.root ? path.resolve(values.root) : projectRoot,
      testOnly: process.env.AG_RELAY_TEST_DISTRIBUTION === '1' });
    console.log(`许可分发材料已生成：${result.version}，${result.sourceArchive}`);
  } catch (error) {
    console.error(`许可分发准备失败：${error.message}`);
    process.exitCode = 1;
  }
}
