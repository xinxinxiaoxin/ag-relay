# codex-context-cleaner：请求侧历史清理机制研究

研究日期：2026-10-16
性质：静态源码研究，不是故障复现报告、实施规格或生产修复验收。

## 1. 先说结论

**可以借鉴“只处理本次发出的请求，不改本地聊天文件”的边界，但不宜照搬它的清理算法，也不能据此认定用户的问题已解决。**

- 它不是 `~/.codex/sessions` 文件修复工具，而是另一个本机 HTTP 代理：每次转发前删除整个 `reasoning` 历史项，再删除其余 `input` 项的顶层 `id` 键。消息正文、工具参数、工具结果和 `call_id` 通常原样保留。删除发生在内存中的请求副本上。[S1][S2]
- 它**不是“遇到明确报错才清理”**：没有识别 `azure_resource_mismatch` 或缺失工具结果错误的逻辑，也没有收到错误后改写重试的逻辑。正常请求也会先清理，代价是丢掉本来有效的推理历史。[S2][S3][S4]
- 它**没有清除所有 `encrypted_content`**：只因整个 `reasoning` 项被丢弃，连带移除该项内的密文；非 `reasoning` 项上的同名字段以及嵌套字段不清除。[S2]
- 它没有核对工具调用与结果是否成对。`custom_tool_call` / `custom_tool_call_output` 只是走通用分支，并无专门修复；缺少的结果不会被补齐。[S2][S5]
- **最重要的协议风险**：它保留 `web_search_call` 却删除其 `id`。OpenAI 官方 Responses 请求类型明确要求此类历史项具有 `id`、`action`、`status`、`type`；不能把本地结构解析成功当作服务器协议合法。[S2][O1][O2]
- 固定源码树没有明确 `LICENSE` 或授权声明，GitHub 许可识别也为空。本次只研究思路，不复制源码、不内置第三方脚本、不增加运行依赖。[S9]

对使用的帮助：这项研究把“剥离问题历史”缩小为可讨论的请求侧恢复手段，指出哪些内容不能盲删，避免为了继续聊天而破坏工具配对、网页搜索历史或远端会话状态。**它尚不能证明哪一项历史触发了用户的 HTTP 400，更不能保证清理后上游服务会接受。**

## 2. 用户现象与证据边界

本次任务的目标是：用户主要通过本地网关使用 Codex，希望在明确报错后剥离问题历史；Claude 尚未报告同样的现象，不因此扩大为所有客户端的通用清理策略。

主会话转来的历史调查记录报告了 **HTTP 400 + 本仓诊断分类 `azure_resource_mismatch`**，相关 Codex 历史类型包括 `reasoning` + `encrypted_content`、`custom_tool_call` / `custom_tool_call_output`、`web_search_call`。`azure_resource_mismatch` 在 AG Relay 中是根据上游响应 message 的特定跨资源短语生成的分类 kind，不代表已证实官方 API 原生返回了同名 `error.code`。[A2]

必须区分：

1. 这是**被引用会话报告的历史现象**，不是本轮新抓到的真实请求或成功复现。
2. 没有拿到该次失败的原始 HTTP 响应正文，也没有在本轮取得请求与响应的完整对应证据。
3. 类型出现不等于该类型就是病因；本仓分类名称也不足以证明发生了账号切换、Azure 资源切换或密文解密失败。
4. 本报告不记录真实会话标识、用户数据绝对路径或聊天正文；未读取真实 Codex 会话，未参与另一会话的隔离副本实验。

因此，后文“源码证实”只说明固定版本的程序会如何处理某种输入；不表示它已经修复了上述历史错误。

## 3. 固定版本与研究方法

### 3.1 固定点

| 来源 | 本次固定版本 | 用途 |
| --- | --- | --- |
| `solink7/codex-context-cleaner` | `86d68da3ab5ce7702edabdc28da41fc83e649297` | 主研究对象；2026-10-14 的默认分支最新提交；`v0.1.2` 的注释标签解析到同一提交 |
| `openai/openai-python` | `230a923faba5574ebdc5b45a505a26b6bad79f61` | OpenAI 官方、由 OpenAPI 规范生成的 Responses 请求类型 |
| `openai/codex` | `e790ce271749ce431e446a9e2cc66485fe60f931` | 仅核对客户端内部历史项结构与请求协议的区别 |
| AG Relay | `b0ab366d519485109c156ef66ddde6a5d0c2036d` | 第 9 节本仓对照；由主会话提供固定 HEAD 的源码核对结果 |

