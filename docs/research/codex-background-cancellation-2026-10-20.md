# Codex 请求取消与“非流式后台审核”判断

研究日期：2026-10-20  
性质：公开一手源码研究，结合主会话提供的本机只读日志核对结果；不是故障复现或修复验收。仅新增本文，不修改产品代码、配置或真实会话。

> 历史快照：本文记录 2026-10-20 核对的固定源码与本机日志；文中“当前”“最新”均指当日快照，不代表后续版本或运行状态。

## 1. 结论

**这两条报错最合理的解释，是普通 Codex 流式任务在本地网关等待上游响应过久后，放弃旧尝试并重试；不是已经证实的“非流式后台审核请求”。**

依据不是只看报错措辞，而是本机两条请求的第二次尝试，分别发生在 **12:15:56.173、12:16:12.980**，与截图 **12:15:57、12:16:13** 的取消记录高度吻合；旧尝试此前已等待约 **179.6 秒、196.3 秒**，始终没有收到响应头。另一条成功请求的完成时间、耗时和 token 数也能对应面板，支持两处时钟相差约 1–2 秒的判断。[L1][L2]

实际影响要分开说：

- **请求 A 被网关重试救回**，12:20:26.823 收到 `response.completed`，有完整成功证据，不只是 HTTP 200。[L1]
- **请求 B 这一次网关转发确实失败了**：12:17:56.728 用尽 300 秒总预算，返回 503。不能称为“完全不影响正常使用”。但同一 Codex 任务后来再次请求，12:25:07 获得 200，随后产生工具调用并于 12:25:36 执行工具，说明任务内部后续恢复并继续工作。[L2][L3][L4]
- 所以“还能继续用”的原因，更符合**失败发生在某次尝试或某轮转发，之后有重试/恢复**，而不是“报错一定来自不重要的后台任务”。等待变长、短暂失败仍然是实际影响。

**“非流式”“0s”“0 token”“$0”都不是审核请求的身份证。** Codex 官方源码中，普通模型请求和 Guardian 异步风险评分请求都明确设置 `stream: true`；参考网关 New API 的错误日志又会直接把 token 和费用字段写成零。需要以原始请求标记及完整链路日志判断用途。[C2][C3][N1]

对使用的帮助：应优先检查“上游迟迟不返回响应头、网关重试和总预算”的链路，不应因为这两条记录就关闭 Guardian、改成非流式或调整审核设置。

## 2. 本机证据与适用边界

以下本机结果由主会话只读核对并提供，本研究子任务没有重复搜索数据库或日志；未把会话正文、请求 ID、内网主机名或密钥提交给搜索引擎。为方便复查，仅保存脱敏字段和本机短路径。[L1–L4]

### 2.1 时间线

时间均为 2026-10-20，本机 UTC+08:00；面板有约 1–2 秒偏差。A、B 为本文代号，不是真实请求 ID。

| 事件 | 请求 A | 请求 B |
| --- | --- | --- |
| 网关接收 | 12:12:56.6 左右 | 12:12:56.6 左右 |
| 请求形态 | 79 个 `input` 项、6 个工具、`reasoning=max` | 98 个 `input` 项、7 个工具、`reasoning=max` |
| 第一次尝试 | 约 179.6 秒停留在 `waiting_headers` | 约 196.3 秒停留在 `waiting_headers` |
| 第二次尝试开始 | 12:15:56.173 | 12:16:12.980 |
| 截图取消错误 | 12:15:57 | 12:16:13 |
| 后续 | 12:16:18.914 收到 200；12:20:26.823 `response.completed` | 12:17:56.728 总预算耗尽，503 |

来源：AG Relay 诊断日志相应行号见 [L1]。第一尝试期间未见 `upstream_response`、`client_closed` 或 `precommit_failure`；这支持“尚在等待响应头时进入下一尝试”，不等于日志已完整记录每个网络细节。[L1]

