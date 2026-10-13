# 发布一个版本的完整流程

面向维护者本人。目标：从「代码改好了」到「用户能在应用内收到更新」，全程不需要临场判断。
命令都在仓库根目录执行。相关设计见 [ADR-0005](adr/0005-whole-product-safe-updates.md)。

## 0. 先记住三件事

1. **签名私钥只有一把**，决定「用户能不能应用内更新」。丢了就再也推不了自动更新，只能让用户手动重装；泄露了别人就能伪造更新包推给你的用户。
   - 本项目当前私钥存放位置（维护者本机，仓库外）：`C:\Users\94376\.ag-relay-signing\ag-relay.key`（未设密码）
   - 公钥已经写在 [src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json) 的 `plugins.updater.pubkey`，公开无妨。
   - **绝不要**把私钥内容贴进聊天、Issue、提交或任何在线文档。
2. **一次发版只需要一次人工点击**：构建阶段自动跑，只有「是否把版本公开」需要人在 GitHub 上点一下。
3. **已有 tag / Release 一律不覆盖**。发错了就发下一个版本号（`0.3.3`），不要删了重发——用户可能已经装上了。

## 1. 前提（一次性，已经配好）

| 项 | 值 |
| --- | --- |
| 仓库 | `xinxinxiaoxin/ag-relay`（必须公开，否则更新端点无法匿名读取） |
| 环境 `release-signing` | 存 `TAURI_SIGNING_PRIVATE_KEY` secret（密钥无密码，故不需要 PASSWORD secret）；只允许从 `v*` tag 部署；**无人工审批** |
| 环境 `release-publish` | **有人工审批**（审批人：维护者）；只允许从 `v*` tag 部署 |
| 发布工作流 | [.github/workflows/release.yml](../.github/workflows/release.yml)，仅手动触发 |

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

### 第四步：本地预检（省一次 CI 白跑）

```bash
node scripts/release.mjs preflight
```

它会检查：生产公钥已配置、更新端点与降级策略正确、版本一致、安装器安全策略在位、随包资源完整、许可材料齐全（含安装器运行时）。**不通过就不要往下走。**

### 第五步：打标签并触发云端构建

```bash
git checkout main && git pull
git tag v0.3.3 && git push origin v0.3.3

gh workflow run release.yml --ref v0.3.3 -f publish=true -f notes="## AG Relay 0.3.3

- 这里写中文更新说明，会显示在应用的更新提示和 Release 页面上。"
```

- `notes` 是**用户会看到的更新说明**，写人话，别写提交摘要。
- `publish=false` 则只构建、产出 Actions 产物，不申请发布——用于先看看能不能编过。

### 第六步：点一次审批（唯一的人工步骤）

打开 Actions 里那次运行的页面 → 右上绿色 **Review deployments** → 勾 `release-publish` → **Approve and deploy**。

之后工作流会自动：完整上传资产到草稿 → 公开为正式 Release → 标记为 latest。**审批前线上不会有任何变化。**

### 第七步：确认 + 补打包快照

- 确认 `https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json` 的 `version` 已变成新版本。
- 在 README 的「打包快照」一节按现有格式追加一条（本仓构建提交、四个核心 `.mjs` 与 `bat` 的 sha256、成品 sha256、发布日期、Release 链接）。**没有实际构建就不要补记录，也不要改日期装成新产物。**

## 3. 备选：在维护者本机构建

需要本机持有私钥（即上面第 0 节的路径）。适合调试发布流程本身，或 CI 不可用时。

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.ag-relay-signing/ag-relay.key)"   # 注意不要回显
node scripts/release.mjs preflight
node scripts/release.mjs build --out <一个尚不存在的新目录> --notes <UTF-8 更新说明文件>
gh release create v0.3.3 --target main --title "AG Relay 0.3.3" --notes-file <说明文件> <目录里的全部资产>
```

`--out` 目录必须不存在，脚本拒绝往已有目录写。资产应包含两个安装包、各自 `.sig`、`latest.json`、`SHA256SUMS`。

## 4. 出问题怎么办

| 现象 | 处理 |
| --- | --- |
| 构建作业一步没跑就失败（waiting → failure） | 通常是环境部署被拒。检查 `release-signing` / `release-publish` 的 tag 策略是否仍为 `v*` |
| 报「产品版本不一致」 | 五处版本号没同步全，回到第二步 |
| 报「依赖或二进制已变化」 | 回到第三步：先证明依赖集合没变，再刷新指纹 |
| 报「缺少安装器运行时许可」/「正式分发许可来源未闭合」 | 许可材料或 `releaseBlockers` 的决定记录被动过，按 `THIRD_PARTY_NOTICES.md` 复核；不要删记录绕过 |
| 签名报密码错误 | 密钥与 `TAURI_SIGNING_PRIVATE_KEY` secret 不是同一把，或换了带密码的密钥却没配 PASSWORD secret |
| 发布作业报「release-publish 未配置必需审批」 | 环境的审批规则被取消了，去仓库 Settings → Environments 恢复 |
| 发布作业报「Release 已存在」 | 该项目已发布过。要改就发下一个版本号，不要覆盖 |

## 5. 发完之后

真机验收（干净机器安装、旧的 ZIP 版升级、在途请求不被掐断、篡改包被拒等）按
[真实 Windows 成品验收清单](windows-acceptance-checklist.md) 在独立沙箱/VM 中逐条执行。
本机自查最少做一遍：应用内检查更新能看到新版本 → 确认 → 升级后版本号变化、连接设置与日志保留。

## 6. 相关脚本

| 脚本 | 作用 |
| --- | --- |
| `scripts/release.mjs` | 预检、构建、生成清单、独立验签（`preflight` / `build` / `test-build` / `verify` / `audit-paths`） |
| `scripts/bump-version.mjs` | 一次同步上面五处版本号；先全部校验再落盘，中途失败不会留下半套版本号（测试：`scripts/bump-version.test.mjs`） |
| `scripts/check-licenses.mjs` | 离线核验许可材料与输入指纹；依赖或二进制变化即拒绝 |
| `scripts/smoke.ps1` | 只读冒烟：对已有发布物验签，不安装、不启动 |

改完脚本后请一并运行它们的测试：`node --test scripts/release.test.mjs scripts/check-licenses.test.mjs scripts/bump-version.test.mjs`。