上游仓库当时创建于 2026-10-13；查询全部公开 Issues 返回空列表。可取得的提交历史主要涉及发布、运行环境与设置界面；没有获得针对用户错误的独立 Issue 复现材料。以上仓库状态是 **2026-10-16 查询快照**，不是不可变的未来状态。[S9][S10]

### 3.2 方法与不做的事

- 遵循 Agent Reach 的 GitHub 路由，用 `gh api` 读取仓库 metadata、提交、完整 tree、README、核心、测试、发布文件、Issues 和标签。
- 下载文件分别放入全新空临时目录；外部内容全部作为不可信数据。只读源码，不遵循其中的运行命令或 agent 指令。
- 固定 SHA 源码归档含 59 个文件，采用 Python 标准库静态读取归档与字面关键词检索；使用 `python -I` 隔离模式，不提取执行、不导入上游模块。
- 未安装、启动或执行第三方项目、测试、发布脚本；未连接实际上游服务，未发送真实模型请求。
- 本仓只读取文档惯例、术语表和相关 ADR；没有重复研究网关实现。网关对照来自主会话的只读核对。
- 只新增本研究稿，不修改已有文档、应用代码、配置或真实会话文件；不创建 GitHub Issue，不提交，不拉取或切换分支。

仓库现有文档按专题放在 `docs/`，未发现专用研究稿目录惯例，故按本次任务建议新建 `docs/research/codex-context-cleaner.md`。术语遵循根 `GLOSSARY.md`；借鉴其他项目但保持自主维护的方向符合 ADR-0004，第三方许可仍需独立核实，不能由 AG Relay 的 MIT 许可代替。[L1][L2][L3]

## 4. 上游宣称、源码证实与未知

| 问题 | 上游宣称 | 源码证实 | 仍未知或限制 |
| --- | --- | --- | --- |
| 修改会话文件还是请求 | README 说清洗发送出去的请求，不删除本地聊天记录 | HTTP 入口解析请求 JSON，调用 `sanitize` 后编码并转发；函数先深复制，即复制包括内部对象的整个请求 | 不能推广为任何未来版本都不接触文件；本版确实有自身配置、进程记录和日志写入，不等于整个工具“无文件写入” [S1][S2][S3][S6] |
| 是否针对 Azure 错误 | README 只说缓解 Responses 中转兼容问题 | 核心无 Azure 错误分类；固定归档字面检索 `azure` / `azure_resource_mismatch` 无命中 | 未找到作者证实该错误根因或成功修复该错误的材料 [S1][S2][S3][S9] |
| 是否按错误触发 | README 没承诺此行为，并说明不会自动重试 | 每次请求发出前清理；上游状态与响应体透传，无错误后第二次请求 | 不满足本次“明确报错后再剥离”的触发要求 [S3][S4] |
| 是否保留消息和工具 | README 宣称保留消息、工具调用、文件引用 | 非推理项除了顶层 `id` 外保留；工具定义不变；普通函数调用的合成测试验证 `call_id` / 结果 / `file_id` 保留 | 并不等于所有工具历史合法或完整；网页搜索存在必需 ID 风险 [S2][S5][O1] |
| 是否消除密文 | README 明说不解密 | 没有通用密文字段清除；仅删除 `reasoning` 整项 | 非推理项密文、加密函数参数等不因此消失 [S2][O8] |
| 是否安全处理远端历史 | README 建议旧会话历史不可用时新开会话 | 显式 `conversation`、`item_reference`、`compaction` 阻断，默认拒绝 `previous_response_id` | 完整历史开关只相信用户声明；无类型裸引用检查有缺口 [S2][S7][O2] |

## 5. 精确清理规则

