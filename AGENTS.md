# AGENTS.md

AG Relay —— 本地重试网关的完整桌面应用，由**本仓自主维护的网关核心**（`src-tauri/resources/gateway/`）与 Tauri 桌面面板组成。产品自 v0.3.x 起已与 `relay-retry-proxy` 分离：核心在本仓开发、测试并随版本发布，`relay-retry-proxy` 只保留为历史来源，不是运行依赖或更新目标。核心职责与维护流程见 `docs/gateway-core-maintenance.md`。

本文件存放面向 agent 技能的配置。

## Agent skills

### Issue tracker

Issues 与 specs 以 GitHub issue 形式存放于 `xinxinxiaoxin/ag-relay`，通过 `gh` CLI 操作。详见 `docs/agents/issue-tracker.md`。

### Domain docs

单上下文（single-context）：仓库根一个 `GLOSSARY.md` + `docs/adr/`。详见 `docs/agents/domain.md`。
