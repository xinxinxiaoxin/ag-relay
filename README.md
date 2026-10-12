# ag-relay

AG Relay 是由**网关核心**与 **Tauri 桌面面板**组成的本地网关软件。核心在本仓自主维护，与面板配套开发、测试和发布。

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

## 构建（一条命令打包 exe）

前置：Node.js + npm、Rust（cargo）、WebView2 运行时。

PowerShell：

```powershell
$env:CARGO_TARGET_DIR='D:\zcode项目\ag-relay-build\target'; npm install; npm run tauri build
```

说明：

- `CARGO_TARGET_DIR` 指向仓库外的共享增量缓存目录（跨工单复用，首次编译约 10–30 分钟，之后增量很快）。
- **并行构建风险**：多个人/多个 agent 同时用同一个共享 target 构建时，`target\release\ag-relay.exe` 会被最后构建的一方覆盖，且 cargo 排队等锁期间产物归属不可辨。并行场景请各自用隔离 target（见下），或在启动 exe 验收前先核对 mtime。
- 隔离 target 写法（本仓库验收工单采用，产物路径后缀随工单号变）：

  ```powershell
  $env:CARGO_TARGET_DIR='D:\zcode项目\ag-relay-build\target-t7'; npm install; npm run tauri build
  ```

- 产物：裸 exe 在 `%CARGO_TARGET_DIR%\release\ag-relay.exe`（旁边自动落 `node.exe` 与 `resources\gateway\`），安装包在同级 `bundle\`（msi + nsis）。
- Git Bash 等价写法：`CARGO_TARGET_DIR='D:/zcode项目/ag-relay-build/target' npx tauri build`。

## 网关核心如何维护与重打包

`src-tauri/resources/gateway/` 是 AG Relay 自主维护的核心源码，不是等待外部项目覆盖的缓存。按 [ADR-0004](docs/adr/0004-owned-gateway-core.md)，继续保留现有 Node 核心与桌面面板分工，不为产品整合重写已有能力。

1. **在本仓演进**：直接修改本仓核心，保留现有能力与测试；配置、控制协议或诊断事件变化时，同步修改桌面面板适配及契约测试。文件职责、测试入口与安全边界见[网关核心维护说明](docs/gateway-core-maintenance.md)。
2. **配套验证**：保留环境装配、诊断字典覆盖打包脚本事件码、诊断读取、状态与重启受理等契约；先跑维护说明中的隔离测试。旧全量测试及 `scripts/smoke.ps1` 可能触碰固定端口或真实系统状态，不能在现用网关所在环境直接执行。真实起停、重启与诊断展示验收需独立隔离环境，不能用 HTTP mock 通过代替。
3. **作为一个产品构建发布**：验证后按构建流程将核心与桌面面板一并打包，使用同一 AG Relay 版本发布；不提供独立追踪外部源码的「仅更新核心」。不得覆盖正在运行的网关文件。
4. **记录可追溯快照**：实际完成构建后，记录本仓构建提交、核心三个 `.mjs` 与 `gateway-config.bat` 的 SHA-256、成品哈希及构建时间。外部改进按需审查、择取补丁，记录来源提交并保留版权与许可要求，不再整体覆盖脚本；详见维护说明。

## 当前打包快照

以下保留已发布 **v0.2.0** 的构建记录与核心历史来源，不表示当前工作分支已经重打包；后续版本以本仓构建提交为基准记录快照。

| 项 | 值 |
| --- | --- |
| 打包日期 | 2026-10-12 07:22（UTC+8，**v0.2.0** 发布版重打包，构建自 `main`，React/shadcn UI 重构 spec #9 全量交付 + 审查修复） |
| 核心历史来源提交 | `28b764d`（xinxinxiaoxin/relay-retry-proxy，fix: Codex 跨资源错误最多尝试 5 次后直接报错；此后由本仓自主维护） |
| `relay-retry-proxy.mjs` sha256 | `e8395e0f90fe46bf1059bfcbac77e65d887d09d4ed5057490f85847e5cde85dc` |
| `relay-supervisor.mjs` sha256 | `f2f030b46999f84afc77f7f92643075c1d1f1ddf7ad3c327fbd92f5593dff77c` |
| `relay-start-check.mjs` sha256 | `bb54cc7255b4f570ba1bdcfda255f19f0ca0a70fa6d028d0ed4e44f8146037c4` |
| `gateway-config.bat` sha256 | `102865c82e8785fcf1630260f25db32e2b9f4ba3a990c768bfc9fce3deaa1faa`（环境变量真值，无密钥） |
| `ag-relay.exe` sha256 | `fb6db99cbef94629cd135e42f51115c34b9d8c88f658e62beffc235e2d1092c8`（**v0.2.0**，`target-t1\release\ag-relay.exe`；GitHub Release 资产 `ag-relay-v0.2.0-win64.zip`；该发布版核心与上述历史来源快照一致） |
| 本票验收证据 | `E:\codex项目\网关\ag-relay-notes\evidence\`（`t9-completion-report.md` 四闸门记录 + `t1`…`t8`/`review-fixes` 每页/每态渲染截图） |

## 冒烟自检

```powershell
npm run smoke
```

检查构建产物存在、启动后主窗口标题含「AG Relay」并能正常退出。

## 许可与代码来源

AG Relay 的自有代码（桌面面板、纳入本仓库自主维护的网关核心，以及维护者确认权属的导入设计代码）按 [MIT LICENSE](LICENSE) 授权，见 [ADR-0007](docs/adr/0007-mit-open-source-distribution.md)。允许使用、修改、再分发和商用，包括修改后闭源分发，但必须保留相应版权和许可声明。

第三方代码、图标、字体和随包二进制不因本项目采用 MIT 或使用 AI 辅助开发而改换许可证。完整说明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)，逐项版本、原文及来源见 [许可材料索引](licenses/COMPONENTS.md)。其中包括 Node 及其内含第三方的完整声明、Geist OFL、Lucide ISC / Feather MIT、React / Radix / shadcn / Tailwind、Rust 依赖和标准库，以及 WebView2Loader 的实际许可。

安装分发应保留安装目录的 `licenses/LICENSE`、`licenses/THIRD_PARTY_NOTICES.md` 与整个 `licenses/third-party/`，不能只复制主程序或只附一份 MIT 文本。提交打包前运行 `node scripts/check-licenses.mjs` 可离线检查材料缺失、原文删节及依赖/Node 二进制变化；它不是法律审核或真实安装验收。本说明不变更任何仓库的可见性，也不表示已通过公开核查或已发布新版。

## 目录结构

- `src/`：前端（原生 HTML/CSS/JS，无框架）
- `src-tauri/`：Tauri 2 壳（Rust）
- `src-tauri/resources/gateway/`：本仓自主维护的网关核心三脚本 + `gateway-config.bat` 默认配置契约（见[维护说明](docs/gateway-core-maintenance.md)）
- `scripts/make-icons.ps1`：重新生成应用图标（AG 字样，PNG + ICO）
- `scripts/smoke.ps1`：构建冒烟检查
