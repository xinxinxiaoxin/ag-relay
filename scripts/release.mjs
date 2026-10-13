// 发布入口默认拒绝未配置生产签名的构建；绝不读取 .env 或输出私钥。
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import { createHash } from 'node:crypto';
import os from 'node:os';
import { distributionPath, prepareDistribution, sourceArchiveName, verifyDistribution, verifySourceArchive } from './prepare-distribution.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const configPath = path.join(root, 'src-tauri/tauri.conf.json');
const endpoint = 'https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json';

function preflight(file = configPath, { signing = true, inputs = true } = {}) {
  const config = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (!config.plugins?.updater?.pubkey?.trim()) throw new Error('生产更新公钥尚未配置；拒绝正式发布');
  if (signing && !process.env.TAURI_SIGNING_PRIVATE_KEY?.trim()) throw new Error('必须通过环境提供 TAURI_SIGNING_PRIVATE_KEY；拒绝正式发布');
  const updater = config.plugins.updater;
  if (JSON.stringify(updater.endpoints) !== JSON.stringify([endpoint])
      || updater.requireSignedVersion !== true || updater.allowDowngrades || updater['allow-downgrades']
      || Object.keys(updater).some(key => key.startsWith('dangerous') && updater[key])) {
    throw new Error('生产更新配置不安全：必须使用正式 HTTPS 端点、版本绑定和默认 TLS 验证，禁止降级');
  }
  if (inputs) checkBuildInputs(config, true);
  return config;
}

export function verify(config, artifacts, production = false, { sourceRoot = root, verifySignatures = verifyUpdaterSignatures } = {}) {
  if (!config || !artifacts) throw new Error('verify 需要 --config 与 --artifacts');
  const settings = JSON.parse(fs.readFileSync(config, 'utf8'));
  if (production && fs.existsSync(path.join(artifacts, 'TEST-ONLY-NOT-FOR-RELEASE'))) {
    throw new Error('临时测试产物不能发布');
  }
  if (production) {
    const settings = preflight(config, { signing: false, inputs: false });
    const manifest = JSON.parse(fs.readFileSync(path.join(artifacts, 'latest.json'), 'utf8'));
    const names = installerNames(settings.version);
    if (manifest.version !== settings.version) throw new Error('清单与发布配置版本不一致');
    const targets = Object.keys(names).map(kind => `windows-x86_64-${kind}`).sort();
    if (JSON.stringify(Object.keys(manifest.platforms).sort()) !== JSON.stringify(targets)) throw new Error('发布清单平台不完整');
    for (const [kind, name] of Object.entries(names)) {
      if (manifest.platforms[`windows-x86_64-${kind}`].url !== `https://github.com/xinxinxiaoxin/ag-relay/releases/download/v${settings.version}/${name}`) {
        throw new Error('发布资产 URL 与正式版本不一致');
      }
    }
    const expected = ['latest.json', sourceArchiveName, ...Object.values(names).flatMap(name => [name, `${name}.sig`])].sort();
    if (JSON.stringify(fs.readdirSync(artifacts).sort()) !== JSON.stringify([...expected, 'SHA256SUMS'].sort())) throw new Error('发布目录缺少文件或混入额外文件');
    const sums = expected.map(name => `${createHash('sha256').update(fs.readFileSync(path.join(artifacts, name))).digest('hex')}  ${name}`).join('\n');
    if (fs.readFileSync(path.join(artifacts, 'SHA256SUMS'), 'utf8') !== `${sums}\n`) throw new Error('发布文件校验和不匹配');
  }
  const testOnly = !production
    && JSON.stringify(settings.plugins?.updater?.endpoints) === JSON.stringify(['https://example.invalid/ag-relay-test/latest.json']);
  verifySourceArchive({ root: sourceRoot, file: path.join(artifacts, sourceArchiveName), version: settings.version, testOnly });
  const env = { ...process.env, AG_RELAY_TEST_DISTRIBUTION: testOnly ? '1' : '0' };
  delete env.TAURI_SIGNING_PRIVATE_KEY;
  delete env.TAURI_SIGNING_PRIVATE_KEY_PATH;
  delete env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;
  verifySignatures(config, artifacts, { env });
}