规则全集位于 `sanitizer.py:86-110`，没有另外隐藏的工具修复层。[S2]

### 5.1 删除什么，保留什么

| 输入位置 / 类型 | 固定版本的实际处理 | 不应误读成 |
| --- | --- | --- |
| `input[]` 中 `type == "reasoning"` | **删除整个数组对象**，无论有没有密文；`id`、`summary`、`content`、`encrypted_content` 一起不再发送 | 仅删除 `encrypted_content`；只删除失效密文；将密文改为空串或 `null` |
| 其余允许继续处理的 `input[]` 对象 | **删除顶层 `id` 键**；不是赋空值；其他键和值保留 | 递归删除一切 ID；仅删除 reasoning ID |
| `call_id`、工具 `name`、`arguments`、`input`、`output` | 原样保留；数组顺序不变 | 已做工具配对校验；已补齐缺失结果 |
| 嵌套 `content`、文件 `file_id`、嵌套 `id` | 原样保留 | 清理文件远端引用或所有嵌套标识 |
| 非 reasoning 项的 `encrypted_content`、请求其他位置的同名键 | 不处理 | 清除所有加密内容 |
| 顶层 `tools`、`reasoning` 配置、`include` 等 | 原样保留 | 禁用下一轮推理或阻止新响应产生密文 |
| `input` 为字符串 | 不进入数组清理分支，原样返回 | 字符串内容也会被截断 |
| `input` 非数组的其他值 / 未知项类型 | 没有完整请求 schema 校验；未知对象通常走通用去 ID 分支 | 已覆盖全部 Responses 新类型 |

源码并不读取持久会话的 `response_item.payload.id` 路径。它处理的是**已经成为 HTTP 请求的 `input[]` 项**；本地历史封装与实际请求不能直接当成同一种 JSON。[S2][S3]

### 5.2 工具调用与结果配对

`call_id` 是把一次工具调用与其结果连接起来的标识，和服务器给历史项分配的 `id` 不是同一个用途。官方自定义工具请求类型明确用 `call_id` 配对调用与结果。[O3][O4]

本版清理器：

- 不收集调用集合，不匹配结果，不核对先后顺序、同名标识或工具类型，也不检测重复调用。
- 对完整、正常的 `function_call` / `function_call_output`，保留 `call_id` 可维持原有配对；但如果原本缺失结果，清理后仍缺失。
- 没有 `custom_tool_call` / `custom_tool_call_output` 的专门分支或专项清理测试；这两类可通过通用分支，保留调用文本与结果，仅删除各自顶层 `id`。
- 不识别或修复 `missing function output` 一类错误，更不能据此凭空构造工具执行结果。[S2][S5]

测试的本地假上游会读取请求并回显，而不是执行真实 Responses 输入校验；其中甚至直接发送单独的 `function_call_output` 验证 ID 与认证头转发。这是运输层测试，不是“孤立工具结果也合法”的证据。[S5]

### 5.3 远端历史、压缩与 ID 连续性

| 情形 | 上游清理器行为 | 影响 / 风险 |
| --- | --- | --- |
| 非空 `conversation` | 抛错，本地返回 HTTP 400 | 拒绝依赖服务端历史的会话，不尝试展开远端内容 |
| 非空 `previous_response_id`，默认配置 | 抛错，不转发 | 避免无依据丢失只存在服务端的前文 |
| `assume_full_history=true` | 删除 `previous_response_id` 键 | **不检查历史是否真的完整**；只有增量 input 时会丢掉所依赖的前文 |
| `previous_response_id` 为假值 | 键仍会被删除 | 默认保护检查的是非空/真值，不是只要存在键就拒绝 |
| 显式 `type=item_reference` | 抛错 | 不会解析远端项引用 |
| 无 `type`、只有 `{ "id": "…" }` 的裸引用 | 未被识别，去 ID 后成为空对象 | 官方请求类型允许省略 `item_reference.type`；因此上游宣称的远端引用保护不是完整覆盖 |
| 显式 `type=compaction` | 抛错 | 不删除压缩密文、不尝试还原前文 |
| `/responses/compact` 及 `/v1/responses/compact` | 路径可以转发，仍先执行相同清理 | 响应直接透传；一旦后续 input 使用返回的 `compaction` 项，就会被清理器拒绝 |
| `compaction_summary` / `context_compaction` / `compaction_trigger` | 无专项分支，按通用逻辑处理 | 不能声称兼容；当前 Codex 内部结构还包含别名与新类型，而 cleaner 仅显式阻断 `compaction` |

