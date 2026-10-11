# ag-relay

AG Relay —— relay-retry-proxy 网关的 Tauri 桌面封装。

规范与工单见仓库 Issues（规范 #1，骨架工单 #2）。

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
