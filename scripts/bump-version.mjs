// 同步产品版本号到所有清单文件，避免手改漏项导致打包被拒。
// 先全部读完并校验，确认无误才落盘——中途失败不会留下半套版本号。
// 用法：node scripts/bump-version.mjs <X.Y.Z> [--root <目录>]
import fs from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: { root: { type: 'string' } },
});

const next = positionals[0];
if (!/^\d+\.\d+\.\d+$/.test(next ?? '')) {
  console.error('用法：node scripts/bump-version.mjs <X.Y.Z> [--root <目录>]\n版本必须是 MSI 可接受的稳定三段式版本号。');
  process.exit(2);
}

const root = path.resolve(values.root ?? process.cwd());
const read = (p) => fs.readFileSync(path.join(root, p), 'utf8');

// ── 阶段一：全部读出、算出新内容（此时不碰磁盘）────────────────────────────
const packagePath = 'package.json';
const lockPath = 'package-lock.json';
const cargoPath = 'src-tauri/Cargo.toml';
const cargoLockPath = 'src-tauri/Cargo.lock';
const confPath = 'src-tauri/tauri.conf.json';

const pkg = JSON.parse(read(packagePath));
const npmLock = JSON.parse(read(lockPath));
const cargo = read(cargoPath);
const cargoLock = read(cargoLockPath);
const conf = JSON.parse(read(confPath));

const cargoVersion = cargo.match(/^version = "([^"]+)"/m)?.[1];
if (!cargoVersion) {
  console.error(`${cargoPath} 里找不到 [package] 段的 version，已中止（未写入任何文件）。`);
  process.exit(3);
}

// Cargo.lock 是 TOML，但没有解析依赖可用的库：按行定位 ag-relay 的 [[package]] 块。
const lockLines = cargoLock.split(/\r?\n/);
const nameAt = lockLines.findIndex((line) => line === 'name = "ag-relay"');
if (nameAt === -1) {
  console.error(`${cargoLockPath} 里找不到 ag-relay 包条目，已中止（未写入任何文件）。`);
  process.exit(3);
}
const versionAt = lockLines.findIndex((line, i) => i > nameAt && line.startsWith('version = "'));
if (versionAt === -1) {
  console.error(`${cargoLockPath} 的 ag-relay 条目缺少 version 行，已中止（未写入任何文件）。`);
  process.exit(3);
}

const changes = [
  {
    file: packagePath,
    from: pkg.version,
    text: JSON.stringify({ ...pkg, version: next }, null, 2) + '\n',
  },
  {
    file: lockPath,
    from: `${npmLock.version} / packages[""] ${npmLock.packages?.['']?.version}`,
    text: JSON.stringify({
      ...npmLock,
      version: next,
      packages: { ...npmLock.packages, '': { ...npmLock.packages[''], version: next } },
    }, null, 2) + '\n',
  },
  {
    file: cargoPath,
    from: cargoVersion,
    // 只替换第一处（[package] 段），依赖声明里的 version 不受影响
    text: cargo.replace(/^version = "[^"]+"/m, `version = "${next}"`),
  },
  {
    file: cargoLockPath,
    from: lockLines[versionAt],
    text: [...lockLines.slice(0, versionAt), `version = "${next}"`, ...lockLines.slice(versionAt + 1)].join('\n'),
  },
  {
    file: confPath,
    from: conf.version,
    text: JSON.stringify({ ...conf, version: next }, null, 2) + '\n',
  },
];

// ── 阶段二：确认无误后一次性写入 ──────────────────────────────────────────
for (const change of changes) fs.writeFileSync(path.join(root, change.file), change.text);

console.log(`版本号已统一为 ${next}：`);
for (const change of changes) console.log(`  ${change.file}: ${change.from} → ${next}`);
console.log('\n下一步（详见 docs/release-runbook.md）：');
console.log('  1) 重新核验许可清单：先证明依赖集合未变，再刷新输入指纹');
console.log('  2) node scripts/release.mjs preflight');
console.log('  3) 打 tag、触发发布工作流，最后在 GitHub 上点一次发布审批');
