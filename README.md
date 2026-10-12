# ag-relay

AG Relay —— relay-retry-proxy 网关的 Tauri 桌面封装。

规范与工单见仓库 Issues（规范 #1，骨架工单 #2，网关工单 #3）。

## 日常使用

双击 `ag-relay.exe` 即可（自用裸目录产物：`ag-relay.exe` 旁边必须有 `node.exe` 和 `resources\gateway\`，整个 release 目录一起拷贝即可，无需安装、无需系统 Node——已实测隐藏系统 Node 的最小 PATH 环境下全链路正常）：

- **开**：双击启动后软件自动拉起本地网关（监听 `127.0.0.1:8787`）；若 8787 已有网关在跑（含面板外启动的实例），面板直接接管显示其状态，不重复拉起。
- **面板**：顶部四张状态卡（运行状态 / 版本 / 在途请求数 / 已运行时长）每 3 秒轮询一次；网关未运行时显示灰态，字段异常时显示「未知」。另有开关机自启的勾选项（当前用户注册表 Run 键，路径随 exe 位置自动改写）。
- **按钮**：`启动网关` / `停止网关`（优雅排空，退出码 0；本壳未受管网关但 8787 上有外部网关时，同样会优雅停止该外部网关——「关了就彻底关了」）/ `安全重启`（POST `/_relay/restart`，排空在途请求后由 supervisor 热重启，最长等 60 秒；网关刚启动约 2 秒内的重启请求可能被受理但不执行——等几秒再点即可）/ `打开日志文件夹`（资源管理器）/ `退出`。
- **托盘**：常驻系统托盘（可能在溢出区）。左键点击切换面板显隐；右键菜单提供与面板一致的启动/停止/重启/日志/退出。
- **关（退出语义）**：关窗口（× / Alt+F4）、托盘菜单「退出」、面板「退出」按钮三条路行为完全一致——退出软件前把网关优雅停掉，本壳受管的与面板外启动的外部网关一视同仁（外部网关经 status 的 pid 核实父进程确为 relay-supervisor.mjs 才动手，宁可不杀不可误杀；在途请求清零后退出，最多等 10 秒；超时强杀整个网关进程树，绝不留孤儿）。退出来源会记入壳侧日志（`window_close` / `tray_menu` / `panel_button` 三种白名单值），外部网关停止另记 `external_stop_graceful` / `external_stop_forced` / `external_stop_abandoned` 白名单事件。
- **日志位置**：`%APPDATA%\ag-relay\logs\`（网关诊断 `relay-diagnostics.jsonl` + 壳侧退出来源 `shell-exit.log`）。
- **开机自启**：注册的是软件本身（HKCU Run 键 `ag-relay` → 当前 exe），开机后由软件再拉起网关；exe 挪位置后下次启动自动改写自启项指向新路径。

## Windows 安装与签名构建

用户分发采用 **NSIS（优先）或 MSI**，两者都自带 `node.exe` 与 `resources/gateway/`，不依赖宿主机安装 Node。保留 `identifier=ag-relay` 和 MSI UpgradeCode，安装位置变化不改变 `%APPDATA%\ag-relay\` 下的设置与日志。旧 ZIP 版需先在安全停止网关后手动安装一次具备更新能力的版本；后续升级保持原安装器类型。

开发前置：Windows x64、Node/npm、Rust/MSVC、WebView2；`npm ci` 安装锁定依赖。构建工具可以下载 WiX、NSIS/WebView2 bootstrapper，**构建不等于运行安装器**。未配置生产公钥时，默认 `pubkey=""`、`endpoints=[]`，应用更新禁用；普通 Tauri 打包若缺少签名私钥会失败，不会降级为无签名发布。

```powershell
# 只编译面板和程序，不生成安装包、不启动应用
$env:CARGO_BUILD_JOBS='2'
npm run tauri -- build --no-bundle -- --locked -j 2

