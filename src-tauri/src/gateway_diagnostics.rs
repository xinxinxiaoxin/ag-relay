//! 诊断汉化核心（T4 工单）：壳侧只读消费者的**纯逻辑**层。
//!
//! 分层（沿用 backend-map「纯逻辑在 gateway_*」铁律）：本模块只做
//! 「一条诊断记录(已解析 JSON) → 可选活动事件」的提炼，**不碰文件/进程/网络**。
//! T5 的增量 tail / 分页读取薄层与 `#[tauri::command]` 命令都薄封装本模块的纯函数。
//!
//! 字典以**稳定封闭枚举**为键：`event` 加 `kind`/`source`/`refusal`/`terminalKind`/
//! `stallReason`/`phase`/`status`（HTTP 段）以及 `status_repair_refused.reason`。
//! 数字 / UUID / sha256 / 时间戳等动态槽**原样透传、绝不翻译**。
//! 未知事件码或未知子枚举值 → **诚实回退**（显示原始码），绝不隐藏、绝不 panic。
//!
//! 严重度不只看 `event`：`sse_ended` 由 `terminalKind`+`stallReason` 定级、
//! `finished`/`upstream_response` 由 HTTP `status` 段定级、`precommit_failure` 由 `kind` 定级
//! （依据 diagnostics-codes.md §4）。
//!
//! 权威事件码/子枚举清单：ag-relay-notes/diagnostics-codes.md（只读通读打包脚本得出）。
//! 覆盖率契约测试（本文件末尾 mod tests）照 `gateway_env`↔`gateway-config.bat` 范式：
//! 从只读打包脚本 `resources/gateway/relay-retry-proxy.mjs` 抽取全部 23 个 `event` 码，
//! 断言字典全覆盖；上游加码即红。

use serde_json::Value;

/// 活动事件级别（GLOSSARY：信息 / 警告 / 错误）。
/// serde 输出小写稳定机器码（info/warn/error），前端据此筛选/着色，中文由前端渲染或取 `as_cn`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityLevel {
    /// 信息
    Info,
    /// 警告
    Warn,
    /// 错误
    Error,
}

impl ActivityLevel {
    /// 中文标签（信息 / 警告 / 错误）——供需要直接展示级别文案处取用。
    pub fn as_cn(&self) -> &'static str {
        match self {
            ActivityLevel::Info => "信息",
            ActivityLevel::Warn => "警告",
            ActivityLevel::Error => "错误",
        }
    }
}

/// 活动事件：由一条诊断记录提炼的、面向用户的中文事件（GLOSSARY「活动事件」）。
/// `time` 是诊断记录里的 ISO8601 时间戳**原样透传**（透传槽，不解析不翻译）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub level: ActivityLevel,
    /// 中文标题（简短，面向「最近活动」列表）
    pub title: String,
    /// 中文详情（含透传槽：路径 / HTTP 码 / 次数等原样搬运）
    pub detail: String,
    /// 原始 ISO8601 时间戳（透传槽；缺失则空串）
    pub time: String,
}

// ───────────────────────── 公共纯函数（T5 包装入口） ─────────────────────────

/// 纯函数核心：一条**已解析**的诊断记录 → 可选活动事件。
///
/// - 无 `event` 字段（或非字符串）⇒ `None`（不是可展示记录）。
/// - `request_profile`（received 的兄弟记录，纯请求体字段画像）⇒ `None`（不面向用户）。
/// - 未知 `event` 码 ⇒ `Some(诚实回退)`：原样显示原始码、级别取信息、不崩。
/// - 其余 ⇒ `Some`：中文标题 + 中文详情 + 据子枚举定级。
pub fn localize_record(record: &Value) -> Option<ActivityEvent> {
    let event = record.get("event").and_then(Value::as_str)?;
    let time = record
        .get("time")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if SUPPRESSED_EVENTS.contains(&event) {
        return None;
    }

    let Some((base_level, title)) = event_base(event) else {
        // 未知事件码：诚实回退，原样显示原始码（不隐藏、不 panic）。
        return Some(ActivityEvent {
            level: ActivityLevel::Info,
            title: format!("未识别事件：{event}"),
            detail: "壳未收录此事件码，已原样显示原始码。".to_string(),
            time,
        });
    };

    let (level, detail) = refine(event, base_level, record);
    Some(ActivityEvent { level, title: title.to_string(), detail, time })
}

/// 便捷入口：一行诊断记录文本 → 可选活动事件（供 fixture 与 T5 读取层逐行调用）。
/// 非法 JSON 行 ⇒ `None`（跳过，绝不 panic）。
pub fn localize_line(line: &str) -> Option<ActivityEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    localize_record(&v)
}

// ───────────────────────── 字典数据（稳定封闭枚举为键） ─────────────────────────

/// 不面向用户、localize 返回 None 的事件码（仍在 EVENT_BASE 中登记，保证契约覆盖）。
/// `request_profile`：与 `received` 同 requestId 的请求体字段画像（diagnostics-codes.md §1-B#5），
/// 属内部诊断、不进「最近活动」。
const SUPPRESSED_EVENTS: &[&str] = &["request_profile"];

