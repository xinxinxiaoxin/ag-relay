# ag-relay

AG Relay —— relay-retry-proxy 网关的 Tauri 桌面封装。

规范与工单见仓库 Issues（规范 #1，骨架工单 #2，网关工单 #3）。

## 日常使用

双击 `ag-relay.exe`（自用裸目录产物：`ag-relay.exe` 旁边必须有 `node.exe` 和 `resources\gateway\`，整个 release 目录一起拷贝即可，无需安装、无需系统 Node）：

- **开**：点「启动网关」，本地 `127.0.0.1:8787` 网关即拉起（端口已被占用时会明确提示，不重复拉起）。
- **关**：直接关闭窗口即可——退出时网关自动优雅排空（在途请求清零后退出），最多等 10 秒；超时则强制结束整个网关进程树。也可以点「停止网关」手动停。
- 诊断日志写在 `%APPDATA%\ag-relay\logs\relay-diagnostics.jsonl`。

## 构建（一条命令打包 exe）

前置：Node.js + npm、Rust（cargo）、WebView2 运行时。

PowerShell：

```powershell
$env:CARGO_TARGET_DIR='D:\zcode项目\ag-relay-build\target'; npm install; npm run tauri build
```

说明：

- `CARGO_TARGET_DIR` 指向仓库外的共享增量缓存目录（跨工单复用，首次编译约 10–30 分钟，之后增量很快）。
- 产物：裸 exe 在 `%CARGO_TARGET_DIR%\release\ag-relay.exe`（即 `D:\zcode项目\ag-relay-build\target\release\ag-relay.exe`），安装包在同级 `bundle\` 下。
- Git Bash 等价写法：`CARGO_TARGET_DIR='D:/zcode项目/ag-relay-build/target' npx tauri build`。

## 冒烟自检

```powershell
npm run smoke
```

检查构建产物存在、启动后主窗口标题含「AG Relay」并能正常退出。

## 目录结构

- `src/`：前端（原生 HTML/CSS/JS，无框架）
- `src-tauri/`：Tauri 2 壳（Rust）
- `scripts/make-icons.ps1`：重新生成应用图标（AG 字样，PNG + ICO）
- `scripts/smoke.ps1`：构建冒烟检查
