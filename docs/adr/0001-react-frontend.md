# 采用 React/Vite/shadcn 作为壳的前端,替换原生 HTML/CSS/JS

为落地已定稿的 shadcn/ui 设计(`ui-design-lab` 中一个可运行的 React 19 + Vite + Tailwind v4 + shadcn/ui 工程),将 Tauri 壳的前端由原生 HTML/CSS/JS 换成直接复用该设计工程,再把其假数据状态机逐个接到现有 Tauri 命令。另一条路是用原生 HTML/CSS/JS 手工复刻整套 shadcn 组件 / 命令面板 / 吐司——保真度与工作量都不划算。换栈只影响 webview 内的静态前端资源,自带 node.exe 与网关脚本不变。

## Consequences

- 构建从「`frontendDist` 指向原生 `src`」改为 Vite 产物;新增 Node / Vite 前端依赖与构建步骤。
- 产物体积仅多出 webview 的静态 JS / CSS;网关运行时与打包链(node sidecar + `resources/gateway`)不受影响。
