# 工单与规格：GitHub

本仓工单与规格以 `xinxinxiaoxin/ag-relay` 的 GitHub issue 为真值，使用 `gh` 操作。在克隆内 `gh` 自动识别仓库；需要确认时查看 `git remote -v`。

## 基础操作

- 创建：`gh issue create --title "..." --body "..."`；多行正文可用 heredoc。
- 读取：`gh issue view <number> --json number,title,body,labels,comments`。
- 列表：`gh issue list --state open --json number,title,body,labels,comments`；按需加 `--label` 或调整 `--state`。
- 评论：`gh issue comment <number> --body "..."`。
- 标签：`gh issue edit <number> --add-label "..."` / `--remove-label "..."`。
- 认领：`gh issue edit <number> --add-assignee @me`。
- 关闭：`gh issue close <number> --comment "..."`。

## 子工单与依赖

- `gh` 2.94+：创建时用 `gh issue create --parent <parent> ...`，或之后用 `gh issue edit <parent> --add-sub-issue <child>` 关联。
- 旧版 `gh`：`gh api --method POST repos/<owner>/<repo>/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>`。
- 设置原生依赖：`gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`。
- 上述 API 的 `<owner>/<repo>` 为 `xinxinxiaoxin/ag-relay`；URL 中使用工单编号，提交的 id 使用数据库 id：`gh api repos/<owner>/<repo>/issues/<n> --jq .id`，不是 `#number` 或 `node_id`。
- `issue_dependencies_summary.blocked_by` 只统计未关闭的阻塞项；所有阻塞工单关闭后才能继续被阻塞的任务。
- 不支持子工单时，在父工单正文用任务列表关联，并在子工单顶部写 `Part of #<parent>`；不支持原生依赖时，在子工单顶部写 `Blocked by: #<n>, #<n>` 并检查所列工单是否关闭。
