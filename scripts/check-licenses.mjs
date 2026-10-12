// 离线、只读校验；不执行随包二进制，不联网，也不启动网关。
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

try {
  const { values } = parseArgs({
    options: { root: { type: 'string' }, 'rust-version': { type: 'string' } },
  });
  const root = resolve(values.root ?? resolve(dirname(fileURLToPath(import.meta.url)), '..'));
  const read = (path) => {
    try { return readFileSync(resolve(root, path)); }
    catch { throw new Error(`缺少许可材料：${path}`); }
  };
  for (const path of ['LICENSE', 'THIRD_PARTY_NOTICES.md']) {
    if (!read(path).length) throw new Error(`许可文件为空：${path}`);
  }
  const inventory = JSON.parse(read('licenses/inventory.json'));
  if (inventory.schemaVersion !== 1 || !inventory.components?.length || !inventory.inputs?.length) {
    throw new Error('许可清单格式无效或组件/输入为空');
  }
  if (values['rust-version'] !== undefined) {
    const actual = values['rust-version'];
    const standardLibrary = inventory.components.find((component) =>
      component.ecosystem === 'toolchain-runtime' && component.name === 'Rust standard library');
    if (!standardLibrary || actual !== standardLibrary.version) {
      throw new Error(`Rust 工具链与许可快照不一致：实测 ${actual}，材料 ${standardLibrary?.version ?? '缺失'}；请重新核验标准库许可`);
    }
  }
  for (const input of inventory.inputs) {
    const bytes = read(input.path);
    const data = input.normalizeLf ? bytes.toString('utf8').replace(/\r\n/g, '\n') : bytes;
    if (createHash('sha256').update(data).digest('hex') !== input.sha256) {
      throw new Error(`依赖或二进制已变化，请重新核验许可清单：${input.path}`);
    }
  }
  for (const component of inventory.components) {
    if (!component.name || !component.version || !component.license || !component.source || !component.materials?.length) {
      throw new Error(`组件缺少版本、来源或许可材料：${component.name ?? '未知组件'}`);
    }
    for (const material of component.materials) {
      const actual = createHash('sha256').update(read(material.path)).digest('hex');
      if (actual !== material.sha256) throw new Error(`许可原文校验不符：${material.path}`);
    }
  }
  console.log(`许可校验通过：${inventory.components.length} 个组件的材料齐全。`);
} catch (error) {
  console.error(`许可校验失败：${error.message}`);
  process.exitCode = 1;
}