/// 事件基表：`event` → (默认级别, 中文标题)。**23 项，与 diagnostics-codes.md §1 全列一致**。
/// 默认级别对 severity-依赖子字段的事件（sse_ended/finished/upstream_response/precommit_failure）
/// 只是兜底，真正级别在 refine 里据子枚举覆盖。
const EVENT_BASE: &[(&str, ActivityLevel, &str)] = &[
    // A. 生命周期 / 状态（3）
    ("gateway_started", ActivityLevel::Info, "网关已启动"),
    ("gateway_draining", ActivityLevel::Info, "网关开始排空"),
    ("request_status", ActivityLevel::Info, "请求进行中"),
    // B. 单请求正常流程（8）
    ("received", ActivityLevel::Info, "收到请求"),
    ("request_profile", ActivityLevel::Info, "请求体画像"),
    ("attempt_started", ActivityLevel::Info, "发起上游尝试"),
    ("upstream_response", ActivityLevel::Info, "收到上游响应头"),
    ("sse_committed", ActivityLevel::Info, "开始流式下发"),
    ("sse_ended", ActivityLevel::Info, "流式响应结束"),
    ("finished", ActivityLevel::Info, "请求结束"),
    ("client_closed", ActivityLevel::Warn, "客户端提前断开"),
    // C. 会话兼容（4）
    ("session_compat_refused", ActivityLevel::Warn, "会话兼容修复被拒"),
    ("codex_session_passthrough", ActivityLevel::Info, "会话原样透传重试"),
    ("session_compat_applied", ActivityLevel::Info, "已应用会话兼容修复"),
    ("session_compat_enabled", ActivityLevel::Info, "已登记会话兼容"),
    // D. 请求体修复 / 上游错误处理（8）
    ("status_repair_refused", ActivityLevel::Warn, "status 字段修复被拒"),
    ("status_repair_applied", ActivityLevel::Info, "已删除 status 字段并重试"),
    ("item_error", ActivityLevel::Warn, "识别到 item 级错误"),
    ("repair_refused", ActivityLevel::Warn, "请求体修复被拒"),
    ("repair_applied", ActivityLevel::Info, "已修复请求体并重试"),
    ("codex_resource_retry", ActivityLevel::Info, "保留历史换通道重试"),
    ("codex_resource_exhausted", ActivityLevel::Error, "资源重试耗尽，放弃"),
    ("precommit_failure", ActivityLevel::Warn, "流式下发前失败"),
];

/// 子枚举中文标签表：(字段名, 枚举值) → 中文。**51 项**，按字段分组：
/// source 6 / kind 10 / refusal 5 / terminalKind 7(含 null) / stallReason 6(含 null) /
/// phase 8 / reason 6（status_repair 专属）/ status_band 3。
/// 字段名区分同名异义值（如 refusal 与 reason 都有 wrong_endpoint/repair_exhausted/time_budget）。
/// 取值与 diagnostics-codes.md §2 封闭枚举逐项对应。
const SUB_LABELS: &[(&str, &str, &str)] = &[
    // source（6）—— 错误识别层 + 会话兼容来源（§2.A）
    ("source", "http_error", "HTTP 错误响应"),
    ("source", "sse_precommit", "流式下发前"),
    ("source", "json_error", "JSON 错误信封"),
    ("source", "json_empty", "空回应（HTTP 200）"),
    ("source", "explicit_session", "显式会话"),
    ("source", "resource_lineage", "资源血缘"),
    // kind（10）—— item 2 + peek 6 + 空回应形状 2（§2.B）
    ("kind", "azure_resource_mismatch", "跨 Azure 资源引用"),
    ("kind", "stale_reasoning_item", "失效的 reasoning 项"),
    ("kind", "other", "其他"),
    ("kind", "peek_timeout", "等待首个内容超时"),
    ("kind", "empty_stream_drop", "上游静默丢弃（空流）"),
    ("kind", "read_timeout", "空闲读超时"),
    ("kind", "buffer_limit", "下发前缓冲超限"),
    ("kind", "stream_error", "流内携带错误"),
    ("kind", "empty_stream_shape", "空回应形状修复"),
    ("kind", "empty_stream_reasoning", "空回应删除 reasoning 项"),
    // refusal（5）—— 修复被拒 / 资源耗尽原因（§2.C）
    ("refusal", "unsafe_inline_history", "内联历史不安全"),
    ("refusal", "wrong_endpoint", "非 /v1/responses 端点"),
    ("refusal", "repair_exhausted", "单次修复额度已用尽"),
    ("refusal", "time_budget", "剩余预算不足"),
    ("refusal", "resource_attempt_limit", "资源尝试达上限"),
    // terminalKind（7，含 null）—— 流终止类型（§2.E）
    ("terminalKind", "null", "未见终止事件（截断）"),
    ("terminalKind", "done_marker", "[DONE] 标记"),
    ("terminalKind", "response.completed", "正常完成"),
    ("terminalKind", "response.failed", "失败终止"),
    ("terminalKind", "response.incomplete", "不完整终止"),
    ("terminalKind", "message_stop", "Anthropic 风格终止"),
    ("terminalKind", "chat_finish_reason", "Chat finish_reason 终止"),
    // stallReason（6，含 null）—— 下发后停滞原因（§2.F）
    ("stallReason", "null", "无停滞"),
    ("stallReason", "progress_timeout", "无生成进展超时"),
    ("stallReason", "client_closed", "客户端断开"),
    ("stallReason", "transport_idle_timeout", "传输层空闲超时"),
    ("stallReason", "upstream_read_error", "上游读取错误"),
    ("stallReason", "event_size_limit", "单事件超缓冲上限"),
    // phase（8）—— request_status 请求状态机（§2.G）
    ("phase", "receiving_request", "正在接收请求"),
    ("phase", "waiting_headers", "等待上游响应头"),
    ("phase", "reading_body", "读取上游响应体"),
    ("phase", "waiting_first_content", "等待首个内容"),
    ("phase", "stream_waiting", "已下发，等待后续"),
    ("phase", "reasoning", "正在输出推理"),
    ("phase", "tool_output", "正在输出工具调用"),
    ("phase", "text_output", "正在输出正文"),
    // reason（6）—— status_repair_refused 专属（§2.H，spec 7 字段列表外的额外封闭枚举）
    ("reason", "wrong_endpoint", "非 /v1/responses 端点"),
    ("reason", "repair_exhausted", "status 修复已做过一次"),
    ("reason", "unexpected_status", "非 400/200 状态"),
    ("reason", "invalid_json", "请求体非 JSON"),
    ("reason", "unsupported_item", "目标项不符合可删 status 的形状"),
    ("reason", "time_budget", "剩余预算不足"),
    // status_band（3）—— HTTP status 段，供 finished/upstream_response 定级与详情（§4）
    ("status_band", "2xx", "成功"),
    ("status_band", "4xx", "请求被拒"),
    ("status_band", "5xx", "上游故障"),
];