另外，截图中的正常消耗 **12:16:34 / 111 秒 / 输入 214274、输出 3471 token**，对应另一条正常流式请求：本地 **12:14:41.201 开始 → 12:15:19.135 收到响应头 → 12:16:32.549 完整结束**，历时约 111.35 秒，token 数与 CC Switch 记录相符。这是时间对齐的旁证，不是把“相邻成功行”直接当成 A 或 B。[L1][L2]

### 2.2 请求 B 的用途与恢复

主会话对请求 B 查到以下信息：

- Codex 调用链包含 `session_task.turn → run_sampling_request → stream_request → model_client.stream_responses_api → responses.stream_request`，CC Switch 同时记录 `is_streaming=1`。[L2][L3]
- 对应线程元数据为 `thread_source='user'`、`source='vscode'`、`originator='Codex Desktop'`、`agent_role=None`、`approval_mode='never'`，客户端版本 `0.163.0-alpha.10.2`。它是普通用户线程，不是 Guardian 专用审核线程。[L4]
- **12:17:56 的 503、12:22:57 的 503、12:25:07 的 200 属于同一 `turn.id`**；12:25:32 出现工具调用，12:25:36 开始执行 `exec` 工具。这比“后面又有一个 200”更强，能支持同一次任务后续恢复。[L3]

在更窄的截图窗口附近未看到 Codex ERROR/WARN，不能据此认定没有影响：请求 B 的明确 503 在 12:17:56 才到达，已经晚于截图中的两条取消记录。[L3]

### 2.3 对“超时取消”的确定程度

本仓固定版本存在完整对应路径：按请求体积计算等待响应头的时间上限；单次尝试用独立 `AbortController`，计时到期触发 `CONNECT_TIMEOUT`；`fetch` 失败后可进入下一尝试；总预算耗尽则返回 503。[A1][A2]

主会话还核对了运行日志 `gateway_started.loadedVersion`，其值 `9ad631456995939f2020c3713d797cf0bfd87b9548c51bb67b1c49771bbcd51e` 与本仓 `relay-retry-proxy.mjs` 的 SHA-256 完全一致，确认上述取消/重试源码与实际运行的网关入口文件吻合，不只是检查未部署代码。[L1][A2]

因此，**等待响应头超时后取消旧尝试，是当时首选解释**。但现有诊断没有单独留下这两次的 `CONNECT_TIMEOUT` 原因字段，所以不能把“具体就是该计时器触发”写成已逐条实锤；等待期间其他网络异常导致重试，尚不能完全排除。

这个剩余不确定点不改变主要判断：**本机证据指向普通流式任务的失败尝试与重试，而不是靠非流标签识别出的后台审核。** 远端面板没有提供可与本机直接连接的请求 ID，因此 A/B 与截图行是时间、错误阶段和旁证共同支持的高可信对应，不是 ID 级匹配。

## 3. 官方 Codex 源码：有后台功能，但不等于本次报错

### 3.1 固定版本

| 来源 | 固定点 | 用途 |
| --- | --- | --- |
| `openai/codex` | `rust-v0.163.0-alpha.10.2` → `4ebc485ca321243727539187169425fcf972f2c4` | 与本机日志报告的版本对应；标签解析自官方 GitHub API [C1] |
| `openai/codex` | `8196bcdc189ad2c189d631b321fa54b8e09b9bb0`，2026-10-20 查询时的 `main` | 补查当前后台标题、记忆、预先压缩和评分行为 |
| `QuantumNous/new-api` | `32e587406c6ebe891f18ab9aeb2f4a1f6b7b9e14` | 解释一种公开用量面板的错误字段语义；**不代表供应商部署了此仓库或版本** |
| `golang/go` | `go1.25.0` → `6e676ab2b809d46623acb5988248d95d1eb7939c` | 解释 `context canceled` 和 HTTP 请求上下文 |
| AG Relay | `94cc94f5815adccaab6c37c4ea4accf69c824320` | 本机超时、重试、总预算实现，由主会话核对 |

官方标签对应的公开源码能说明该版本设计，但不是对用户机器上二进制的完整性鉴定。最新 `main` 的附加功能也不能无条件倒推到所有旧版客户端。

### 3.2 核对结果