# 独立临时测试密钥构建：仅用于验签，不可发布、不执行安装
node scripts/release.mjs test-build --out C:\Temp\ag-relay-test-artifacts
```

可设置 `CARGO_TARGET_DIR` 复用隔离构建缓存；不要指向现用程序目录。共享 target 的构建必须串行排队，cargo 采用 `-j 2`，不要在另一个打包任务进行时重用其 `release/bundle`。测试构建在临时目录产生并销毁私钥，仅输出测试配置、公钥、签名产物及醒目标记。

### 正式发布门槛（目前未满足，不代表已发布）

1. 维护者在可信离线环境生成并备份正式 Tauri 签名密钥，只把**公钥内容**提交到 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。不要把私钥、密码或 `.env` 提交/随包分发。
2. 将 `endpoints` 设置为 `https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json`，保留 `requireSignedVersion=true`，不得打开不安全 TLS 或允许降级。公钥变更需设计旧版本信任迁移，不能随意换钥。
3. GitHub Environment `release-signing` 保存 `TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets（密码变量可为空）；`release-publish` 设置人工必需审批与允许的发布 tag。仓库及 Release 必须能被用户匿名读取；私有仓库的 404 不能伪装成“没有更新”，不向客户端嵌入 GitHub token。
4. 同步 `package.json`、`package-lock.json`、`Cargo.toml`、`tauri.conf.json` 版本并创建新的稳定 `vX.Y.Z` tag；补齐所有 `LICENSE`、`THIRD_PARTY_NOTICES.md`、`licenses/**`。发布前完成源码/历史/资产敏感信息复核。
5. 在独立 Windows Sandbox/VM 实测无系统 Node 安装启动、旧版升级、数据保留及安全排空更新。**本机尚未执行干净机器安装验收，不运行安装包，也未触碰现用网关。**

隔离验收使用独立 Windows 用户、AppData、安装目录和端口，不连接现用上游服务。至少验证：检查更新显示版本/说明但未确认不下载；独立测试签名新版升级；超过排空预算的合成长请求不断流、提示稍后再试且不安装；篡改字节验签失败且不停机；更新前运行/停止两种状态重开后分别恢复；设置和日志保留；UAC 拒绝、取消安装或旧版重开不得误报成功；窗口/托盘退出不能绕过更新事务的安全排空。此人工验收清单不由只读 smoke 自动完成。

手动触发 `.github/workflows/release.yml`：默认只构建并上传 Actions 产物，勾选发布后仍须通过 `release-publish` 审批才创建 GitHub Release；不覆盖已有 tag/Release。也可先本地执行 `node scripts/release.mjs preflight`，通过后运行 `node scripts/release.mjs build --out <新空目录> --notes <UTF-8更新说明文件>`。私钥仅通过当前进程环境/CI secrets 提供，脚本不读 `.env`、不回显密钥。

发布资产包括 NSIS `.exe`、MSI `.msi`、各自 `.sig`、UTF-8 可读 `latest.json` 与 `SHA256SUMS`。清单使用 `windows-x86_64-nsis` / `windows-x86_64-msi`，嵌入签名内容而非签名路径；应用按原安装器选择。Tauri updater 签名与 Windows Authenticode 是两件事：此流程未配置 Authenticode 证书，不能承诺“已验证发布者”或无 SmartScreen 警告。

## 网关上游更新后如何重打包

网关脚本以普通资源打进壳里（`src-tauri/resources/gateway/`），上游更新后重打包只需四步：