// ───────────────────────── 查表与透传辅助 ─────────────────────────

/// 事件基表查表。
fn event_base(event: &str) -> Option<(ActivityLevel, &'static str)> {
    EVENT_BASE
        .iter()
        .find(|(e, _, _)| *e == event)
        .map(|(_, lvl, title)| (*lvl, *title))
}

/// 子枚举中文标签查表。
fn sub_label(field: &str, value: &str) -> Option<&'static str> {
    SUB_LABELS
        .iter()
        .find(|(f, v, _)| *f == field && *v == value)
        .map(|(_, _, cn)| *cn)
}

/// 子枚举值 → 中文；未知值诚实回退为「原始码：X」（不隐藏、不 panic）。
fn sub_cn(field: &str, value: &str) -> String {
    match sub_label(field, value) {
        Some(cn) => cn.to_string(),
        None => format!("原始码：{value}"),
    }
}

/// 从记录取某封闭枚举字段的「值 token」：字符串原样；JSON null ⇒ "null"；
/// 缺失/其它类型 ⇒ None（调用方按「未携带该判别」处理）。
fn field_token(record: &Value, field: &str) -> Option<String> {
    match record.get(field) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) => Some("null".to_string()),
        _ => None,
    }
}

/// 把子枚举字段渲染成中文（未知值诚实回退）；字段缺失 ⇒ None。
fn field_cn(record: &Value, field: &str) -> Option<String> {
    field_token(record, field).map(|v| sub_cn(field, &v))
}

/// HTTP status 段 → (段 token, 该段默认级别)。段外（非 2xx/4xx/5xx）⇒ None。
fn http_band(status: i64) -> Option<(&'static str, ActivityLevel)> {
    match status {
        200..=299 => Some(("2xx", ActivityLevel::Info)),
        400..=499 => Some(("4xx", ActivityLevel::Warn)),
        500..=599 => Some(("5xx", ActivityLevel::Error)),
        _ => None,
    }
}

/// 透传槽取整数（status/attempt/activeRequests 等）。
fn num(record: &Value, field: &str) -> Option<i64> {
    record.get(field).and_then(Value::as_i64)
}

/// 透传槽取字符串（path 等半枚举位，原样搬运不翻译）。
fn text(record: &Value, field: &str) -> Option<String> {
    record.get(field).and_then(Value::as_str).map(str::to_string)
}

// ───────────────────────── 定级 + 详情提炼 ─────────────────────────