以上处理见 [S2][S3][S7]。裸引用的协议依据见 [O2]；官方压缩项的 `encrypted_content` 是必需字段，不能把删除密文理解为已恢复压缩前的内容。[O5] Codex 的压缩别名与其他内部类型见 [O8]。

## 6. 必须单独指出的 `web_search_call` 协议风险

### 6.1 已核实的一手事实

1. `cleaner` 只有 reasoning / 远端引用 / compaction 的类型分支；`web_search_call` 会被保留，同时执行 `item.pop("id", None)`。[S2]
2. 官方 `ResponseFunctionWebSearchParam` 使用 `Required` 标记 **`id`、`action`、`status`、`type`**。`total=False` 不会取消这些显式标记的必需字段。[O1]
3. 这不是只返回给客户端的 output 类型：官方 `ResponseInputItemParam` 联合类型明确包含 `ResponseFunctionWebSearchParam`，用于后续 Responses 请求 input。[O2]
4. Codex 内部 `ResponseItem::WebSearchCall` 的 `id`、`status`、`action` 都是 `Option`，缺失时可以省略。这个内部数据模型宽于官方请求类型。[O6]

### 6.2 能推出什么，不能推出什么

**能推出**：清理器的“保留全部工具，只通删历史项 ID”策略与此官方请求类型存在明确冲突。若给 AG Relay 加上网页搜索白名单后仍一律去 ID，也会遇到同一问题。

**不能推出**：本次用户报错一定由网页搜索 ID 导致，或某个 Azure / 中转兼容端点一定严格按这份类型声明拒绝请求。官方 SDK 是声明层证据，本次没有实际调用任何端点。准确判断是“与官方类型定义冲突、不能保证协议安全，可能引入缺字段错误”，不是“已实测会被 Azure 拒绝”。

本地 rollout 副本可以重新读取、Rust 内部结构可以反序列化、假上游可以回显 JSON，分别只证明对应那一层可用，**均不能替代真实 Responses 协议与实际上游服务的验收**。

同样，不能只因 `web_search_call` 看起来“不是用户正文”就删除整个历史项或转换成消息；它可能承载工具动作及来源连续性，本次没有证实一种通用安全转换方案。

作为对照，官方普通函数调用和自定义工具调用 / 结果类型的顶层 `id` 在所查版本中是可省略的。不同历史类型必须分别判断，不能从这些类型推导网页搜索的 ID 也可省略。[O3][O4][O7]

## 7. dry-run、备份、原子替换与上下文损失

### 7.1 没有会话清理的 dry-run / backup

这里的 dry-run 指“只预览将删除哪些内容、不实际应用”；backup 指“保存被改文件的可恢复副本”。本版没有面向会话清理的这两项功能。

- 清理函数深复制请求，原参数不变；这是内存副本边界，**不是持久备份或用户可查看的清理预览**。[S2][S5]
- `--check-config` 只校验配置并输出端口，不启动代理；不是对历史清理规则的 dry-run。[S8]
- 请求处理不把原始或清理后聊天正文写入备份文件；默认 HTTP 访问日志被关闭，完成时只打印状态。[S3]

### 7.2 的确有原子写入，但对象是自己的配置

“原子替换”即先写好临时文件，再一次性换成目标文件，避免目标只写了一半。`control.py:26-46` 对**工具自己的配置文件**使用同目录临时文件、flush、fsync、`os.replace`；解析真实目标路径以保留 macOS 的配置符号链接。`apply` 在新配置启动失败时尝试恢复旧配置并重启旧代理，测试覆盖了这条回滚路径。[S6]

不能把它描述成“备份 Codex 历史后原子清理”：它没有做这件事。配置恢复也有失败边界，旧代理可能重新启动失败；并非所有意外中断都承诺可恢复。[S6]

