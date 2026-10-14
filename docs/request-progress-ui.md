# 首页请求进展与诊断解释

更新日期：2026-10-14。本文描述面板展示；同期核心边界修复见 [重试与取消边界](retry-boundaries.md)。没有改变重试次数、等待间隔、超时参数或退出流程。

## 展示契约

- 首页上方仅保留紧凑控制条和启动与退出入口。请求进展与最近活动均使用全宽紧凑表格，一条记录一行；最近活动不重复列出周期性的 `request_status`。
- 使用核心已有 `/_relay/status.requests`：请求编号、阶段、尝试次数、累计耗时、距上次内容进展时间。沿用约 3 秒的状态读取，不额外请求上游。
- 最多传递 100 条明细。明细截断明确提示；旧核心缺少明细不等于空闲；离线不保留旧请求作为当前进展。
- `sinceLastProgressMs` 的显式 `null` 表示尚无内容进展。缺失/非法字段在 IPC 中省略，显示“内容进展时间未知”。未知阶段保留原始阶段码，不猜测卡死或完成倒计时。
- 推理、文本和工具调用表示最近收到的内容类型，工具调用输出不代表工具已执行完成。

## 活动事件

- `summary` 为直接显示的简短原因/结果，`attempt`、`httpStatus` 从已有诊断数字透传（缺失显示 —）；`detail` 为普通用户解释，`technicalDetail` 保留汉化技术解释，均按需展开。活动列为时间、请求、级别、事件、尝试、HTTP、原因/结果及展开按钮。
- 增加 `code` 和可空的 `requestId`。搜索覆盖解释、技术信息、事件码和请求编号。点击请求编号可筛选现有日志中的关联事件，不声称已加载完整永久历史。
- 日志页展开详情会暂停自动更新，防止阅读期间列表移位。分页每页 20 条，保留冻结阅读、错误提示与重读机制。展开不触发额外读取。
- `sse_ended` 只有 `sawTerminal=true`、正常终止类型、且 `stallReason=null` 同时成立，才显示“回复已完整结束”。只有 HTTP 200 或不完整/未知终止证据不宣称完成，也不推算恢复成功率。
- 没有新增核心诊断事件、没有持久化请求内容或原始报错正文，诊断文件及轮转契约不变。
- 关闭窗口保留取消、托盘、彻底退出三个操作，只精简解释。托盘不可用、正在退出和退出失败仍单独提示。

## 隔离验证

已运行：

- `gateway_status::tests`：28 项，使用维护说明中的四项 `--skip` 安全过滤。
- `gateway_diagnostics::tests`：17 项。
- `gateway_diagnostics_reader::tests`：19 项。
- `gateway_activity::tests`：6 项。
- `tests/window.test.ts`、`tests/update.test.ts`、`scripts/window-chrome.test.mjs`：14 项。
- `tests/request-progress.test.ts`、`tests/activity-table.test.ts`、`tests/components.test.ts`：9 项，使用 Vite SSR 加载 Node 测试，验证真实组件静态渲染及状态映射。
- 前端构建、类型检查与 lint；lint 无错误，保留既有警告。

组件测试入口（无需新增依赖）：

```powershell
node --input-type=module -e "const {createServer}=await import('vite'); const s=await createServer({server:{middlewareMode:true},appType:'custom', optimizeDeps:{noDiscovery:true}}); try {await s.ssrLoadModule('/tests/request-progress.test.ts'); await s.ssrLoadModule('/tests/activity-table.test.ts'); await s.ssrLoadModule('/tests/components.test.ts')} finally {await s.close()}"
```

浏览器隔离入口：`tests/fixtures/ui-preview.html`，仅用于 Vite 开发验证，不进入生产入口。所有 IPC 均被模拟拦截，未列出的操作直接失败：

- `?mode=legacy` / `unknown` / `empty` / `offline` / `truncated`：请求明细兼容与空态。
- `?close=1`：显示关闭确认；模拟彻底退出返回超时错误，验证弹窗保留。
- `?close=1&tray=0`：托盘不可用时禁用相应按钮。

浏览器已核对 1280×820、1048×735 的布局，以及日志请求编号筛选、技术信息展开和退出失败提示。全页无障碍扫描仍报告既有侧栏对比度、重复 main 和全局区域标注问题；本次新增卡片的命名与角色已修正，不能将这轮结果称为全页无障碍验收通过。

## 未执行的验收

没有访问现用网关、没有使用真实密钥/上游，没有安装、启动、停止、重启或替换现用应用。实现阶段没有创建新安装包或更新发布快照；后续发布记录以 README 打包快照为准。上述隔离契约和浏览器模拟验证不能替代独立环境中的成品验收。

输入/输出 token 及 tok/s 暂不采集。已有诊断记录没有这几项的完整数据；不做额外分词、SSE 重解析、额外上游请求或新增逐块写盘。不能保证每个上游都返回用量，也不把事件数/字数当 token 数。速度优先，不为填表引入转发热路径统计。
