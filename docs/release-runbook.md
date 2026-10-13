# 发布一个版本的完整流程

面向维护者本人。目标：从「代码改好了」到「用户能在应用内收到更新」，全程不需要临场判断。
命令都在仓库根目录执行。相关设计见 [ADR-0005](adr/0005-whole-product-safe-updates.md)。

## 0. 先记住三件事

1. **签名私钥只有一把**，决定「用户能不能应用内更新」。丢了就再也推不了自动更新，只能让用户手动重装；泄露了别人就能伪造更新包推给你的用户。
   - 本项目私钥存放在**维护者本机、仓库之外的专用路径**（未设密码）。具体位置以维护者自己的密钥备份记录为准，**不写进本仓库、本手册或任何在线文档**。
   - 公钥已经写在 [src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json) 的 `plugins.updater.pubkey`，公开无妨。
   - **绝不要**把私钥内容贴进聊天、Issue、提交或任何在线文档。
2. **构建在维护者本机跑**：用私钥签名，不依赖云端流水线，也因此没有配额、没有排队，构建期间不用守着。
3. **已有 tag / Release 一律不覆盖**。发错了就发下一个版本号，不要删了重发——用户可能已经装上了。

## 1. 前提（一次性，已经配好）

| 项 | 值 |
| --- | --- |
| 仓库 | `xinxinxiaoxin/ag-relay`（必须公开，否则更新端点无法匿名读取） |
| 签名私钥 | 维护者本机、仓库之外的专用路径（未设密码）；**只在本机构建时用**，不经云端 |
| 云端流水线 | 已停用。仓库不再有 GitHub Actions 发布工作流，也不需要任何仓库/环境 secret |

密钥最早是在维护者本机用 `npm run tauri -- signer generate -w <路径> --ci` 生成的。**换密钥等于换信任根**：旧版本内置的是旧公钥，认不了新私钥签的包，必须让用户手动装一次新版本才能继续自动更新。

## 2. 正常发版（推荐路径）

### 第一步：改代码、提交、合并到 main

```bash
git checkout main && git pull
# ... 改代码 ...
git push origin main
```

### 第二步：同步版本号（**五处，缺一处构建就会拒绝**）

```bash
node scripts/bump-version.mjs 0.3.3   # 工具脚本见下方「本地辅助脚本」
```

要一致的文件：`package.json`、`package-lock.json`（根 `version` 与 `packages[""].version`）、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`（`ag-relay` 条目）、`src-tauri/tauri.conf.json`。
`checkBuildInputs` 会逐个比对，任何一处落后都会以「产品版本不一致」拒绝打包。

### 第三步：重新核验许可清单（**版本号一改就会触发这一步**）

许可校验对四个清单文件做指纹校验，所以改完版本号它必定报「依赖或二进制已变化」。**这不是要你改指纹蒙混过关**，而是要求先证明依赖集合真的没变：

```bash
# 1) 对比 cargo 解析树与 npm 锁定树，确认与清单记录逐项一致
cargo metadata --locked --filter-platform x86_64-pc-windows-msvc --format-version 1 --manifest-path src-tauri/Cargo.toml
# 2) 确认无差异后刷新四个输入文件的指纹；有任何差异都必须按 THIRD_PARTY_NOTICES.md 重新取证
# 3) 复核
node scripts/check-licenses.mjs --root . --rust-version <实测 rustc 版本> --installer-runtime
```

若顺手加了新依赖，还要补它的许可原文与 `licenses/inventory.json` 记录，不能只刷新指纹。

### 第四步：本地预检

```bash
node scripts/release.mjs preflight
```

它会检查：生产公钥已配置、更新端点与降级策略正确、版本一致、安装器安全策略在位、随包资源完整、许可材料齐全（含安装器运行时）。**不通过就不要往下走。**

### 第五步：维护者本机构建（用私钥签名）

私钥只在本机、仓库之外；构建进程通过环境变量拿到它，不写进任何文件、不回显、不上传云端。

```bash
# 未设密码的密钥；不要 `cat` 出来，也不要贴进聊天
TAURI_SIGNING_PRIVATE_KEY="$(cat <维护者本机私钥路径>)" \
  node scripts/release.mjs build --out <一个尚不存在的新目录> --notes <UTF-8 更新说明文件>