### 7.3 可能删除正常上下文吗

**会**。代码并不判断 reasoning 是否失效：所有 `reasoning` 项都被删，包括没有密文、只有摘要或明文内容的项；README 也承认模型会失去对应推理连续性。[S2][S4]

官方 reasoning 输入类型建议手工管理历史时在后续轮次带回这些项，并说明完成事件中的 `encrypted_content` 对无状态、`store=false` 或零数据留存场景的重要性。这说明推理历史不是普遍无用的垃圾，清理应被视为有代价的恢复降级，而非无损优化。[O9]

其他风险还包括：误开启完整历史开关而丢失服务端前文；对未知新类型通删必需 ID；透传非 reasoning 密文却让使用者误以为已经全部去除。它保留普通消息不代表模型保留了全部工作状态。[S2][O1][O2][O9]

## 8. 测试与许可：不能夸大的保障

### 8.1 已看到的测试

静态阅读了 `tests/test_sanitizer.py`：

- 验证清理不修改原参数、普通函数调用 / 结果的 `call_id` 与内容保留、文件引用和 tools 保留。[S5]
- 验证默认拒绝远端历史；显式完整历史开关移除前序响应 ID。[S7]
- 验证路径别名、认证转发、SSE 首块、HTTP 429 透传、不合法 JSON 或显式 compaction 不送到假上游，以及断连不自动重试。[S5][S7]

**没有看到这些专项保障**：真实 Azure 错误复现；网页搜索请求必需字段校验；自定义工具配对；缺失/孤立/重复工具结果恢复；非 reasoning 密文字段清理；裸引用；完整历史真实性检测。这里的“没有看到”指所锁定源码与测试，不是断言作者从未私下做过实验。

本次没有运行这些测试。即使上游提交说明声称测试通过，也不转述为本次验证通过。

### 8.2 许可状态

在所锁定完整源码树与 59 个归档文件中：

- 未发现 `LICENSE` / `COPYING` 等授权文件或 SPDX / copyright / license 授权正文。
- GitHub repository metadata 的 `license` 为 `null`；许可 API 返回 HTTP 404。
- 发布构建清单枚举 README、核心、配置与平台资源，没有提供项目源码许可证的复制步骤。[S9]

结论是**未找到明确许可**，不是替作者判定法律权属。公开可阅读不等于已经明确允许把源码复制进 AG Relay 后分发。现阶段建议只借鉴思路，独立设计与实现；如未来拟复制实现或引入依赖，先让维护者核实并取得明确授权。AG Relay 的 MIT 决定不会自动覆盖该项目。[L3]

## 9. 本仓对照：与上游清理器分开

本节基于主会话对 AG Relay 固定 HEAD 的源码核对；本研究未重复读取网关核心，也未执行本仓测试。这里记录已有机制，不表示它已经处理过本轮真实失败。

| 对照点 | cleaner 固定版本 | AG Relay 固定 HEAD 的已有行为 |
| --- | --- | --- |
| 触发与范围 | 所有支持路径的请求转发前清理 | 限定 POST `/v1/responses`；一般首次正常请求不清理；命中过 Azure 恢复的已登记兼容会话存在请求前预清理，不能写成任何情况下都只清一次 [A1] |
| 错误识别 | 没有 Azure / 推理错误分类 | 精准识别 Azure 跨资源短语及特定推理错误，尝试 `stripItemIds: azure` 与 `dropReasoning: true`；同一次请求改写后不重复改写 [A2] |
| 完整历史要求 | `input` 非数组通常不清理；full-history 开关只相信声明 | repair 要求完整 `input` 数组，拒绝 `previous_response_id` / `conversation`，遇到远端引用或未知类型不贸然恢复 [A3] |
| 密文 | 非 reasoning 密文不处理，仍可继续转发 | 非 reasoning 密文构成拒绝恢复的边界 [A3] |
| 工具身份 | 保留 `call_id`，不核对配对 | 保留工具字段和 `call_id`，检查调用在同类结果之前；支持 `custom_tool_call` / `custom_tool_call_output` [A3] |
| 网页搜索 | 保留并通删 ID | `web_search_call` 仍作为未知项拒绝 repair；这是恢复保守边界，不是正常转发时禁止网页搜索 [A3] |
| 裸引用 | 无显式 type 时漏过，可能变空对象 | 无 type、只有 ID 且没有消息 role 的对象同样不能安全恢复，落入拒绝分支 [A3] |
| 恢复被拒 / 用尽 | 不自动重试 | 满足 Codex 标识与开关条件时，可以保留原请求字节作有限重试；资源错误预算最多 5 次。原样重试不等于已清理历史 [A2] |
| 持久会话 | 不改真实 Codex 会话文件 | 仅改内存请求，无 Codex 会话文件读写 [A1][A2][A3] |

