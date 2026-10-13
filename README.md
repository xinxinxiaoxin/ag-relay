# ag-relay

AG Relay 是由**网关核心**与 **Tauri 桌面面板**组成的本地网关软件。核心在本仓自主维护，与面板配套开发、测试和发布。

规范与工单见仓库 [Issues](https://github.com/xinxinxiaoxin/ag-relay/issues)。面向 agent 的 issue 操作约定见 [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md)，领域术语见 [GLOSSARY.md](GLOSSARY.md)。

## 安装

到 [Releases](https://github.com/xinxinxiaoxin/ag-relay/releases) 下载最新版的 Windows 安装程序（`ag-relay_x.y.z_x64-setup.exe`，或 `.msi`），双击安装即可。安装包自带运行所需的 Node，**不要求机器上预装 Node**；安装位置与数据位置分离，`%APPDATA%\ag-relay\` 下的连接设置与日志在安装和升级中都保留。

装好之后不需要再手动找新版本：软件会检查更新，发现新版时显示更新说明，**由你确认后**才下载安装；安装前会先排空在途请求，若一次请求还没结束就等到超时并提示稍后再试，不会强行掐断。更新包经过签名校验，被篡改的包会被拒绝。

从旧的 ZIP 版本迁移：ZIP 版没有更新能力，需要**先手动安装一次本版本**，之后才能在应用内升级。

## 便携使用（ZIP / 裸目录）

双击 `ag-relay.exe` 即可（自用裸目录产物：`ag-relay.exe` 旁边必须有 `node.exe` 和 `resources\gateway\`，整个 release 目录一起拷贝即可，无需安装、无需系统 Node——已实测隐藏系统 Node 的最小 PATH 环境下全链路正常）：

- **开**：双击启动后软件自动拉起本地网关（监听 `127.0.0.1:8787`）；若 8787 已有网关在跑（含面板外启动的实例），面板直接接管显示其状态，不重复拉起。
- **面板**：顶部四张状态卡（运行状态 / 版本 / 在途请求数 / 已运行时长）每 3 秒轮询一次；网关未运行时显示灰态，字段异常时显示「未知」。另有开关机自启的勾选项（当前用户注册表 Run 键，路径随 exe 位置自动改写）。
- **按钮**：`启动网关` / `停止网关`（优雅排空，退出码 0；本壳未受管网关但 8787 上有外部网关时，同样会优雅停止该外部网关——「关了就彻底关了」）/ `安全重启`（POST `/_relay/restart`，排空在途请求后由 supervisor 热重启，最长等 60 秒；网关刚启动约 2 秒内的重启请求可能被受理但不执行——等几秒再点即可）/ `打开日志文件夹`（资源管理器）/ `退出`。
- **托盘**：常驻系统托盘（可能在溢出区）。左键点击切换面板显隐；右键菜单提供与面板一致的启动/停止/重启/日志/退出。
- **关（退出语义）**：关窗口（× / Alt+F4）、托盘菜单「退出」、面板「退出」按钮三条路行为完全一致——退出软件前把网关优雅停掉，本壳受管的与面板外启动的外部网关一视同仁（外部网关经 status 的 pid 核实父进程确为 relay-supervisor.mjs 才动手，宁可不杀不可误杀；在途请求清零后退出，最多等 10 秒；超时强杀整个网关进程树，绝不留孤儿）。退出来源会记入壳侧日志（`window_close` / `tray_menu` / `panel_button` 三种白名单值），外部网关停止另记 `external_stop_graceful` / `external_stop_forced` / `external_stop_abandoned` 白名单事件。
- **日志位置与容量**：`%APPDATA%\ag-relay\logs\`。网关诊断 `relay-diagnostics.jsonl` 沿用现有保护：默认每份 1 MiB、保留 `.1` 一份旧档（合计约 2 MiB）；壳侧退出来源 `shell-exit.log` 每份最多 512 KiB、保留 `.1` 一份（合计最多 1 MiB）。到限先淘汰最旧档，退出不会清空；旧版超大壳日志会在下次写入时只保留预算内的最新完整行。
  - 普通文件日志默认关闭；显式指定 `PROXY_LOG_FILE` 时写到该路径，默认当前文件 + `.1` **合计最多 2 MiB**，可用 `PROXY_LOG_MAX_BYTES` 设置两份合计预算（每份占一半，非法值回退默认）。单条记录若超过单档预算则不写文件，不截断文字。
  - 默认只按容量保留。普通文件日志可设置 `PROXY_LOG_RETENTION_DAYS` 为正数启用期限；按每份文件的最后修改时间、在下一次日志写入时清理过期档（不是逐条记录的精确期限，也不在退出时或后台定时清空）。壳侧日志模块支持可选期限，但产品默认不启用。以上不改变网关诊断的容量与脱敏规则；普通文件日志仍可能包含开发排障自由文本，不应当作可公开分享的脱敏诊断记录。
- **开机自启**：注册的是软件本身（HKCU Run 键 `ag-relay` → 当前 exe），开机后由软件再拉起网关；exe 挪位置后下次启动自动改写自启项指向新路径。

## Windows 安装与签名构建

用户分发采用 **NSIS（优先）或 MSI**，两者都自带 `node.exe` 与 `resources/gateway/`，不依赖宿主机安装 Node。保留 `identifier=ag-relay` 和 MSI UpgradeCode，安装位置变化不改变 `%APPDATA%\ag-relay\` 下的设置与日志。旧 ZIP 版需先在安全停止网关后手动安装一次具备更新能力的版本；后续升级保持原安装器类型。

开发前置：Windows x64、Node/npm、Rust/MSVC、WebView2；`npm ci` 安装锁定依赖。构建工具可能准备 WiX、NSIS；安装时若缺少 WebView2，将联网下载其 bootstrapper。**构建不等于运行安装器**。生产更新公钥与端点现已配置（见 [tauri.conf.json](src-tauri/tauri.conf.json) 的 `plugins.updater`）；若某次构建未配置生产公钥，则为 `pubkey=""`、`endpoints=[]`，应用更新禁用。普通 Tauri 打包若缺少签名私钥会失败，不会降级为无签名发布。

```powershell
# 只编译面板和程序，不生成安装包、不启动应用
$env:CARGO_BUILD_JOBS='2'
npm run tauri -- build --no-bundle -- --locked -j 2

# 独立临时测试密钥构建：仅用于验签，不可发布、不执行安装
node scripts/release.mjs test-build --out C:\Temp\ag-relay-test-artifacts
```

可设置 `CARGO_TARGET_DIR` 复用隔离构建缓存；不要指向现用程序目录。共享 target 的构建必须串行排队，cargo 采用 `-j 2`，不要在另一个打包任务进行时重用其 `release/bundle`。测试构建在独立 TEMP/TMP 目录产生并销毁私钥，仅输出测试配置、公钥、签名产物及醒目标记。构建子进程通过 Rust `--remap-path-prefix` 把工作树、用户 home、Cargo/Rustup 与 target 路径映射为固定无身份前缀，不改变系统环境；生成清单前扫描未压缩主程序的 UTF-8/UTF-16 本机路径残留，发现即拒绝。该检查不能代替完整资产核查。

### 正式发布门槛

当前状态（2026-10-12）：生产签名密钥已生成并保存在仓库之外，公钥与更新端点已提交，`v0.3.0`/`v0.3.1` 已按本文流程发布，仓库已公开，安装、启动、网关托管、连接设置持久化与实时活动刷新已在本机实测通过。**仍待完成的是独立 Windows Sandbox/VM 中的干净机器安装与升级验收**，以及可选的 Authenticode 代码签名（见本小节末）。

发布前逐项确认：

1. 维护者在可信离线环境生成并备份正式 Tauri 签名密钥，只把**公钥内容**提交到 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。不要把私钥、密码或 `.env` 提交/随包分发。私钥是后续所有更新的信任根：**丢失后无法再通过应用内更新升级，只能让用户手动重装**，务必备份到仓库之外的安全位置。
2. 将 `endpoints` 设置为 `https://github.com/xinxinxiaoxin/ag-relay/releases/latest/download/latest.json`，保留 `requireSignedVersion=true`，不得打开不安全 TLS 或允许降级。公钥变更需设计旧版本信任迁移，不能随意换钥。
3. GitHub Environment `release-signing` 保存 `TAURI_SIGNING_PRIVATE_KEY` secret（本项目的密钥未设密码，因此不需要 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；若密钥带密码则须一并配置），仓库及 Release 必须能被用户匿名读取——私有仓库的 404 不能伪装成“没有更新”，也不向客户端嵌入 GitHub token。**人工审批只保留在发布前的 `release-publish` 环境**：签名构建直接跑，是否把版本公开给人点一下决定（两个环境都只允许从 `v*` 稳定版本 tag 部署）。
4. 同步 `package.json`、`package-lock.json`、`Cargo.toml`、`tauri.conf.json` 版本并创建新的稳定 `vX.Y.Z` tag；补齐所有 `LICENSE`、`THIRD_PARTY_NOTICES.md`、`licenses/**`（含实际 NSIS/WiX 及插件运行时材料）；正式预检强制按实际 Rust 版本及 `--installer-runtime` 核对许可。发布前完成源码/历史/资产敏感信息复核，结论记在[公开前敏感信息复核报告](docs/security/spec19-prepublication-audit.md)。
5. 在独立 Windows Sandbox/VM 实测无系统 Node 安装启动、旧版升级、数据保留及安全排空更新。本机已实测「安装 → 启动 → 网关托管 → 设置持久化 → 实时活动刷新」，但**干净机器与完整升级链路仍须在隔离环境中逐条确认**。逐条步骤、判定标准与证据留存见[真实 Windows 成品验收清单](docs/windows-acceptance-checklist.md)。

隔离验收使用独立 Windows 用户、AppData、安装目录和端口，不连接现用上游服务。至少验证：检查更新显示版本/说明但未确认不下载；独立测试签名新版升级；超过排空预算的合成长请求不断流、提示稍后再试且不安装；篡改字节验签失败且不停机；更新前运行/停止两种状态重开后分别恢复；设置和日志保留；UAC 拒绝、取消安装或旧版重开不得误报成功；窗口/托盘退出不能绕过更新事务的安全排空。上述每条的可执行步骤、判定标准与证据位置见[真实 Windows 成品验收清单](docs/windows-acceptance-checklist.md)；此人工验收不由只读 smoke 自动完成。

从新的稳定版本 tag 手动触发 `.github/workflows/release.yml`：默认只构建并上传 Actions 产物，勾选发布后仍须通过 `release-publish` 审批，先完整上传到草稿再公开 GitHub Release；不覆盖已有 tag/Release。也可先本地执行 `node scripts/release.mjs preflight`，通过后运行 `node scripts/release.mjs build --out <新空目录> --notes <UTF-8更新说明文件>`。私钥仅通过当前进程环境/CI secrets 提供，脚本不读 `.env`、不回显密钥。

发布资产包括 NSIS `.exe`、MSI `.msi`、各自 `.sig`、UTF-8 可读 `latest.json` 与 `SHA256SUMS`。清单使用 `windows-x86_64-nsis` / `windows-x86_64-msi`，嵌入签名内容而非签名路径；应用按原安装器选择。Tauri updater 签名与 Windows Authenticode 是两件事：此流程未配置 Authenticode 证书，不能承诺“已验证发布者”或无 SmartScreen 警告。

## 网关核心如何维护与重打包

`src-tauri/resources/gateway/` 是 AG Relay 自主维护的核心源码，不是等待外部项目覆盖的缓存。按 [ADR-0004](docs/adr/0004-owned-gateway-core.md)，继续保留现有 Node 核心与桌面面板分工，不为产品整合重写已有能力。

1. **在本仓演进**：直接修改本仓核心，保留现有能力与测试；配置、控制协议或诊断事件变化时，同步修改桌面面板适配及契约测试。文件职责、测试入口与安全边界见[网关核心维护说明](docs/gateway-core-maintenance.md)。
2. **配套验证**：保留环境装配、诊断字典覆盖打包脚本事件码、诊断读取、状态与重启受理等契约；先跑维护说明中的隔离测试。旧全量测试及旧版 `scripts/smoke.ps1` 可能触碰固定端口或真实系统状态，不能在现用网关所在环境直接执行。真实起停、重启与诊断展示验收需独立隔离环境，不能用 HTTP mock 通过代替。
3. **作为一个产品构建发布**：验证后按构建流程将核心与桌面面板一并打包，使用同一 AG Relay 版本发布；不提供独立追踪外部源码的「仅更新核心」。不得覆盖正在运行的网关文件。
4. **记录可追溯快照**：实际完成构建后，记录本仓构建提交、核心四个 `.mjs`（`relay-retry-proxy.mjs`、`relay-supervisor.mjs`、`relay-start-check.mjs`、`bounded-file-log.mjs`）与 `gateway-config.bat` 的 SHA-256、成品哈希及构建时间。四个 `.mjs` 与配置文件一样逐份纳入发布快照，任一改动后重打包都刷新对应哈希。外部改进按需审查、择取补丁，记录来源提交并保留版权与许可要求，不再整体覆盖脚本；详见维护说明。

## 打包快照

以下按时间倒序记录每次实际构建的快照，用于追溯「哪个提交、哪些核心脚本、产出哪个成品」。**没有实际重打包就不新增记录，也不更新日期来伪装新产物。**

### v0.3.1（当前）

| 项 | 值 |
| --- | --- |
| 打包日期 | 2026-10-12 22:38（UTC+8） |
| 本仓构建提交 | `92a8104`（`main`，spec #19 全量交付，含安装器、应用内更新与日志有界） |
| `relay-retry-proxy.mjs` sha256 | `9f2c196d2a2077fda192e41bb903222f3f8d87bb8d5c3b571f6159db01a299b8` |
| `relay-supervisor.mjs` sha256 | `f2f030b46999f84afc77f7f92643075c1d1f1ddf7ad3c327fbd92f5593dff77c` |
| `relay-start-check.mjs` sha256 | `bb54cc7255b4f570ba1bdcfda255f19f0ca0a70fa6d028d0ed4e44f8146037c4` |
| `bounded-file-log.mjs` sha256 | `3841b877ce76e6ff37497bfbcd65e3dd4f478442dc7fff9eaf4b39612b25c205` |
| `gateway-config.bat` sha256 | `102865c82e8785fcf1630260f25db32e2b9f4ba3a990c768bfc9fce3deaa1faa`（环境变量真值，无密钥） |
| NSIS `ag-relay_0.3.1_x64-setup.exe` sha256 | `01b2c42a4e6cca05ab2c6946e49aca9319fd248e1b1e80a11db3716102cdff2d` |
| MSI `ag-relay_0.3.1_x64_zh-CN.msi` sha256 | `748a7e96da3a239dafa9a62ed5d231b4fb05c1560c6be6952d6486eeeb1b4aea` |
| Release | [v0.3.1](https://github.com/xinxinxiaoxin/ag-relay/releases/tag/v0.3.1)（NSIS + MSI + 各自 `.sig` + `latest.json` + `SHA256SUMS`） |

### v0.2.0（历史）

v0.2.0 早于 `bounded-file-log.mjs` 引入，此表按当时口径保留，不回填新脚本哈希。

| 项 | 值 |
| --- | --- |
| 打包日期 | 2026-10-12 07:22（UTC+8，React/shadcn UI 重构 spec #9 全量交付 + 审查修复） |
| 核心历史来源提交 | `28b764d`（xinxinxiaoxin/relay-retry-proxy；此后由本仓自主维护） |
| `relay-retry-proxy.mjs` sha256 | `e8395e0f90fe46bf1059bfcbac77e65d887d09d4ed5057490f85847e5cde85dc` |
| `relay-supervisor.mjs` sha256 | `f2f030b46999f84afc77f7f92643075c1d1f1ddf7ad3c327fbd92f5593dff77c` |
| `relay-start-check.mjs` sha256 | `bb54cc7255b4f570ba1bdcfda255f19f0ca0a70fa6d028d0ed4e44f8146037c4` |
| `gateway-config.bat` sha256 | `102865c82e8785fcf1630260f25db32e2b9f4ba3a990c768bfc9fce3deaa1faa` |
| `ag-relay.exe` sha256 | `fb6db99cbef94629cd135e42f51115c34b9d8c88f658e62beffc235e2d1092c8` |
| Release | [v0.2.0](https://github.com/xinxinxiaoxin/ag-relay/releases/tag/v0.2.0)（资产 `ag-relay-v0.2.0-win64.zip`，ZIP 分发，无应用内更新） |

验收证据与复核结论（不提交进仓库）：规格 #19 的敏感信息复核见[公开前复核报告](docs/security/spec19-prepublication-audit.md)，真机验收步骤与判定标准见[真实 Windows 成品验收清单](docs/windows-acceptance-checklist.md)。

## 冒烟自检

```powershell
npm run smoke -- -Artifacts C:\Temp\ag-relay-test-artifacts -Config C:\Temp\ag-relay-test-artifacts\tauri.test.json
```

只读检查 `latest.json` 与两种安装器签名，**不启动应用、不执行安装、不访问/停止网关**；不传产物目录直接拒绝。正式产物省略 `-Config`，使用仓库生产公钥。此检查不能代替独立虚拟机上的安装/升级验收（见[真实 Windows 成品验收清单](docs/windows-acceptance-checklist.md)）。发布工具回归测试：`node --test --test-concurrency=1 scripts/release.test.mjs`（临时密钥自动销毁，Cargo `-j 2`）。

## 许可与代码来源

AG Relay 的自有代码（桌面面板、纳入本仓库自主维护的网关核心，以及维护者确认权属的导入设计代码）按 [MIT LICENSE](LICENSE) 授权，见 [ADR-0007](docs/adr/0007-mit-open-source-distribution.md)。允许使用、修改、再分发和商用，包括修改后闭源分发，但必须保留相应版权和许可声明。

第三方代码、图标、字体和随包二进制不因本项目采用 MIT 或使用 AI 辅助开发而改换许可证。完整说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)，逐项版本、原文及来源见 [许可材料索引](licenses/COMPONENTS.md)。其中包括 Node 及其内含第三方的完整声明、Geist OFL、Lucide ISC / Feather MIT、React / Radix / shadcn / Tailwind、Rust 依赖和标准库，以及 WebView2Loader 的实际许可。

安装分发应保留安装目录的 `licenses/LICENSE`、`licenses/THIRD_PARTY_NOTICES.md` 与整个 `licenses/third-party/`，不能只复制主程序或只附一份 MIT 文本。提交打包前运行 `node scripts/check-licenses.mjs` 可离线检查材料缺失、原文删节及依赖/Node 二进制变化；它不是法律审核或真实安装验收。

本 README 的许可与可见性状态（2026-10-13）：`xinxinxiaoxin/ag-relay` **已公开**并已发布 `v0.3.0` / `v0.3.1`；`xinxinxiaoxin/relay-retry-proxy` 仍保持私有。公开前的敏感信息复核结论见[公开前复核报告](docs/security/spec19-prepublication-audit.md)。

## 目录结构

- `src/`：**桌面面板**前端（React 19 + Vite + Tailwind v4 + shadcn/ui，见 [ADR-0001](docs/adr/0001-react-frontend.md)）
- `src-tauri/`：Tauri 2 桌面面板（Rust）
- `src-tauri/resources/gateway/`：本仓自主维护的网关核心四脚本（`relay-retry-proxy.mjs`、`relay-supervisor.mjs`、`relay-start-check.mjs`、`bounded-file-log.mjs`）+ `gateway-config.bat` 默认配置契约（见[维护说明](docs/gateway-core-maintenance.md)）
- `scripts/make-icons.ps1`：重新生成应用图标（AG 字样，PNG + ICO）
- `scripts/smoke.ps1`：构建冒烟检查