```

- 构建会自动生成安装用许可资源与同版 `third-party-sources.tar.gz`（`build.rs` 与发布脚本都会准备，失败即拒绝打包）。
- 加了 LTO / `opt-level="z"` 的瘦身配置后，**首次编译约 10 分钟、有 target 缓存后约 5-6 分钟**；构建期间不用守着。
- 输出目录里应有：NSIS `.exe`、MSI `.msi`、各自 `.sig`、`latest.json`、`third-party-sources.tar.gz`、`SHA256SUMS`。

### 第六步：提交推送 + 打 tag

```bash
git checkout main && git pull
git push origin main
git tag v0.3.4 && git push origin v0.3.4
```

tag 只是留痕与今后对照用的版本锚点，**不再触发任何云端构建**。

### 第七步：上传 Release 并公开

```bash
gh release create v0.3.4 --target main --title "AG Relay 0.3.4" \
  --notes-file <说明文件> <构建输出目录里的全部资产>
```

- 说明文件就是用户会看到的更新说明，写人话，别写提交摘要。
- **已有 tag / Release 一律不覆盖**：发错了就发下一个版本号。
- 公开后约一两分钟，用户端应用检查更新即可看到新版本。

### 第八步：确认 + 补打包快照

- 确认 `https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json` 的 `version` 已变成新版本。
- 在 README 的「打包快照」一节按现有格式追加一条（本仓构建提交、四个核心 `.mjs` 与 `bat` 的 sha256、成品 sha256、发布日期、Release 链接）。**没有实际构建就不要补记录，也不要改日期装成新产物。**

## 3. 出问题怎么办

| 现象 | 处理 |
| --- | --- |
| 报「产品版本不一致」 | 五处版本号没同步全，回到第二步 |
| 报「依赖或二进制已变化」 | 回到第三步：先证明依赖集合没变，再刷新指纹 |
| 报「缺少安装器运行时许可」/「正式分发许可来源未闭合」 | 许可材料或 `releaseBlockers` 的决定记录被动过，按 `THIRD_PARTY_NOTICES.md` 复核；不要删记录绕过 |
| 报「必须通过环境提供 TAURI_SIGNING_PRIVATE_KEY」 | 私钥没提供给构建进程，回到第五步；密钥本身不设密码 |
| 报「发布目录缺少文件或混入额外文件」/「发布文件校验和不匹配」 | 资产不齐或被改动，重新构建，不要手工增删文件 |
| 报「Release 已存在」 | 该项目已发布过。要改就发下一个版本号，不要覆盖 |
| 构建很慢（十分钟以上） | 瘦身用的 LTO 与单 codegen unit 会明显拉长编译；确认在复用同一 `CARGO_TARGET_DIR` 缓存 |

## 4. 发完之后

真机验收（干净机器安装、旧的 ZIP 版升级、在途请求不被掐断、篡改包被拒等）按
[真实 Windows 成品验收清单](windows-acceptance-checklist.md) 在独立沙箱/VM 中逐条执行。
本机自查最少做一遍：应用内检查更新能看到新版本 → 确认 → 升级后版本号变化、连接设置与日志保留。

## 5. 相关脚本

| 脚本 | 作用 |
| --- | --- |
| `scripts/release.mjs` | 预检、构建、生成清单、独立验签（`preflight` / `build` / `test-build` / `verify` / `audit-paths`） |
| `scripts/prepare-distribution.mjs` | 离线重建安装用许可资源与同版第三方源码附件；Tauri 打包前与 `build.rs` 自动执行，也可单独运行排查 |
| `scripts/bump-version.mjs` | 一次同步上面五处版本号；先全部校验再落盘，中途失败不会留下半套版本号（测试：`scripts/bump-version.test.mjs`） |
| `scripts/check-licenses.mjs` | 离线核验许可材料与输入指纹；依赖或二进制变化即拒绝 |
| `scripts/smoke.ps1` | 只读冒烟：对已有发布物验签，不安装、不启动 |

改完脚本后请一并运行它们的测试：`node --test scripts/release.test.mjs scripts/check-licenses.test.mjs scripts/bump-version.test.mjs scripts/license-distribution.test.mjs`。
