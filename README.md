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
| 打包日期 | 2026-10-11 19:59（UTC+8） |
| 上游仓库提交 | `28b764d`（xinxinxiaoxin/relay-retry-proxy，fix: Codex 跨资源错误最多尝试 5 次后直接报错） |
| `relay-retry-proxy.mjs` sha256 | `e8395e0f90fe46bf1059bfcbac77e65d887d09d4ed5057490f85847e5cde85dc` |
| `relay-supervisor.mjs` sha256 | `f2f030b46999f84afc77f7f92643075c1d1f1ddf7ad3c327fbd92f5593dff77c` |
| `relay-start-check.mjs` sha256 | `bb54cc7255b4f570ba1bdcfda255f19f0ca0a70fa6d028d0ed4e44f8146037c4` |
| `gateway-config.bat` sha256 | `102865c82e8785fcf1630260f25db32e2b9f4ba3a990c768bfc9fce3deaa1faa`（环境变量真值，无密钥） |
| `ag-relay.exe` sha256 | `d4e35a4bb68b17fc92343a99c9cb5abb340bb73261508602f17cd5fdfd962001`（隔离 target-t7 release 产物，9,314,304 字节） |
| 本票验收证据 | `D:\zcode项目\ag-relay-notes\t7-verification.md`（7 条验收逐项记录） |

## 冒烟自检

```powershell
npm run smoke
```

检查构建产物存在、启动后主窗口标题含「AG Relay」并能正常退出。

## 目录结构

- `src/`：前端（原生 HTML/CSS/JS，无框架）
- `src-tauri/`：Tauri 2 壳（Rust）
- `src-tauri/resources/gateway/`：网关三脚本 + `gateway-config.bat` 环境变量真值（来自上游，更新流程见上）
- `scripts/make-icons.ps1`：重新生成应用图标（AG 字样，PNG + ICO）
- `scripts/smoke.ps1`：构建冒烟检查