/// 据 `event` + 子枚举 refine 出最终级别与中文详情。
/// severity-依赖事件（sse_ended/finished/precommit_failure）在此覆盖 base_level（§4）。
fn refine(event: &str, base_level: ActivityLevel, record: &Value) -> (ActivityLevel, String) {
    match event {
        "gateway_started" => {
            let detail = match num(record, "pid") {
                Some(pid) => format!("网关监听就绪，进程 {pid}。"),
                None => "网关监听就绪。".to_string(),
            };
            (base_level, detail)
        }
        "gateway_draining" => {
            let restart = record.get("restart").and_then(Value::as_bool).unwrap_or(false);
            let active = num(record, "activeRequests").unwrap_or(0);
            let kind = if restart { "热重启排空" } else { "关停排空" };
            (base_level, format!("开始{kind}，等待 {active} 个在途请求完成。"))
        }
        "request_status" => {
            let detail = match field_cn(record, "phase") {
                Some(p) => format!("{p}。"),
                None => "在途请求状态快照。".to_string(),
            };
            (base_level, detail)
        }
        "received" => {
            let detail = match text(record, "path") {
                Some(p) => format!("收到客户端请求：{p}。"),
                None => "收到客户端请求。".to_string(),
            };
            (base_level, detail)
        }
        "attempt_started" => {
            let detail = match num(record, "attempt") {
                Some(n) => format!("第 {n} 次向上游发起尝试。"),
                None => "向上游发起尝试。".to_string(),
            };
            (base_level, detail)
        }
        "upstream_response" => {
            // 级别恒信息：是否成功由后续 finished / sse_ended 定（§1-B#7）；此处只在详情报 HTTP 段。
            let detail = match num(record, "status") {
                Some(s) => {
                    let band = http_band(s)
                        .map(|(b, _)| sub_cn("status_band", b))
                        .unwrap_or_else(|| "其它".to_string());
                    format!("收到上游响应头：HTTP {s}（{band}）。")
                }
                None => "收到上游响应头。".to_string(),
            };
            (base_level, detail)
        }
        "sse_committed" => (base_level, "已确认有真实内容，开始向客户端下发。".to_string()),
        "sse_ended" => {
            let tk = field_token(record, "terminalKind");
            let sr = field_token(record, "stallReason");
            let level = sse_ended_level(tk.as_deref(), sr.as_deref());
            let tk_cn = field_cn(record, "terminalKind").unwrap_or_else(|| "未知".to_string());
            let sr_cn = field_cn(record, "stallReason").unwrap_or_else(|| "未知".to_string());
            (level, format!("流式响应结束：终止类型 {tk_cn}，停滞 {sr_cn}。"))
        }
        "finished" => match num(record, "status") {
            Some(s) => match http_band(s) {
                Some((band, lvl)) => {
                    (lvl, format!("请求结束：HTTP {s}（{}）。", sub_cn("status_band", band)))
                }
                None => (base_level, format!("请求结束：HTTP {s}。")),
            },
            None => (base_level, "请求结束。".to_string()),
        },
        "client_closed" => (base_level, "客户端在响应完成前断开连接。".to_string()),
        "session_compat_refused" => {
            (base_level, "命中已知会话但内联历史不安全，无法无损剥离。".to_string())
        }
        "codex_session_passthrough" => {
            (base_level, "命中已知会话，codex 资源重试放行、原样透传。".to_string())
        }
        "session_compat_applied" => {
            let src = field_cn(record, "source").unwrap_or_else(|| "未知来源".to_string());
            (base_level, format!("已剥离会话 id / reasoning 项（来源：{src}）。"))
        }
        "session_compat_enabled" => {
            let src = field_cn(record, "source").unwrap_or_else(|| "未知来源".to_string());
            (base_level, format!("azure 修复时顺带登记会话兼容（来源：{src}）。"))
        }
        "status_repair_refused" => {
            let reason = field_cn(record, "reason").unwrap_or_else(|| "未知原因".to_string());
            (base_level, format!("拒绝删除 input.status 修复：{reason}。"))
        }
        "status_repair_applied" => {
            (base_level, "已删除被拒的 status 字段并重试。".to_string())
        }
        "item_error" => {
            let kind = field_cn(record, "kind").unwrap_or_else(|| "未知类别".to_string());
            let src = field_cn(record, "source").unwrap_or_else(|| "未知来源".to_string());
            let st = num(record, "status").map(|s| format!("，HTTP {s}")).unwrap_or_default();
            (base_level, format!("识别到 item 级错误：{kind}（来源：{src}{st}）。"))
        }
        "repair_refused" => {
            let kind = field_cn(record, "kind").unwrap_or_else(|| "未知类别".to_string());
            let refusal = field_cn(record, "refusal").unwrap_or_else(|| "未知原因".to_string());
            (base_level, format!("拒绝 item / 形状修复：{kind}；原因：{refusal}。"))
        }
        "repair_applied" => {
            let kind = field_cn(record, "kind").unwrap_or_else(|| "修复".to_string());
            (base_level, format!("已做 {kind} 并重试。"))
        }
        "codex_resource_retry" => {
            let kind = field_cn(record, "kind").unwrap_or_else(|| "资源错误".to_string());
            (base_level, format!("{kind}：保留历史、换通道重试。"))
        }
        "codex_resource_exhausted" => {
            let kind = field_cn(record, "kind").unwrap_or_else(|| "资源错误".to_string());
            let refusal = field_cn(record, "refusal").unwrap_or_else(|| "未知原因".to_string());
            (base_level, format!("{kind}：资源重试耗尽（{refusal}），放弃。"))
        }
        "precommit_failure" => {
            let level = match field_token(record, "kind").as_deref() {
                Some("stream_error") | Some("buffer_limit") => ActivityLevel::Error,
                Some("peek_timeout") | Some("empty_stream_drop") | Some("read_timeout")
                | Some("other") => ActivityLevel::Warn,
                _ => ActivityLevel::Warn, // 未知 kind：安全兜底警告
            };
            let kind_cn = field_cn(record, "kind").unwrap_or_else(|| "未知类别".to_string());
            (level, format!("流式下发前失败：{kind_cn}（换通道重试）。"))
        }
        _ => (base_level, String::new()),
    }
}