因此 cleaner **不是 AG Relay 现有恢复机制的完整替代品**。本仓已具备更严格的触发、历史完整性与工具配对边界；上游主要提供请求侧处理的参考，而不是值得整体导入的新依赖。

用户在 Claude 中没遇到同类现象，是重要的实际观察，但不能仅凭客户端名称解释原因。本仓这套历史恢复只作用于 Responses 端点及完整数组历史；`/v1/messages` 等其他端点不走这套恢复。应核对实际请求协议、历史类型与上游服务条件，不能据此判定“Codex 一定有缺陷”或“Claude 永远不会遇到资源绑定问题”。[A1][A2][A3]

本仓已有随机端口假上游配合随包 Node 的连接测试设施，已有 HTTP 错误、200 JSON 与 SSE 情形的断言，包含只丢推理而保留工具内容、未知项 / 压缩 / 引用 / 孤立或错型结果拒绝恢复后原字节重试的边界。可在未来独立验证运输与恢复逻辑，不需要依赖 Tauri 面板操作；但本轮没有运行这套测试，不能把已有断言写成“本次测试通过”，也不能由假上游验收替代实际 Responses 服务兼容验收。[A4]

## 10. 阶段性建议与未决问题

以下只是研究判断，不形成具体实施 spec，也不提出一个已确定安全的历史转换算法。

### 值得保留的方向

- **请求侧、内存副本、可明确告知损失**：不动用户的持久历史，减少不可逆损失。
- **明确错误类别后才考虑恢复**，继续区分 Codex 与没有同样现象的其他客户端；不能仅凭 HTTP 400 就删历史。
- **按历史项类型判断字段是否可删**；`id` 与 `call_id` 分开，工具调用与结果作为有关联的整体评估。
- **保守对待远端引用、压缩项和非 reasoning 密文**：不能还原时明确拒绝，不冒充恢复成功。
- **分类型规则与实际网关链路验收**：这是比通删字段更可靠的研究方向。即便提出“保留网页搜索必需 ID，只去 reasoning”的候选，也仍需验证；原网页搜索 ID 是否涉及上游资源状态，本次未知，不能保证它修复跨资源错误。失败时拒绝恢复，不静默删除搜索、工具或压缩历史。

### 不建议照搬的部分

- 所有正常请求默认删 reasoning；无条件对所有类型去 `id`。
- 仅增加 `web_search_call` 白名单，却继续沿用通删 ID。
- 把 custom tool 通用透传当作完整支持；把缺少工具结果当作推理密文错误处理。
- 用开关名字“假定完整历史”代替实际完整性保障。
- 在没有明确许可时复制第三方源码或内置其启动脚本。

### 在讨论生产改动前仍缺的证据

1. 精确失败响应的错误结构与参数位置，以及对应请求里实际包含的历史形状；需按隐私边界取得，不能从历史类型列表猜。
2. 在相同模型与上游服务条件下区分：原请求、仅丢 reasoning、按类型处理可省略 ID等是否分别被接受。离线结构通过不能回答此问题。
3. 如果网页搜索或压缩历史涉及服务端状态，其原样保留、拒绝恢复或其他处理各有什么代价；当前没有一手证据证明通用无损转换。
4. 一次恢复若成功，如何向使用者说明丢失了哪些连续性；保留了聊天正文不能表述为“无损修复”。