1. **复制新脚本与配置**：把上游仓库（`C:\Users\Administrator\Desktop\relay-retry-proxy\程序\`，GitHub 公开仓库 xinxinxiaoxin/relay-retry-proxy）里的四个文件

   - `relay-retry-proxy.mjs`（主网关）
   - `relay-supervisor.mjs`（守护进程）
   - `relay-start-check.mjs`（启动前端口检查）
   - `gateway-config.bat`（环境变量真值，测试逐项核对它；该文件只含默认值，**不含任何密钥**）

   覆盖复制到本仓库 `src-tauri/resources/gateway/`。`relay-control.mjs` 与其他 bat/ps1 **不要**复制（控制面壳已用自带 HTTP 调用替代）。

2. **跑测试**：`cargo test`（在 `src-tauri/` 下）。环境变量契约（17 项配置 + 诊断路径，真值即上一步复制的 `gateway-config.bat`）、supervisor 生命周期桩、重启 202 契约等测试必须全绿；上游若改了环境变量语义，这里会红。

3. **一条命令重打包**：按上一节设置 `CARGO_TARGET_DIR` 后 `npm run tauri build`（前端与 Rust 无改动时只是资源重嵌入，增量很快）。

4. **更新快照**：更新本文「当前打包快照」表——记录上游当前提交哈希（`git -C <上游仓库> log --oneline -1`）与三个脚本的 sha256（`sha256sum src-tauri/resources/gateway/*.mjs`），再补 exe 的 sha256 与构建时间。

## 当前打包快照

| 项 | 值 |
| --- | --- |
| 打包日期 | 2026-10-12 07:22（UTC+8，**v0.2.0** 发布版重打包，构建自 `main`，React/shadcn UI 重构 spec #9 全量交付 + 审查修复） |
| 上游仓库提交 | `28b764d`（xinxinxiaoxin/relay-retry-proxy，fix: Codex 跨资源错误最多尝试 5 次后直接报错） |
| `relay-retry-proxy.mjs` sha256 | `e8395e0f90fe46bf1059bfcbac77e65d887d09d4ed5057490f85847e5cde85dc` |
| `relay-supervisor.mjs` sha256 | `f2f030b46999f84afc77f7f92643075c1d1f1ddf7ad3c327fbd92f5593dff77c` |
| `relay-start-check.mjs` sha256 | `bb54cc7255b4f570ba1bdcfda255f19f0ca0a70fa6d028d0ed4e44f8146037c4` |
| `gateway-config.bat` sha256 | `102865c82e8785fcf1630260f25db32e2b9f4ba3a990c768bfc9fce3deaa1faa`（环境变量真值，无密钥） |
| `ag-relay.exe` sha256 | `fb6db99cbef94629cd135e42f51115c34b9d8c88f658e62beffc235e2d1092c8`（**v0.2.0**，`target-t1\release\ag-relay.exe`；GitHub Release 资产 `ag-relay-v0.2.0-win64.zip`；上游网关脚本零改动） |
| 本票验收证据 | `E:\codex项目\网关\ag-relay-notes\evidence\`（`t9-completion-report.md` 四闸门记录 + `t1`…`t8`/`review-fixes` 每页/每态渲染截图） |

## 冒烟自检

```powershell
npm run smoke -- -Artifacts C:\Temp\ag-relay-test-artifacts -Config C:\Temp\ag-relay-test-artifacts\tauri.test.json
```

只读检查 `latest.json` 与两种安装器签名，**不启动应用、不执行安装、不访问/停止网关**；不传产物目录直接拒绝。正式产物省略 `-Config`，使用仓库生产公钥。此检查不能代替独立虚拟机上的安装/升级验收。发布工具回归测试：`node --test --test-concurrency=1 scripts/release.test.mjs`（临时密钥自动销毁，Cargo `-j 2`）。

## 目录结构

- `src/`：前端（原生 HTML/CSS/JS，无框架）
- `src-tauri/`：Tauri 2 壳（Rust）
- `src-tauri/resources/gateway/`：网关三脚本 + `gateway-config.bat` 环境变量真值（来自上游，更新流程见上）
- `scripts/make-icons.ps1`：重新生成应用图标（AG 字样，PNG + ICO）
- `scripts/smoke.ps1`：构建冒烟检查