| 功能 | 一手源码证实什么 | 能否据“非流式”识别 |
| --- | --- | --- |
| 普通模型请求 | 与本机对应版本的 `core/src/client.rs` 构造 Responses 请求时明确写入 `stream: true`、`tool_choice: "auto"`。[C2] | 不能；本机 B 的调用链也确实是流式。 |
| Guardian 异步评分 | 与本机对应版本中，评分请求设置 `tools: None`、`tool_choice: "none"`、`stream: true`，缓存键前缀为 `guardian-v2:`。[C3] | **不能，而且“Guardian 必然非流式”与该源码直接冲突。** |
| Guardian 同步批准审核 | 有单独的审核线程，使用 `InternalSessionSource::Guardian`、`ThreadSource::GuardianReview`，并设置专用 reviewer 路由头。[C6] | 审核用途应结合线程来源、审核标记和内容，不是只看流式开关；同步审核也不应默认视为“失败无影响”。 |
| Guardian 被更新结果取代 | 评分并发达到限制时会向旧请求发送 `supersede`；执行层在等待连接、发请求、读流时都可提前结束。[C3][C4] | 源码确有主动取消，但不能据此认领截图两条记录。 |
| 后台标题生成 | 当前 TUI 会启动隐藏临时线程，来源为 `Feature("thread_title")`；手工改名会取消待完成的标题生成；输出 schema 要求 `title`。[C7] | 标题是另一种后台用途，说明“后台”不等于“审核”。这是公开 **TUI** 实现，不据此声称桌面界面此次也调用了它。 |
| 记忆摘要 / 整理 | 当前 API 客户端存在 `memories/trace_summarize` POST，读取完整 JSON；记忆整理另有 `MemoryConsolidation` 内部线程。[C8][C9] | 并非所有摘要都走 `/responses`；路径和来源比“非流”更有区分力。 |
| 预先压缩上下文 | 当前 `async_compaction` 明确描述为预先启动的后台压缩：失败结果可丢弃，会话关闭会取消剩余网络工作。[C10] | 能说明还有其他可取消的后台工作，不能证明截图来自压缩。 |

当前 Guardian 预热代码还明确要求自动审核模式且不是 Full Access 才预热。**功能开关开启，不等于某条请求已经执行了审核。** 这里的源码只证实“预热有条件”，不能扩大成“Full Access 下所有 Guardian 行为都绝对不存在”。[C11]

## 4. `client cancelled` 到底是谁取消了谁

把链路理解成：**Codex → 本地代理 / AG Relay → 第三方网关 → 第三方内部上游**。

在某一层服务端口中的 `client`，指的是**直接连接这一层的调用方**。对第三方网关而言，client 可以是前面的本地代理，而不是用户亲手点击“停止”，也不一定是 Codex 本进程。

Go 官方源码说明：服务端收到的请求，其上下文可在客户端连接关闭、HTTP/2 请求取消或处理函数返回时被取消；出站请求的上下文则控制该次出站调用的取消。`context canceled` 与因本层上下文截止时间到期而产生的 `context deadline exceeded` 是不同错误。[G1][G2]

因此，上面一层发生超时并主动断开，本层观察到的完全可能是 **`context canceled`**。不能因为错误里没有 `deadline exceeded`，就排除更前面一层的超时。

### 完整错误短语溯源的限制

通过 GitHub `gh search code`、`gh search issues` 和 Exa 查找通用错误短语，并静态检查 New API 当前源码后，**没有确认生成截图完整措辞的公开实现**。New API 确有 `do request failed: %w` 和出站请求错误处理，但本次检查未找到完整的 `client cancelled request before upstream response` 文案。[N4]

所以：

- 可以解释 Go 取消机制和本仓取消/重试机制。
- **不能仅凭文案认定第三方就是原版 New API，也不能推定它如何传播取消、设置状态码或记录耗时。**
- 错误中的内部 `/v1/responses` 是某一跳的目标路径，不携带“审核”“标题”这类用途证明。

## 5. 为什么错误行会有 0 token、$0、0s、非流式