/// `sse_ended` 定级：先看下发后停滞（stallReason），无停滞再看终止类型（terminalKind）。
/// 依据 diagnostics-codes.md §4；未知值安全兜底（不崩、不误报为严重）。
fn sse_ended_level(terminal_kind: Option<&str>, stall_reason: Option<&str>) -> ActivityLevel {
    if let Some(sr) = stall_reason {
        match sr {
            "null" => {} // 无停滞 → 落到 terminalKind 判定
            "client_closed" => return ActivityLevel::Info, // 客户端主动断开
            "upstream_read_error" => return ActivityLevel::Error,
            "progress_timeout" | "transport_idle_timeout" | "event_size_limit" => {
                return ActivityLevel::Warn
            }
            _ => return ActivityLevel::Warn, // 未知停滞：安全兜底警告
        }
    }
    match terminal_kind {
        Some("response.completed") | Some("done_marker") | Some("message_stop")
        | Some("chat_finish_reason") => ActivityLevel::Info,
        Some("response.incomplete") => ActivityLevel::Warn,
        Some("response.failed") => ActivityLevel::Error,
        Some("null") | None => ActivityLevel::Warn, // 未见终止事件 = 流被截断
        Some(_) => ActivityLevel::Info, // 未知终止类型：有终止即视为信息（诚实回退）
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    // ────────── 权威清单（独立真值，来源 diagnostics-codes.md §1/§2/§5；file:line 见该文档） ──────────
    // 与 gateway_env 的 EXPECTED_17 同角色：契约测试把它与打包脚本只读抽取结果交叉核对。

    const EXPECTED_EVENTS: [&str; 23] = [
        "gateway_started", "gateway_draining", "request_status", "received", "request_profile",
        "attempt_started", "upstream_response", "sse_committed", "sse_ended", "finished",
        "client_closed", "session_compat_refused", "codex_session_passthrough",
        "session_compat_applied", "session_compat_enabled", "status_repair_refused",
        "status_repair_applied", "item_error", "repair_refused", "repair_applied",
        "codex_resource_retry", "codex_resource_exhausted", "precommit_failure",
    ];
    const EXPECTED_SOURCE: [&str; 6] = [
        "http_error", "sse_precommit", "json_error", "json_empty",
        "explicit_session", "resource_lineage",
    ];
    const EXPECTED_KIND: [&str; 10] = [
        "azure_resource_mismatch", "stale_reasoning_item", "other", "peek_timeout",
        "empty_stream_drop", "read_timeout", "buffer_limit", "stream_error",
        "empty_stream_shape", "empty_stream_reasoning",
    ];
    const EXPECTED_REFUSAL: [&str; 5] = [
        "unsafe_inline_history", "wrong_endpoint", "repair_exhausted",
        "time_budget", "resource_attempt_limit",
    ];
    // 含 "null"（JS null，非字面量——脚本字面量核对时跳过）
    const EXPECTED_TERMINAL_KIND: [&str; 7] = [
        "null", "done_marker", "response.completed", "response.failed",
        "response.incomplete", "message_stop", "chat_finish_reason",
    ];
    const EXPECTED_STALL_REASON: [&str; 6] = [
        "null", "progress_timeout", "client_closed", "transport_idle_timeout",
        "upstream_read_error", "event_size_limit",
    ];
    const EXPECTED_PHASE: [&str; 8] = [
        "receiving_request", "waiting_headers", "reading_body", "waiting_first_content",
        "stream_waiting", "reasoning", "tool_output", "text_output",
    ];
    const EXPECTED_STATUS_REPAIR_REASON: [&str; 6] = [
        "wrong_endpoint", "repair_exhausted", "unexpected_status",
        "invalid_json", "unsupported_item", "time_budget",
    ];
    const EXPECTED_STATUS_BAND: [&str; 3] = ["2xx", "4xx", "5xx"];

    // ────────── 打包脚本只读抽取（照 gateway_env↔bat：从真值源抽取再比对） ──────────

    /// 唯一诊断来源脚本（禁改，.gitattributes -text 锁字节）。
    fn mjs_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources").join("gateway").join("relay-retry-proxy.mjs")
    }

    /// 抽取 marker 之后到下一个单引号之间的字面量（逐个不重叠）。
    fn scan_quoted_after(text: &str, marker: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(rel) = text[from..].find(marker) {
            let start = from + rel + marker.len();
            match text[start..].find('\'') {
                Some(q) => {
                    out.push(text[start..start + q].to_string());
                    from = start + q + 1;
                }
                None => break,
            }
        }
        out
    }

    /// 从脚本抽取全部 event 码：`diagnose('X'` 的首参 + 3 处直发 `event: 'X'`。
    /// （diagnostics-codes.md §1：所有 event 均字符串字面量，无动态拼接 → 清单封闭完整。）
    fn extract_events(text: &str) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        for e in scan_quoted_after(text, "diagnose('") {
            set.insert(e);
        }
        for e in scan_quoted_after(text, "event: '") {
            set.insert(e);
        }
        set
    }

    /// 覆盖率契约（主闸门）：从只读打包脚本抽取的 event 码，必须恰为清单 23 项，
    /// 且字典（EVENT_BASE）对每一项都有中文条目。上游新增/删除事件码即红。
    #[test]
    fn 契约_打包脚本事件码恰为清单23项且字典全覆盖() {
        let text = std::fs::read_to_string(mjs_path()).expect("读取打包脚本失败（禁改，只读）");
        let scanned = extract_events(&text);
        let expected: BTreeSet<String> = EXPECTED_EVENTS.iter().map(|s| s.to_string()).collect();

        // 抽取自洽：脚本里的 event 码恰为清单（diagnostics-codes.md §1 的 23 项）
        assert_eq!(expected.len(), 23, "清单应恰 23 项");
        assert_eq!(
            scanned, expected,
            "打包脚本 event 码与清单不一致（上游改动？）。\n脚本独有：{:?}\n清单独有：{:?}",
            scanned.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&scanned).collect::<Vec<_>>()
        );

        // 字典全覆盖：每个脚本会发的 event 都有基表条目（非回退）
        for e in &scanned {
            assert!(event_base(e).is_some(), "字典缺事件码 `{e}` 的中文条目");
        }
    }

    /// 字典对清单每个封闭子枚举值都有中文标签；且每个值（null 除外）确为脚本中的字面量
    /// （上游重命名/删除子枚举值 → 其字面量消失 → 红）。
    #[test]
    fn 契约_字典覆盖全部子枚举值且与脚本字面量一致() {
        let text = std::fs::read_to_string(mjs_path()).expect("读取打包脚本失败");
        let groups: [(&str, &[&str]); 8] = [
            ("source", &EXPECTED_SOURCE),
            ("kind", &EXPECTED_KIND),
            ("refusal", &EXPECTED_REFUSAL),
            ("terminalKind", &EXPECTED_TERMINAL_KIND),
            ("stallReason", &EXPECTED_STALL_REASON),
            ("phase", &EXPECTED_PHASE),
            ("reason", &EXPECTED_STATUS_REPAIR_REASON),
            ("status_band", &EXPECTED_STATUS_BAND),
        ];
        for (field, values) in groups {
            for &v in values {
                // 字典覆盖（非回退）
                assert!(
                    sub_label(field, v).is_some(),
                    "字典缺子枚举 `{field}={v}` 的中文标签"
                );
                // 与脚本字面量一致（null 是 JS null、status_band 是壳侧派生段，均跳过字面量核对）
                if v != "null" && field != "status_band" {
                    let needle = format!("'{v}'");
                    assert!(
                        text.contains(needle.as_str()),
                        "子枚举值 `{field}={v}` 未作为字面量出现在打包脚本（上游改名？）"
                    );
                }
            }
        }
    }

    /// 字典键计数（可报告锚点）：23 事件基 + 51 子枚举标签 = 74；且无清单外多余条目。
    /// 51 = source6 + kind10 + refusal5 + terminalKind7 + stallReason6 + phase8 + reason6 + band3
    /// （前两项 23/48 即 diagnostics-codes.md 的精确锚点；band3 为壳侧派生的 HTTP 段）。
    #[test]
    fn 契约_字典键计数且无清单外多余条目() {
        assert_eq!(EVENT_BASE.len(), 23, "事件基表应恰 23 项");
        // 事件基表 ⊆ 清单（无多余事件）
        let expected_events: BTreeSet<&str> = EXPECTED_EVENTS.iter().copied().collect();
        for row in EVENT_BASE {
            assert!(expected_events.contains(row.0), "事件基表出现清单外事件 `{}`", row.0);
        }

        // 子枚举标签分字段计数，逐字段与清单条数一致（防多译/漏译）
        let count = |f: &str| SUB_LABELS.iter().filter(|t| t.0 == f).count();
        assert_eq!(count("source"), EXPECTED_SOURCE.len());
        assert_eq!(count("kind"), EXPECTED_KIND.len());
        assert_eq!(count("refusal"), EXPECTED_REFUSAL.len());
        assert_eq!(count("terminalKind"), EXPECTED_TERMINAL_KIND.len());
        assert_eq!(count("stallReason"), EXPECTED_STALL_REASON.len());
        assert_eq!(count("phase"), EXPECTED_PHASE.len());
        assert_eq!(count("reason"), EXPECTED_STATUS_REPAIR_REASON.len());
        assert_eq!(count("status_band"), EXPECTED_STATUS_BAND.len());

        assert_eq!(SUB_LABELS.len(), 51, "子枚举标签应恰 51 项");
        assert_eq!(EVENT_BASE.len() + SUB_LABELS.len(), 74, "字典键总数应为 74");
    }

    /// 真实样本（trimmed）：读 tests/fixtures/diagnostics-samples.jsonl，逐条断言译文/级别。
    fn load_fixture() -> Vec<Value> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests").join("fixtures").join("diagnostics-samples.jsonl");
        let text = std::fs::read_to_string(&path).expect("读取诊断样本 fixture 失败");
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("fixture 行应为合法 JSON"))
            .collect()
    }

    /// 按谓词取 fixture 里第一条匹配记录。
    fn pick<'a>(recs: &'a [Value], pred: impl Fn(&Value) -> bool) -> &'a Value {
        recs.iter().find(|&r| pred(r)).expect("fixture 应含该记录")
    }
    fn ev(r: &Value) -> &str {
        r.get("event").and_then(Value::as_str).unwrap_or("")
    }
    fn status_is(r: &Value, s: i64) -> bool {
        r.get("status").and_then(Value::as_i64) == Some(s)
    }

    #[test]
    fn 真实样本fixture_各记录译文级别正确() {
        use ActivityLevel::*;
        let recs = load_fixture();

        // 生命周期
        let a = localize_record(pick(&recs, |r| ev(r) == "gateway_started")).unwrap();
        assert_eq!((a.level, a.title.as_str()), (Info, "网关已启动"));
        let a = localize_record(pick(&recs, |r| ev(r) == "gateway_draining")).unwrap();
        assert_eq!(a.level, Info);
        assert!(a.detail.contains("热重启排空"), "restart=true → 热重启排空：{}", a.detail);

        // happy-path
        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "received")).unwrap().title, "收到请求");
        // request_profile 抑制为 None（received 的兄弟记录）
        assert!(localize_record(pick(&recs, |r| ev(r) == "request_profile")).is_none());
        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "attempt_started")).unwrap().title, "发起上游尝试");
        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "sse_committed")).unwrap().level, Info);

        let up200 = localize_record(pick(&recs, |r| ev(r) == "upstream_response" && status_is(r, 200))).unwrap();
        assert_eq!(up200.level, Info);
        assert!(up200.detail.contains("成功"), "{}", up200.detail);
        let up400 = localize_record(pick(&recs, |r| ev(r) == "upstream_response" && status_is(r, 400))).unwrap();
        assert_eq!(up400.level, Info, "upstream_response 级别恒信息（成败由 finished/sse_ended 定）");
        assert!(up400.detail.contains("请求被拒"), "{}", up400.detail);

        // sse_ended：三种 terminalKind/stallReason → 三种级别
        let done = localize_record(pick(&recs, |r| ev(r) == "sse_ended" && r.get("terminalKind").and_then(Value::as_str) == Some("response.completed"))).unwrap();
        assert_eq!(done.level, Info);
        assert!(done.detail.contains("正常完成"), "{}", done.detail);
        let failed = localize_record(pick(&recs, |r| ev(r) == "sse_ended" && r.get("terminalKind").and_then(Value::as_str) == Some("response.failed"))).unwrap();
        assert_eq!(failed.level, Error);
        assert!(failed.detail.contains("失败终止"), "{}", failed.detail);
        let closed = localize_record(pick(&recs, |r| ev(r) == "sse_ended" && r.get("stallReason").and_then(Value::as_str) == Some("client_closed"))).unwrap();
        assert_eq!(closed.level, Info, "stallReason=client_closed 客户端主动断 → 信息");
        assert!(closed.detail.contains("客户端断开"), "{}", closed.detail);

        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "client_closed")).unwrap().level, Warn);

        // finished：HTTP 段定级
        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "finished" && status_is(r, 200))).unwrap().level, Info);
        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "finished" && status_is(r, 400))).unwrap().level, Warn);
        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "finished" && status_is(r, 402))).unwrap().level, Warn);
        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "finished" && status_is(r, 503))).unwrap().level, Error);

        // request_status：phase 决定详情
        let rs_r = localize_record(pick(&recs, |r| ev(r) == "request_status" && r.get("phase").and_then(Value::as_str) == Some("reasoning"))).unwrap();
        assert!(rs_r.detail.contains("正在输出推理"), "{}", rs_r.detail);
        let rs_t = localize_record(pick(&recs, |r| ev(r) == "request_status" && r.get("phase").and_then(Value::as_str) == Some("tool_output"))).unwrap();
        assert!(rs_t.detail.contains("正在输出工具调用"), "{}", rs_t.detail);

        // 错误/修复路径
        let ie = localize_record(pick(&recs, |r| ev(r) == "item_error")).unwrap();
        assert_eq!(ie.level, Warn);
        assert!(ie.detail.contains("跨 Azure 资源引用"), "{}", ie.detail);
        let rr = localize_record(pick(&recs, |r| ev(r) == "repair_refused")).unwrap();
        assert_eq!(rr.level, Warn);
        assert!(rr.detail.contains("内联历史不安全"), "{}", rr.detail);
        assert_eq!(localize_record(pick(&recs, |r| ev(r) == "codex_resource_retry")).unwrap().level, Info);

        let exh_limit = localize_record(pick(&recs, |r| ev(r) == "codex_resource_exhausted" && r.get("refusal").and_then(Value::as_str) == Some("resource_attempt_limit"))).unwrap();
        assert_eq!(exh_limit.level, Error);
        assert!(exh_limit.detail.contains("资源尝试达上限"), "{}", exh_limit.detail);
        let exh_unsafe = localize_record(pick(&recs, |r| ev(r) == "codex_resource_exhausted" && r.get("refusal").and_then(Value::as_str) == Some("unsafe_inline_history"))).unwrap();
        assert_eq!(exh_unsafe.level, Error);

        let pf = localize_record(pick(&recs, |r| ev(r) == "precommit_failure")).unwrap();
        assert_eq!(pf.level, Error, "kind=stream_error → 错误");
        assert!(pf.detail.contains("流内携带错误"), "{}", pf.detail);
    }

    /// 样本未触发的 7 个事件（手造最小记录，字段形状照 diagnostics-codes.md §1/§2）。
    #[test]
    fn 未采样事件_手造记录_译文级别正确() {
        use ActivityLevel::*;
        let base = |ev: Value| localize_record(&ev).unwrap();

        let r = base(json!({"time":"t","event":"session_compat_refused","reason":"unsafe_inline_history"}));
        assert_eq!(r.level, Warn);

        let r = base(json!({"time":"t","event":"codex_session_passthrough","reason":"unsafe_inline_history"}));
        assert_eq!(r.level, Info);

        let r = base(json!({"time":"t","event":"session_compat_applied","source":"explicit_session","removedIds":2}));
        assert_eq!(r.level, Info);
        assert!(r.detail.contains("显式会话"), "{}", r.detail);
        let r = base(json!({"time":"t","event":"session_compat_applied","source":"resource_lineage"}));
        assert!(r.detail.contains("资源血缘"), "{}", r.detail);

        let r = base(json!({"time":"t","event":"session_compat_enabled","kind":"azure","source":"explicit_session"}));
        assert_eq!(r.level, Info);

        // status_repair_refused：reason 全 6 值均有译文且为警告
        for reason in EXPECTED_STATUS_REPAIR_REASON {
            let r = base(json!({"time":"t","event":"status_repair_refused","source":"http_error","status":400,"reason":reason}));
            assert_eq!(r.level, Warn, "status_repair_refused reason={reason}");
            assert!(sub_label("reason", reason).is_some());
            assert!(!r.detail.contains("原始码"), "reason={reason} 应已汉化：{}", r.detail);
        }

        let r = base(json!({"time":"t","event":"status_repair_applied","source":"http_error","status":400,"index":3}));
        assert_eq!(r.level, Info);

        // repair_applied（样本里从未触发）：空回应形状修复
        let r = base(json!({"time":"t","event":"repair_applied","kind":"empty_stream_shape","source":"json_empty","status":200,"removedIds":4}));
        assert_eq!(r.level, Info);
        assert!(r.detail.contains("空回应形状修复"), "{}", r.detail);
    }

    /// 样本未触发的子枚举值（kind/source/refusal/terminalKind/stallReason/phase）全覆盖、不回退。
    #[test]
    fn 未采样子枚举值_均已汉化不回退() {
        // item/peek/shape 全部 kind
        for k in EXPECTED_KIND {
            let r = localize_record(&json!({"time":"t","event":"item_error","kind":k,"source":"json_error","status":200})).unwrap();
            assert!(!r.detail.contains("原始码"), "kind={k} 未汉化：{}", r.detail);
        }
        // 全部 phase
        for p in EXPECTED_PHASE {
            let r = localize_record(&json!({"time":"t","event":"request_status","phase":p})).unwrap();
            assert!(!r.detail.contains("原始码"), "phase={p} 未汉化：{}", r.detail);
        }
        // 全部 source（经 item_error 详情体现）
        for s in EXPECTED_SOURCE {
            assert!(sub_label("source", s).is_some(), "source={s}");
        }
        // 全部 refusal（经 repair_refused）
        for rf in EXPECTED_REFUSAL {
            let r = localize_record(&json!({"time":"t","event":"repair_refused","kind":"stale_reasoning_item","source":"http_error","status":400,"refusal":rf})).unwrap();
            assert_eq!(r.level, ActivityLevel::Warn);
            assert!(!r.detail.contains("原始码"), "refusal={rf} 未汉化：{}", r.detail);
        }
    }

    /// sse_ended 定级：覆盖全部 terminalKind × stallReason 的代表组合（§4）。
    #[test]
    fn sse_ended_定级覆盖terminalKind与stallReason() {
        use ActivityLevel::*;
        let lvl = |tk: Value, sr: Value| {
            localize_record(&json!({"time":"t","event":"sse_ended","terminalKind":tk,"stallReason":sr})).unwrap().level
        };
        // 无停滞：按 terminalKind
        assert_eq!(lvl(json!("response.completed"), json!(null)), Info);
        assert_eq!(lvl(json!("done_marker"), json!(null)), Info);
        assert_eq!(lvl(json!("message_stop"), json!(null)), Info);
        assert_eq!(lvl(json!("chat_finish_reason"), json!(null)), Info);
        assert_eq!(lvl(json!("response.incomplete"), json!(null)), Warn);
        assert_eq!(lvl(json!("response.failed"), json!(null)), Error);
        assert_eq!(lvl(json!(null), json!(null)), Warn, "未见终止事件=截断");
        // 有停滞：停滞决定级别（压过 terminalKind）
        assert_eq!(lvl(json!(null), json!("progress_timeout")), Warn);
        assert_eq!(lvl(json!(null), json!("transport_idle_timeout")), Warn);
        assert_eq!(lvl(json!(null), json!("upstream_read_error")), Error);
        assert_eq!(lvl(json!(null), json!("event_size_limit")), Warn);
        assert_eq!(lvl(json!("response.completed"), json!("client_closed")), Info);
    }

    /// finished 定级：2xx 信息 / 4xx 警告 / 5xx 错误（§4）。
    #[test]
    fn finished_定级按http段() {
        use ActivityLevel::*;
        let lvl = |s: i64| localize_record(&json!({"time":"t","event":"finished","status":s})).unwrap().level;
        assert_eq!(lvl(200), Info);
        assert_eq!(lvl(400), Warn);
        assert_eq!(lvl(402), Warn);
        assert_eq!(lvl(429), Warn);
        assert_eq!(lvl(502), Error);
        assert_eq!(lvl(503), Error);
    }

    /// precommit_failure 定级：按 kind（§4：stream_error/buffer_limit=错误，其余=警告）。
    #[test]
    fn precommit_failure_定级按kind() {
        use ActivityLevel::*;
        let lvl = |k: &str| localize_record(&json!({"time":"t","event":"precommit_failure","source":"sse_precommit","kind":k})).unwrap().level;
        assert_eq!(lvl("peek_timeout"), Warn);
        assert_eq!(lvl("empty_stream_drop"), Warn);
        assert_eq!(lvl("read_timeout"), Warn);
        assert_eq!(lvl("other"), Warn);
        assert_eq!(lvl("stream_error"), Error);
        assert_eq!(lvl("buffer_limit"), Error);
    }

    /// 未知事件码：诚实回退，原样显示原始码，不隐藏、不 panic。
    #[test]
    fn 未知事件码_诚实回退显示原始码() {
        let r = localize_record(&json!({"time":"2026-10-11T00:00:00Z","event":"brand_new_upstream_code","foo":1})).unwrap();
        assert_eq!(r.level, ActivityLevel::Info);
        assert!(r.title.contains("brand_new_upstream_code"), "回退标题须含原始码：{}", r.title);
        assert_eq!(r.time, "2026-10-11T00:00:00Z", "时间透传");
    }

    /// 已知事件 + 未知子枚举值：不 panic，详情回退「原始码：X」，级别安全兜底。
    #[test]
    fn 未知子枚举值_诚实回退不崩() {
        // sse_ended 未知 terminalKind（无停滞）→ 有终止即信息（诚实回退）
        let r = localize_record(&json!({"time":"t","event":"sse_ended","terminalKind":"response.brand_new","stallReason":null})).unwrap();
        assert_eq!(r.level, ActivityLevel::Info);
        assert!(r.detail.contains("原始码：response.brand_new"), "{}", r.detail);
        // precommit_failure 未知 kind → 兜底警告
        let r = localize_record(&json!({"time":"t","event":"precommit_failure","kind":"mystery_kind"})).unwrap();
        assert_eq!(r.level, ActivityLevel::Warn);
        assert!(r.detail.contains("原始码：mystery_kind"), "{}", r.detail);
        // item_error 未知 source → 详情回退，不崩
        let r = localize_record(&json!({"time":"t","event":"item_error","kind":"azure_resource_mismatch","source":"new_layer","status":400})).unwrap();
        assert!(r.detail.contains("原始码：new_layer"), "{}", r.detail);
    }

    /// None 语义：无 event / event 非字符串 / request_profile / 非法行。
    #[test]
    fn 无event字段_或request_profile_返回None() {
        assert!(localize_record(&json!({"time":"t","path":"/x"})).is_none(), "无 event → None");
        assert!(localize_record(&json!({"time":"t","event":123})).is_none(), "event 非字符串 → None");
        assert!(localize_record(&json!({"time":"t","event":"request_profile","client":"codex"})).is_none(), "request_profile 抑制");
        assert!(localize_line("not a json line").is_none(), "非法 JSON 行 → None");
        assert!(localize_line("").is_none());
    }

    /// 透传槽：时间 / 路径 / 次数等原样搬运、绝不翻译。
    #[test]
    fn 透传槽原样保留_数字与时间不翻译() {
        let r = localize_record(&json!({
            "time":"2026-10-11T11:49:17.160Z","event":"attempt_started",
            "path":"/v1/responses","attempt":7,"repairCount":0
        })).unwrap();
        assert_eq!(r.time, "2026-10-11T11:49:17.160Z");
        assert!(r.detail.contains("第 7 次"), "attempt 数字透传：{}", r.detail);

        // 级别序列化为稳定小写机器码；中文标签经 as_cn 取用
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"level\":\"info\""), "{json}");
        assert_eq!(ActivityLevel::Warn.as_cn(), "警告");
    }
}