**当前可以下的判断**：这个项目证明了“请求侧删除推理项、保留普通消息与部分工具结构”的一种实现方式；它不证明 Azure 错误的根因，也不提供覆盖全部 Codex 历史类型的安全清理方案。本次建议停在研究结论，不直接落生产修复。

## 11. 一手来源索引

下列源码链接全部固定到完整提交 SHA；正文引用按事实所在行范围定位。GitHub metadata / Issues / 提交列表等状态接口注明查询日期，不当作固定源码事实。

### 上游清理器

[S1]: https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/README.md#L1-L17
[S2]: https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/sanitizer.py#L86-L110
[S3]: https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/sanitizer.py#L113-L237
[S4]: https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/README.md#L113-L118
[S5]: https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/tests/test_sanitizer.py#L86-L271
[S6]: https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/control.py#L26-L46
[S7]: https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/tests/test_sanitizer.py#L103-L114
[S8]: https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/sanitizer.py#L240-L262
[S9]: https://github.com/solink7/codex-context-cleaner/tree/86d68da3ab5ce7702edabdc28da41fc83e649297
[S10]: https://github.com/solink7/codex-context-cleaner/commit/86d68da3ab5ce7702edabdc28da41fc83e649297

- **[S1]** README `1-17`：请求侧定位、保留对象和 HTTP 范围。
- **[S2]** `sanitizer.py:86-110`：完整清理算法。
- **[S3]** `sanitizer.py:113-237`：入口、请求清理后转发、响应透传、日志与不重试。
- **[S4]** README `113-118`：推理连续性损失、不记录正文、不自动重试的作者声明。
- **[S5]** `tests/test_sanitizer.py:86-114`：基本历史测试；`117-153`：假上游回显；`185-197`：孤立结果运输测试；`229-235`：本地拒绝；`267-271`：断连不重试。
- **[S6]** `control.py:26-46`：仅自己的配置原子写入；回滚另见 [`181-211`](https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/control.py#L181-L211)，测试见 [`test_settings.py:73-122`](https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/tests/test_settings.py#L73-L122)。
- **[S7]** 历史保护测试；作者的完整历史开关与旧会话提示另见 [README `76-106`](https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/README.md#L76-L106)。compact 的原始声明另见 [首次公开 README `69-79`](https://github.com/solink7/codex-context-cleaner/blob/33beadc20495c30a8b7d8e2847664382d826b453/README.md#L69-L79)。
- **[S8]** `--check-config` 的实际意义与 CLI 参数。
- **[S9]** 固定源码树：许可文件缺项的范围证据；发布文件清单另见 [`tools/release.py:50-98`](https://github.com/solink7/codex-context-cleaner/blob/86d68da3ab5ce7702edabdc28da41fc83e649297/tools/release.py#L50-L98)。全 tree API：`repos/solink7/codex-context-cleaner/git/trees/86d68da3ab5ce7702edabdc28da41fc83e649297?recursive=1`，`truncated=false`。2026-10-16 通过 `gh api` 读取的 metadata `license=null`、`repos/solink7/codex-context-cleaner/license` 返回 404，全部状态 Issues 列表为空。
- **[S10]** 固定最新提交。`v0.1.2` 标签对象 `eb80a7e44c5c7754073eb855e58c3da8687ac636` 的 `object.sha` 同为主研究 SHA；核心文件提交列表在该固定点仅有首次公开、设置界面和品牌调整等提交，没有取得针对 Azure 错误的专项证据。

### OpenAI 官方请求类型与 Codex 内部类型

[O1]: https://github.com/openai/openai-python/blob/230a923faba5574ebdc5b45a505a26b6bad79f61/src/openai/types/responses/response_function_web_search_param.py#L72-L92
[O2]: https://github.com/openai/openai-python/blob/230a923faba5574ebdc5b45a505a26b6bad79f61/src/openai/types/responses/response_input_item_param.py#L726-L804
[O3]: https://github.com/openai/openai-python/blob/230a923faba5574ebdc5b45a505a26b6bad79f61/src/openai/types/responses/response_custom_tool_call_param.py#L34-L56
[O4]: https://github.com/openai/openai-python/blob/230a923faba5574ebdc5b45a505a26b6bad79f61/src/openai/types/responses/response_custom_tool_call_output_param.py#L33-L52
[O5]: https://github.com/openai/openai-python/blob/230a923faba5574ebdc5b45a505a26b6bad79f61/src/openai/types/responses/response_compaction_item_param_param.py#L11-L23
[O6]: https://github.com/openai/codex/blob/e790ce271749ce431e446a9e2cc66485fe60f931/codex-rs/protocol/src/models.rs#L1228-L1251
[O7]: https://github.com/openai/openai-python/blob/230a923faba5574ebdc5b45a505a26b6bad79f61/src/openai/types/responses/response_function_tool_call_param.py#L34-L66
[O8]: https://github.com/openai/codex/blob/e790ce271749ce431e446a9e2cc66485fe60f931/codex-rs/protocol/src/models.rs#L1034-L1306
[O9]: https://github.com/openai/openai-python/blob/230a923faba5574ebdc5b45a505a26b6bad79f61/src/openai/types/responses/response_reasoning_item_param.py#L31-L68

- **[O1]** 网页搜索历史项必需 `id/action/status/type`。
- **[O2]** 裸项引用的 `type` 可省略，以及网页搜索等类型属于真实 input 联合类型。
- **[O3] / [O4]** 自定义工具调用 / 结果的 `call_id` 配对、顶层 ID 可省略。
- **[O5]** compaction 密文是必需字段。
- **[O6] / [O8]** Codex 内部可选字段、非 reasoning 密文字段、压缩别名与内部新类型；不作为服务器接收 schema。
- **[O7]** 普通函数调用的顶层 ID 可省略。
- **[O9]** 官方建议后续轮次带回 reasoning 及无状态密文连续性说明；`previous_response_id` / `conversation` 的服务器历史用途另见 [`response_create_params.py:58-63,148-154`](https://github.com/openai/openai-python/blob/230a923faba5574ebdc5b45a505a26b6bad79f61/src/openai/types/responses/response_create_params.py#L148-L154)。

### AG Relay 固定 HEAD 与领域文档

[A1]: https://github.com/xinxinxiaoxin/ag-relay/blob/b0ab366d519485109c156ef66ddde6a5d0c2036d/src-tauri/resources/gateway/relay-retry-proxy.mjs#L951-L973
[A2]: https://github.com/xinxinxiaoxin/ag-relay/blob/b0ab366d519485109c156ef66ddde6a5d0c2036d/src-tauri/resources/gateway/relay-retry-proxy.mjs#L1015-L1063
[A3]: https://github.com/xinxinxiaoxin/ag-relay/blob/b0ab366d519485109c156ef66ddde6a5d0c2036d/src-tauri/resources/gateway/relay-retry-proxy.mjs#L237-L315
[A4]: https://github.com/xinxinxiaoxin/ag-relay/blob/b0ab366d519485109c156ef66ddde6a5d0c2036d/src-tauri/tests/gateway_connection.rs#L420-L575
[L1]: ../../GLOSSARY.md
[L2]: ../adr/0004-owned-gateway-core.md
[L3]: ../adr/0007-mit-open-source-distribution.md

- **[A1]** `src-tauri/resources/gateway/relay-retry-proxy.mjs:951-973`：限定路径及已登记会话兼容预清理。
- **[A2]** 同文件 `1015-1063`：错误后恢复与原样有限重试；错误匹配另见 [`151-163`](https://github.com/xinxinxiaoxin/ag-relay/blob/b0ab366d519485109c156ef66ddde6a5d0c2036d/src-tauri/resources/gateway/relay-retry-proxy.mjs#L151-L163)。
- **[A3]** 同文件 `237-315`：完整历史、密文、未知项与工具配对保护；`287-295`：自定义工具；`296-298`：未知项拒绝。
- **[A4]** `src-tauri/tests/gateway_connection.rs:420-575`：假上游与随包 Node 测试设施。
- **[L1] / [L2] / [L3]** 本仓术语、自主维护与第三方许可边界；这些是本仓文档引用，不是上游能力证据。