function verifyUpdaterSignatures(config, artifacts, { env }) {
  const result = spawnSync('cargo', ['run', '--locked', '-j', '2', '--manifest-path',
    path.join(root, 'src-tauri/Cargo.toml'), '--example', 'verify-update', '--', config, artifacts],
  { cwd: root, env, stdio: 'inherit' });
  if (result.status !== 0) throw new Error('更新物验签未通过');
}

function installerNames(version) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)
      || version.split('.').some((part, i) => Number(part) > (i < 2 ? 255 : 65535))) {
    throw new Error('发布版本必须是 MSI 可接受的稳定 X.Y.Z');
  }
  return { nsis: `ag-relay_${version}_x64-setup.exe`, msi: `ag-relay_${version}_x64_zh-CN.msi` };
}

export function stage(configFile, bundleDir, out, notesFile, testOnly, {
  sourceRoot = root, distributionDir = distributionPath(sourceRoot), verifySignatures = verifyUpdaterSignatures,
} = {}) {
  if (!bundleDir || !out || !notesFile) throw new Error('生成清单需要 --bundle-dir、--out、--notes');
  const config = JSON.parse(fs.readFileSync(configFile, 'utf8'));
  const names = installerNames(config.version);
  const base = testOnly ? 'https://example.invalid/ag-relay-test' : `https://github.com/xinxinxiaoxin/ag-relay/releases/download/v${config.version}`;
  const manifest = { version: config.version, notes: fs.readFileSync(notesFile, 'utf8').replace(/^﻿/, ''),
    pub_date: new Date().toISOString(), platforms: {} };
  // 只准创建新目录，防止覆盖现用目录或混入旧版本文件。
  fs.mkdirSync(out);
  try {
    for (const [kind, name] of Object.entries(names)) {
      const source = fs.existsSync(path.join(bundleDir, kind, name))
        ? path.join(bundleDir, kind, name) : path.join(bundleDir, name);
      fs.copyFileSync(source, path.join(out, name));
      fs.copyFileSync(`${source}.sig`, path.join(out, `${name}.sig`));
      manifest.platforms[`windows-x86_64-${kind}`] = { url: `${base}/${name}`,
        signature: fs.readFileSync(`${source}.sig`, 'utf8').trim() };
    }
    verifyDistribution({ root: sourceRoot, out: distributionDir, version: config.version, testOnly });
    fs.copyFileSync(path.join(distributionDir, sourceArchiveName), path.join(out, sourceArchiveName));
    fs.writeFileSync(path.join(out, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
    if (testOnly) {
      fs.writeFileSync(path.join(out, 'TEST-ONLY-NOT-FOR-RELEASE'), '独立临时测试签名，禁止发布，未运行安装验收。\n');
      fs.copyFileSync(configFile, path.join(out, 'tauri.test.json'));
    }
    verify(configFile, out, false, { sourceRoot, verifySignatures });
    const sums = fs.readdirSync(out).sort().map(name => `${createHash('sha256').update(fs.readFileSync(path.join(out, name))).digest('hex')}  ${name}`).join('\n');
    fs.writeFileSync(path.join(out, 'SHA256SUMS'), `${sums}\n`);
    console.log(`${testOnly ? '仅测试' : '正式'}发布物已验证：${path.resolve(out)}`);
  } catch (error) {
    // 此目录刚由本次调用创建，失败时删除不完整清单，不留下貌似可发布的 latest.json。
    fs.rmSync(out, { recursive: true, force: true });
    throw error;
  }
}

// ADR-0005：安装与卸载都不得替应用强停进程、不得由 Restart Manager 重启、不得重启系统、不得降级。
// 产物生成前静态校验配置与安装器模板/hooks 确实声明了该策略；只写注释不算数，缺失即 fail-closed。
function assertInstallerSafetyPolicy(config) {
  const windows = config.bundle?.windows;
  if (windows?.allowDowngrades !== false
      || windows.nsis?.installerHooks !== 'windows/installer-hooks.nsh'
      || JSON.stringify(windows.wix?.fragmentPaths) !== JSON.stringify(['windows/installer-policy.wxs'])
      || JSON.stringify(windows.wix?.componentRefs) !== JSON.stringify(['AGRelayInstallerSafety'])) {
    throw new Error('安装器安全策略缺失：必须拒绝占用目录、禁强停/自动重启并禁止降级');
  }
  // 配置里写了路径还不够：被引用的模板文件必须真实存在且确实覆盖了 Tauri 的强停接缝。
  const hooks = path.join(root, 'src-tauri/windows/installer-hooks.nsh');
  const policy = path.join(root, 'src-tauri/windows/installer-policy.wxs');
  if (!fs.existsSync(hooks) || !fs.existsSync(policy)) {
    throw new Error('安装器安全策略缺失：NSIS hooks 或 WiX 片段文件不存在');
  }
  const hooksText = fs.readFileSync(hooks, 'utf8');
  if (!hooksText.includes('!macroundef CheckIfAppIsRunning') || !hooksText.includes('!macro CheckIfAppIsRunning')) {
    throw new Error('安装器安全策略缺失：NSIS hooks 未覆盖 Tauri 的 CheckIfAppIsRunning 占用检查');
  }
  if (/RestartManager_Shutdown|RmShutdown|RmForceShutdown/.test(hooksText)) {
    throw new Error('安装器安全策略缺失：NSIS hooks 仍包含强停进程的调用');
  }
  const policyText = fs.readFileSync(policy, 'utf8');
  for (const marker of ['MSIRESTARTMANAGERCONTROL', 'DisableShutdown', 'MSIDISABLERMRESTART', 'REBOOT', 'ReallySuppress']) {
    if (!policyText.includes(marker)) {
      throw new Error('安装器安全策略缺失：WiX 片段未声明禁用 Restart Manager 关程序/重启');
    }
  }
}

function checkBuildInputs(config, installerRuntime = false) {
  const versions = [JSON.parse(fs.readFileSync(path.join(root, 'package.json'))).version,
    JSON.parse(fs.readFileSync(path.join(root, 'package-lock.json'))).version,
    JSON.parse(fs.readFileSync(path.join(root, 'package-lock.json'))).packages[''].version,
    fs.readFileSync(path.join(root, 'src-tauri/Cargo.toml'), 'utf8').match(/^version = "([^"]+)"/m)?.[1]];
  installerNames(config.version);
  if (versions.some(version => version !== config.version)) throw new Error('产品版本不一致，拒绝混合版本发布');
  if (config.identifier !== 'ag-relay' || config.productName !== 'ag-relay'
      || config.bundle?.createUpdaterArtifacts !== true
      || JSON.stringify(config.bundle.targets) !== JSON.stringify(['nsis', 'msi'])
      || config.bundle.windows?.wix?.upgradeCode !== 'ba1f2de2-325d-5641-9f4f-cdc34e692497') {
    throw new Error('安装标识或签名安装器配置漂移，拒绝改变既有数据/升级约定');
  }
  assertInstallerSafetyPolicy(config);
  const resources = config.bundle.resources;
  const requiredResources = { 'resources/gateway/': 'resources/gateway/',
    'generated/license-distribution/installer/': 'licenses/' };
  if (config.build?.beforeBuildCommand !== 'node scripts/prepare-distribution.mjs && npm run build'
      || JSON.stringify(config.bundle.externalBin) !== JSON.stringify(['binaries/node'])
      || !resources || Array.isArray(resources)
      || Object.entries(requiredResources).some(([from, to]) => resources[from] !== to)
      || Object.keys(resources).length !== Object.keys(requiredResources).length
      || !fs.existsSync(path.join(root, 'src-tauri/binaries/node-x86_64-pc-windows-msvc.exe'))) {
    throw new Error('随包资源必须包含自带 Node、完整网关及构建前重新生成的许可目录，且不能混入额外资源');
  }
  const rustc = spawnSync('rustc', ['--version'], { cwd: root, encoding: 'utf8' });
  const rustVersion = rustc.stdout?.match(/^rustc (\S+)/)?.[1];
  if (rustc.status !== 0 || !rustVersion) throw new Error('无法读取实际 Rust 工具链版本');
  const licenses = spawnSync(process.execPath, [path.join(root, 'scripts/check-licenses.mjs'), '--rust-version', rustVersion,
    ...(installerRuntime ? ['--installer-runtime'] : [])], { cwd: root, stdio: 'inherit' });
  if (licenses.status !== 0) throw new Error('许可资源校验未通过，拒绝打包');
}

function buildPathMappings() {
  const home = os.homedir();
  const mappings = [
    [home, '/builder'],
    [process.env.CARGO_HOME || path.join(home, '.cargo'), '/cargo'],
    [process.env.RUSTUP_HOME || path.join(home, '.rustup'), '/rustup'],
    [root, '/workspace'],
    [process.env.CARGO_TARGET_DIR || path.join(root, 'src-tauri/target'), '/target'],
  ];
  return mappings.flatMap(([from, to]) => {
    const absolute = path.resolve(from);
    return [...new Set([absolute, absolute.replaceAll('\\', '/')])].map(prefix => [prefix, to]);
  });
}

function auditPaths(file) {
  if (!file) throw new Error('audit-paths 需要 --binary');
  const bytes = fs.readFileSync(file);
  const texts = [bytes.toString('utf8'), bytes.toString('utf16le'), bytes.subarray(1).toString('utf16le')].map(text => text.toLowerCase());
  const found = buildPathMappings().filter(([prefix]) => texts.some(text => text.includes(prefix.toLowerCase()))).length;
  if (found) throw new Error(`本机构建路径残留：匹配 ${found} 项，拒绝发布（不显示路径内容）`);
  console.log('产物路径隐私检查通过');
}

function build(options, testOnly) {
  const config = testOnly ? JSON.parse(fs.readFileSync(configPath, 'utf8')) : preflight();
  if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('仅支持 Windows x64 构建');
  if (!options.out || fs.existsSync(options.out)) throw new Error('--out 必须是一个尚不存在的新目录');
  if (!testOnly && !options.notes) throw new Error('正式构建需要 --notes 更新说明文件');
  if (testOnly) checkBuildInputs(config);
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'ag-relay-build-'));
  const tauri = path.join(root, 'node_modules/@tauri-apps/cli/tauri.js');
  // WiX 会把 sidecar 中转为临时目录下的 node.exe；必须隔离 TEMP/TMP，不能共用用户临时文件。
  const env = { ...process.env, CARGO_BUILD_JOBS: '2', TEMP: temp, TMP: temp, TMPDIR: temp,
    AG_RELAY_TEST_DISTRIBUTION: testOnly ? '1' : '0' };
  // 仅影响本次构建子进程；编码形式避免 Windows 含空格路径被 Cargo 拆成多个参数。
  const inheritedFlags = env.CARGO_ENCODED_RUSTFLAGS !== undefined
    ? env.CARGO_ENCODED_RUSTFLAGS.split('\x1f') : (env.RUSTFLAGS || '').split(/\s+/);
  env.CARGO_ENCODED_RUSTFLAGS = [...inheritedFlags.filter(Boolean),
    ...buildPathMappings().map(([from, to]) => `--remap-path-prefix=${from}=${to}`)].join('\x1f');
  delete env.RUSTFLAGS;
  let activeConfig = configPath;
  let notes = options.notes;
  try {
    if (testOnly) {
      delete env.TAURI_SIGNING_PRIVATE_KEY;
      delete env.TAURI_SIGNING_PRIVATE_KEY_PATH;
      env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '';
      const key = path.join(temp, 'TEST-ONLY.key');
      const generated = spawnSync(process.execPath, [tauri, 'signer', 'generate', '--ci', '-p', '', '-w', key],
        { cwd: root, env, encoding: 'utf8' });
      if (generated.status !== 0) throw new Error('独立临时测试密钥生成失败（不回显密钥）');
      env.TAURI_SIGNING_PRIVATE_KEY = key;
      config.plugins.updater.pubkey = fs.readFileSync(`${key}.pub`, 'utf8').trim();
      config.plugins.updater.endpoints = ['https://example.invalid/ag-relay-test/latest.json'];
      activeConfig = path.join(temp, 'tauri.test.json');
      fs.writeFileSync(activeConfig, JSON.stringify(config));
      if (!notes) {
        notes = path.join(temp, 'notes.txt');
        fs.writeFileSync(notes, '仅临时测试签名：未发布、未安装，不可用于生产更新。');
      }
    }
    const target = path.resolve(process.env.CARGO_TARGET_DIR || path.join(root, 'src-tauri/target'));
    env.CARGO_TARGET_DIR = target;
    console.log(`开始${testOnly ? '临时测试' : '正式'}签名构建（-j 2，不运行安装）：${target}`);
    const started = Date.now();
    const result = spawnSync(process.execPath, [tauri, 'build', '--ci', '--bundles', 'nsis,msi',
      '--target', 'x86_64-pc-windows-msvc', '--config', activeConfig, '--', '--locked', '-j', '2'],
    { cwd: root, env, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 });
    // 构建工具可能回显签名错误输入，先移除环境私钥、文件内容和密码再显示日志。
    let output = `${result.stdout || ''}${result.stderr || ''}`;
    const privateKey = env.TAURI_SIGNING_PRIVATE_KEY;
    const secrets = [privateKey, env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD];
    if (privateKey && fs.existsSync(privateKey)) secrets.push(fs.readFileSync(privateKey, 'utf8').trim());
    for (const secret of secrets.filter(Boolean)) output = output.replaceAll(secret, '[签名材料已隐藏]');
    process.stdout.write(output);
    if (result.status !== 0) throw new Error('Tauri 签名构建失败，未生成发布清单');
    const releaseDir = path.join(target, 'x86_64-pc-windows-msvc/release');
    auditPaths(path.join(releaseDir, 'ag-relay.exe'));
    const bundleDir = path.join(releaseDir, 'bundle');
    for (const [kind, name] of Object.entries(installerNames(config.version))) {
      if (fs.statSync(path.join(bundleDir, kind, name)).mtimeMs < started - 2000) throw new Error('检测到旧安装产物，拒绝生成清单');
    }
    stage(activeConfig, bundleDir, options.out, notes, testOnly);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
}

function main() {
  try {
    const { values, positionals } = parseArgs({ allowPositionals: true, options: {
      config: { type: 'string' }, artifacts: { type: 'string' },
      'bundle-dir': { type: 'string' }, out: { type: 'string' }, notes: { type: 'string' },
      production: { type: 'boolean', default: false }, binary: { type: 'string' },
    } });
    if (positionals[0] === 'preflight') {
      preflight(values.config);
      console.log('发布前置检查通过');
    } else if (positionals[0] === 'build' || positionals[0] === 'test-build') {
      if (values.config) throw new Error('构建只能使用仓库已审核配置，测试密钥由 test-build 独立生成');
      build(values, positionals[0] === 'test-build');
    } else if (positionals[0] === 'test-manifest') {
      const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'ag-relay-test-manifest-'));
      try {
        const distributionDir = path.join(temp, 'license-distribution');
        const config = JSON.parse(fs.readFileSync(values.config, 'utf8'));
        prepareDistribution({ out: distributionDir, version: config.version, testOnly: true });
        stage(values.config, values['bundle-dir'], values.out, values.notes, true, { distributionDir });
      } finally {
        fs.rmSync(temp, { recursive: true, force: true });
      }
    } else if (positionals[0] === 'audit-paths') {
      auditPaths(values.binary);
    } else if (positionals[0] === 'verify') {
      verify(values.config, values.artifacts, values.production);
    } else throw new Error('用法：node scripts/release.mjs preflight|build|test-build|test-manifest|verify|audit-paths；构建/清单需 --out，验签需 --config 与 --artifacts');
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