以下是 **New API 固定源码作为参考实现**，不是对第三方面板数据库字段的直接读取。

### 5.1 0 token 和 $0

`RecordErrorLog` 构造错误记录时直接赋值：`PromptTokens: 0`、`CompletionTokens: 0`、`Quota: 0`。[N1]

所以，在这种实现里，错误行的零值说明的是“该错误记录按零值存储”，**不足以证明上游完全没开始计算、请求原本没有输入，或整次用户任务免费**。成功重试还可以另外产生正常用量行；本机 A 的完整成功记录也应和旧尝试错误分开看。

### 5.2 0s

该实现用 `int(time.Since(startTime).Seconds())`，不足一秒会取整为 0；如果开始时间字段缺失，还会用记录时的 `time.Now()` 作起点。[N2]

但本机两条旧尝试已经等了约三分钟，因此**不能用“面板 0s”覆盖本机实际等待时间**。第三方的 0s 具体属于哪一跳、是否用了缺省起点、是否是另一种实现，本次没拿到其源码/版本和原始日志，不能指定一种原因。

### 5.3 非流式

当前 New API 会先取 `request.IsStream(c.Request)`，存到请求上下文，再用于错误日志；默认初值是 `false`，但正常解析后会按请求设置。[N2][N3]

**本次没有查到“New API 的错误记录一律显示非流式”的依据，不能把它当解释。** 对第三方截图，只能确定面板这样显示；至于它表示入站请求、出站请求、响应形态还是错误分支默认值，需要第三方实际版本和记录字段定义确认。

本机 B 已有 `is_streaming=1` 与 Codex 流式调用链。这意味着**至少不能把截图的非流式标签直接当成 Codex 原始请求的 `stream=false`，再进一步判定后台审核**。[L2][L3]

## 6. 后续怎样验证或推翻当前判断

不必为此长期保存所有请求正文。优先保留或对照下列脱敏元数据：

| 想核实的解释 | 有区分力的证据 | 什么会推翻或削弱它 |
| --- | --- | --- |
| 本地网关取消旧尝试并重试 | 同一逻辑请求下的尝试编号、开始/取消时间、是否收到响应头、取消原因、重试时间，以及远端请求 ID | 远端 ID 指向另一条请求，或本机旧尝试在截图时仍正常持续、没有重试。 |
| 具体是等待响应头超时 | 单次尝试取消原因为 `CONNECT_TIMEOUT`，与当次超时阈值吻合。[A1][A2] | 明确的连接重置、DNS/TLS 故障或用户取消记录；这些会改变“具体取消原因”，但不自动变成审核。 |
| Guardian 异步评分 | 请求/元数据含 `x-openai-subagent: guardian`；可能有 `x-codex-guardian: classifier`；缓存键 `guardian-v2:`；`client_metadata` 的来源为 `guardian_classifier`；与本机版本一致的 `tools` 为空、`tool_choice=none`、`stream=true`。[C3][C4][C5] | 在无中间层改写的前提下，同一请求被确认来自普通用户流式任务，带正常工具集合且不满足评分请求形态。注意头可能被代理剥离，缺一个头不单独定案。 |
| Guardian 同步审核 | `ThreadSource::GuardianReview` / 内部 Guardian 来源；相应 reviewer 路由和待审核动作上下文。[C6] | ID 关联回普通用户任务且不存在审核线程或审核动作。不要把 `review`、代码审查、Guardian 风险审核混为一谈。 |
| 标题请求 | 临时线程来源 `thread_title`、生成短标题的提示、仅要求 `title` 的结构化输出。[C7] | 请求是完整工具运行任务而非标题生成；也要确认使用的界面确实采用对应实现。 |
| 记忆摘要 / 整理 | `memories/trace_summarize` 路径，或 `MemoryConsolidation` 内部线程及 `memory_consolidation` 触发来源。[C8][C9] | 原始请求及任务来源明确属于正常用户问答；仅有 `reasoning.summary` 字段不能认定单独发出了记忆请求。[C2] |

