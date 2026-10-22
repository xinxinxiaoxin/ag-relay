# AGENTS.md

AG Relay —— 本地重试网关的完整桌面应用，由**本仓自主维护的网关核心**（`src-tauri/resources/gateway/`）与 Tauri 桌面面板组成。产品自 v0.3.x 起已与 `relay-retry-proxy` 分离：核心在本仓开发、测试并随版本发布，`relay-retry-proxy` 只保留为历史来源，不是运行依赖或更新目标。核心职责与维护流程见 `docs/gateway-core-maintenance.md`。

## Agent 工作入口

### 诊断、开发与验证

排查报错、修改核心或面板、验收改动时，先按 `docs/gateway-core-maintenance.md` 的「快速工作流与定向验证」选择范围。修改重试或历史处理前，读 `docs/retry-boundaries.md` 顶部的现行边界导航。

### Issue tracker

Issues 与 specs 以 GitHub issue 形式存放于 `xinxinxiaoxin/ag-relay`，通过 `gh` CLI 操作。详见 `docs/agents/issue-tracker.md`。

### 本地打包与发布

用户要求提交、合并、推送、打包、上传或发布时，读 `docs/release-runbook.md` 对应章节。由 agent 按用户授权范围完成准备、执行与结果核对，不要求用户记命令或手工操作。较大改动需要适配时，优先维护既有发布入口；适配与授权规则见该文档。

### Domain docs

单上下文（single-context）：仓库根一个 `GLOSSARY.md` + `docs/adr/`。详见 `docs/agents/domain.md`。