关联时记录 **header 名、用途值、脱敏后的关联 ID、`stream`、模型名、工具数量、输入项数量** 已足以先做判断；不需要向搜索引擎发送完整请求、鉴权头、会话正文、真实路径或供应商内部地址。

本次没有需要立即修改的产品代码结论；尤其不应从面板标签直接推出“关掉审核即可解决”。若继续排查，应围绕上游长时间不出响应头及取消原因，而不是把所有取消错误归成可忽略的后台噪声。

## 7. 来源与复查位置

### 本机证据（由主会话核对，未提交到互联网）

- **[L1]** `%APPDATA%/ag-relay/logs/relay-diagnostics.jsonl`：A 接收约第 2192–2195 行，B 接收第 2204–2207 行；A 第二次尝试第 2291 行，B 第 2300 行；A 收到 200 / 开始 SSE 第 2304–2305 行；B 总预算 503 第 2369 行；A `response.completed` 第 2435–2436 行。行号属于本次读取快照，日志轮转后可能变化。
- **[L2]** `~/.cc-switch/cc-switch.db`，`proxy_request_logs` 表；`~/.cc-switch/logs/cc-switch.log` 第 69094、69116 行的预算耗尽 503。核对流式字段、错误信息及正常请求用量。
- **[L3]** `~/.codex/logs_2.sqlite`，`logs` 表：按上述时段关联流式调用链、同一 `turn.id` 的 503/恢复与工具执行；本文不保存原始 ID。
- **[L4]** `~/.codex/state_5.sqlite`，`threads` 表：请求 B 对应线程来源、客户端版本、批准模式。

### 本仓固定源码

- **[A1]** [按请求体积计算等待响应头时间上限，563–566 行](https://github.com/xinxinxiaoxin/ag-relay/blob/94cc94f5815adccaab6c37c4ea4accf69c824320/src-tauri/resources/gateway/relay-retry-proxy.mjs#L563-L566)。
- **[A2]** [单尝试控制器、CONNECT_TIMEOUT、fetch 重试、总预算 503、响应头诊断，1374–1426 行](https://github.com/xinxinxiaoxin/ag-relay/blob/94cc94f5815adccaab6c37c4ea4accf69c824320/src-tauri/resources/gateway/relay-retry-proxy.mjs#L1374-L1426)。

### OpenAI 官方源码

- **[C1]** [官方版本标签](https://github.com/openai/codex/tree/rust-v0.163.0-alpha.10.2)；[该注释标签的 API 对象，指向固定 commit](https://api.github.com/repos/openai/codex/git/tags/d9151c94a70013f176ff370fa93da02734662e41)。
- **[C2]** [本机对应版本：普通 Responses 请求构造，1071–1088 行](https://github.com/openai/codex/blob/4ebc485ca321243727539187169425fcf972f2c4/codex-rs/core/src/client.rs#L1071-L1088)；[当前 main 同一路径，1089–1106 行](https://github.com/openai/codex/blob/8196bcdc189ad2c189d631b321fa54b8e09b9bb0/codex-rs/core/src/client.rs#L1089-L1106)。
- **[C3]** [本机对应版本：Guardian 评分请求、stream=true 及 supersede](https://github.com/openai/codex/blob/4ebc485ca321243727539187169425fcf972f2c4/codex-rs/ext/guardian-v2/src/async_scorer/sampler.rs#L230-L347)；[当前 main 对照](https://github.com/openai/codex/blob/8196bcdc189ad2c189d631b321fa54b8e09b9bb0/codex-rs/ext/guardian-v2/src/async_scorer/sampler.rs#L233-L365)。
- **[C4]** [本机对应版本：Guardian 请求元数据、等待/读流期间的 superseded 处理](https://github.com/openai/codex/blob/4ebc485ca321243727539187169425fcf972f2c4/codex-rs/ext/guardian-v2/src/async_scorer/sampler/execution.rs#L290-L354)。
- **[C5]** [本机对应版本：Guardian headers 及专用路由条件，400–453 行](https://github.com/openai/codex/blob/4ebc485ca321243727539187169425fcf972f2c4/codex-rs/ext/guardian-v2/src/async_scorer/sampler/connection_pool.rs#L400-L453)。
- **[C6]** [本机对应版本：同步审核线程来源和 reviewer 路由](https://github.com/openai/codex/blob/4ebc485ca321243727539187169425fcf972f2c4/codex-rs/ext/guardian-v2/src/sync_reviewer/mod.rs#L77-L108)。
- **[C7]** 当前 main：[隐藏标题线程及取消，49–118 行](https://github.com/openai/codex/blob/8196bcdc189ad2c189d631b321fa54b8e09b9bb0/codex-rs/tui/src/app/thread_title.rs#L49-L118)；[title schema 和提示，250–290 行](https://github.com/openai/codex/blob/8196bcdc189ad2c189d631b321fa54b8e09b9bb0/codex-rs/tui/src/app/thread_title.rs#L250-L290)。
- **[C8]** [当前 main：记忆摘要 endpoint 与完整 JSON 响应读取，32–58 行](https://github.com/openai/codex/blob/8196bcdc189ad2c189d631b321fa54b8e09b9bb0/codex-rs/codex-api/src/endpoint/memories.rs#L32-L58)。
- **[C9]** [当前 main：记忆整理内部线程，399–424 行](https://github.com/openai/codex/blob/8196bcdc189ad2c189d631b321fa54b8e09b9bb0/codex-rs/memories/write/src/runtime.rs#L399-L424)。
- **[C10]** [当前 main：异步预先压缩的生命周期、启动条件及失败处理](https://github.com/openai/codex/blob/8196bcdc189ad2c189d631b321fa54b8e09b9bb0/codex-rs/core/src/session/async_compaction.rs#L1-L141)。
- **[C11]** [当前 main：Guardian 预热须满足自动审核、非 Full Access 等条件，128–147 行](https://github.com/openai/codex/blob/8196bcdc189ad2c189d631b321fa54b8e09b9bb0/codex-rs/ext/guardian-v2/src/async_scorer/extension.rs#L128-L147)。

### New API 参考实现与 Go 官方语义

- **[N1]** [New API：错误记录 token、Quota 置零，278–317 行](https://github.com/QuantumNous/new-api/blob/32e587406c6ebe891f18ab9aeb2f4a1f6b7b9e14/model/log.go#L278-L317)。
- **[N2]** [New API：错误耗时取整、读取流式标记，76–97 行](https://github.com/QuantumNous/new-api/blob/32e587406c6ebe891f18ab9aeb2f4a1f6b7b9e14/service/relay_error.go#L76-L97)。
- **[N3]** [New API：流式字段默认值与按请求赋值，597–607 行](https://github.com/QuantumNous/new-api/blob/32e587406c6ebe891f18ab9aeb2f4a1f6b7b9e14/relay/common/relay_info.go#L597-L607)。
- **[N4]** [New API：出站调用失败处理，482–486 行](https://github.com/QuantumNous/new-api/blob/32e587406c6ebe891f18ab9aeb2f4a1f6b7b9e14/relay/channel/api_request.go#L482-L486)；[`do request failed` 外层包装，529–535 行](https://github.com/QuantumNous/new-api/blob/32e587406c6ebe891f18ab9aeb2f4a1f6b7b9e14/relay/channel/api_request.go#L529-L535)。
- **[G1]** [Go：入站/出站 HTTP 请求上下文的取消语义，341–351 行](https://github.com/golang/go/blob/6e676ab2b809d46623acb5988248d95d1eb7939c/src/net/http/request.go#L341-L351)。
- **[G2]** [Go：Canceled 与 DeadlineExceeded 的区别，165–175 行](https://github.com/golang/go/blob/6e676ab2b809d46623acb5988248d95d1eb7939c/src/context/context.go#L165-L175)。

研究方法：遵循 agent-reach 的 GitHub / gh CLI 路由，Exa 仅经 mcporter 检索通用错误短语；各份源码下载到独立新建临时目录，仅读文本，不运行下载仓库的代码、测试或脚本。对完整错误措辞的检索未形成可确认的来源归属，不把搜索未命中当成“公开世界不存在”。
