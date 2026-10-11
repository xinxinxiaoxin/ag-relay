//! 状态总览（T3 工单）：读网关本地控制端点 `/_relay/status`，产出前端四卡快照。
//!
//! 分层：解析与格式化全部是纯函数（时钟/HTTP 都可注入，cargo test 无运行时可测）；
//! 网络只有一个薄函数 fetch_status_body；Tauri 命令 get_status 再薄封装一层。
//!
//! 网关实况（relay-retry-proxy.mjs 第 77-78、1342-1351 行）：
//!   - `startedAt` 是 ISO 8601 UTC 字符串（`new Date().toISOString()`）；
//!   - `loadedVersion` 是网关脚本自身的 sha256（64 位 hex）；
//!   - `activeRequests` 是数字；status JSON 没有 `available` 字段——
//!     「运行状态」卡由壳判定：端点可达且响应可解析 ⇒ 运行中，否则未运行。
//!
//! 容错语义（工单验收第 4 条）：
//!   - 端点不可达 / 非 200 / 响应体不是完整 JSON 对象 ⇒ 整体离线（四卡「未运行/—」）；
//!   - JSON 合法但某字段缺失或类型不对 ⇒ 仅该卡为 None（前端显示「未知」）。

use crate::timeutil::{days_from_civil, system_time_ms};

/// 把网关 `startedAt`（ISO 8601 UTC，如 `2026-10-11T08:30:00.000Z`）换算成 epoch 毫秒。
/// 格式不符 / 越界日期一律 None（调用方按「未知」处理）。只认 Z 结尾的 UTC。
pub fn parse_iso8601_utc_ms(s: &str) -> Option<i64> {
    // 形如 YYYY-MM-DDTHH:MM:SS[.fff][Z]；网关恒发 toISOString() 的 24 字符形式
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let seg = s.get(r)?;
        if seg.is_empty() || !seg.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        seg.parse::<i64>().ok()
    };
    let year = num(0..4)?;
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let month = num(5..7)?;
    let day = num(8..10)?;
    // 分隔符容忍 T/t/空格（ISO 允许空格；网关恒为 T）
    if !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let hour = num(11..13)?;
    let minute = num(14..16)?;
    let second = num(17..19)?;

    if !(1..=12).contains(&month) {
        return None;
    }
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    // 可选小数秒（截断到毫秒）
    let mut frac_ms: i64 = 0;
    let mut tail = 19;
    if b.get(19) == Some(&b'.') {
        let start = 20;
        let mut end = start;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
        if end == start {
            return None;
        }
        let first3 = &s[start..end.min(start + 3)];
        let padded = format!("{:0<3}", first3);
        frac_ms = padded.parse::<i64>().ok()?;
        tail = end;
    }
    // 结尾必须是 Z（或省略=UTC）；±hh:mm 时区偏移不支持，宁「未知」不错算
    match b.get(tail) {
        None => {}
        Some(&b'Z') | Some(&b'z') => {}
        Some(_) => return None,
    }

    let days = days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3600 + minute * 60 + second) * 1000 + frac_ms)
}

/// 平年各月天数（2 月由闰年规则补正）。
fn days_in_month(year: i64, month: i64) -> i64 {
    const PER_MONTH: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut d = PER_MONTH[(month - 1) as usize];
    if month == 2 && is_leap_year(year) {
        d = 29;
    }
    d
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// 由 startedAt 与当前时刻换算「x小时y分」；不足 1 小时显示「y分钟」。
/// startedAt 在未来（时钟偏差）按 0 计；非法输入返回 None（前端显示「未知」）。
pub fn format_uptime(started_at_ms: i64, now_ms: i64) -> Option<String> {
    if started_at_ms <= 0 {
        return None;
    }
    let elapsed_ms = (now_ms - started_at_ms).max(0);
    let total_minutes = elapsed_ms / 60_000;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    Some(if hours > 0 {
        format!("{hours}小时{minutes}分")
    } else {
        format!("{minutes}分钟")
    })
}

/// 前端四卡快照（serde 输出 camelCase，与网关 status 字段风格一致）。
/// available=false ⇒ 端点不可达/响应不可解析，面板整体显示「未运行」；
/// available=true 时某字段为 None ⇒ 该卡显示「未知」。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    pub available: bool,
    /// loadedVersion 前 8 位短哈希
    pub version: Option<String>,
    pub active_requests: Option<u64>,
    /// 「x小时y分」/「y分钟」
    pub uptime_text: Option<String>,
}

impl StatusSnapshot {
    /// 离线快照：网关不可达时的四卡值。
    pub fn offline() -> Self {
        Self { available: false, version: None, active_requests: None, uptime_text: None }
    }
}

/// 宽松解析 `/_relay/status` 响应体（以当前系统时间为「现在」）。
/// 响应体不是完整 JSON 对象 ⇒ None（调用方按离线处理）；
/// JSON 合法但字段缺失/类型不对 ⇒ 仅该字段为 None。
pub fn parse_status_snapshot(body: &str) -> Option<StatusSnapshot> {
    parse_status_snapshot_with_now(body, system_time_ms())
}

/// 同上，但「现在」由调用方注入（测试不依赖真实时钟）。
pub fn parse_status_snapshot_with_now(body: &str, now_ms: i64) -> Option<StatusSnapshot> {
    // 必须是完整 JSON 对象：半截 JSON / 非 JSON 一律按离线处理
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let obj = v.as_object()?;

    let version = obj
        .get("loadedVersion")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(short_version);
    let active_requests = obj.get("activeRequests").and_then(serde_json::Value::as_u64);
    let uptime_text = obj
        .get("startedAt")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_iso8601_utc_ms)
        .and_then(|started| format_uptime(started, now_ms));

    Some(StatusSnapshot { available: true, version, active_requests, uptime_text })
}

/// 版本短哈希：前 8 位（不足 8 位原样）。
fn short_version(s: &str) -> String {
    s.chars().take(8).collect()
}

/// 从 `/_relay/status` 响应体提取网关进程 pid（外部停止链 F1 的起点）。
/// status 顶层的 `pid` 是网关脚本自身的 process.pid（relay-retry-proxy.mjs:1344），
/// 它由 supervisor fork 出来——其父进程即 supervisor。
/// 非 JSON / 无 pid / pid 不是正整数 → None（调用方按「不可识别」处理）。
pub fn parse_status_pid(body: &str) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let pid = v.as_object()?.get("pid")?.as_u64()?;
    u32::try_from(pid).ok().filter(|p| *p != 0)
}

/// 本地状态端点的连接/读超时。端点应答为毫秒级；前端轮询 3 秒一次，
/// 超时取小值避免个别挂起拖住轮询节奏。
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// GET http://{host}:{port}/_relay/status，返回响应体文本。
/// 网关本地端点要求 Host 头精确匹配端口（否则 403）、带 Origin 的跨站请求直接拒——
/// 所以用 HTTP/1.0 + 仅 Host 头（与 T2 get_relay_status 同一手法，避免 chunked）。
pub fn fetch_status_body(host: &str, port: u16) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};

    let addr_pair = format!("{host}:{port}");
    let addr = addr_pair
        .to_socket_addrs()
        .map_err(|e| format!("解析 {addr_pair} 失败：{e}"))?
        .next()
        .ok_or_else(|| format!("地址 {addr_pair} 无可用套接字地址"))?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| format!("连接 {addr_pair} 失败：{e}"))?;
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|e| format!("设置读超时失败：{e}"))?;
    let request = format!("GET /_relay/status HTTP/1.0\r\nHost: {host}:{port}\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("发送请求失败：{e}"))?;
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .map_err(|e| format!("读取响应失败：{e}"))?;
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("响应缺少分隔符：{:.80}", raw))?;
    if !(head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200")) {
        return Err(format!("状态接口返回异常：{}", head.lines().next().unwrap_or("")));
    }
    Ok(body.to_string())
}

/// 命令 get_status 的同步实现：不可达/非 200/响应不可解析一律返回离线快照，
/// 绝不 panic、绝不把网络错误抛给前端（面板只做状态展示，不弹错）。
pub fn snapshot_via_http(host: &str, port: u16) -> StatusSnapshot {
    match fetch_status_body(host, port) {
        Ok(body) => parse_status_snapshot(&body).unwrap_or_else(StatusSnapshot::offline),
        Err(_) => StatusSnapshot::offline(),
    }
}

/// Tauri 命令：前端每 3 秒轮询，刷新四张状态卡。
/// 永不失败（返回值即完整快照）；8787 上已有网关（含面板外启动的实例）经此直接显示。
/// 放 spawn_blocking：本地端点最坏 1+2 秒超时，不占异步运行时的工作线程。
#[tauri::command]
pub async fn get_status() -> StatusSnapshot {
    tauri::async_runtime::spawn_blocking(|| {
        snapshot_via_http(crate::gateway_process::GATEWAY_HOST, crate::gateway_process::GATEWAY_PORT)
    })
    .await
    .unwrap_or_else(|_| StatusSnapshot::offline())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    // ------------------------------------------------------------------
    // 时间换算（独立真值：node -e "new Date(...).getTime()"）
    // ------------------------------------------------------------------

    #[test]
    fn iso8601_标准zulu格式转epoch毫秒_含闰年与边界() {
        // node -e "new Date('2026-10-11T08:30:00.000Z').getTime()" → 1791707400000
        assert_eq!(parse_iso8601_utc_ms("2026-10-11T08:30:00.000Z"), Some(1_791_707_400_000));
        // 无毫秒形式（网关恒带 .sss，但容忍省略）
        assert_eq!(parse_iso8601_utc_ms("1970-01-01T00:00:00Z"), Some(0));
        // 闰年 2 月 29 日（node → 951825600000）
        assert_eq!(parse_iso8601_utc_ms("2000-02-29T12:00:00Z"), Some(951_825_600_000));
        // 毫秒位数多于 3 位时截断到毫秒
        assert_eq!(parse_iso8601_utc_ms("1970-01-01T00:00:00.123456Z"), Some(123));
    }

    #[test]
    fn iso8601_畸形或越界输入返回None() {
        assert_eq!(parse_iso8601_utc_ms(""), None);
        assert_eq!(parse_iso8601_utc_ms("not-a-date"), None);
        assert_eq!(parse_iso8601_utc_ms("2026-10-11"), None, "缺时间部分");
        assert_eq!(parse_iso8601_utc_ms("2026-13-01T00:00:00Z"), None, "月越界");
        assert_eq!(parse_iso8601_utc_ms("2026-02-30T00:00:00Z"), None, "日超出当月天数");
        assert_eq!(parse_iso8601_utc_ms("2026-10-11T25:00:00Z"), None, "时越界");
        assert_eq!(parse_iso8601_utc_ms("2026-10-11T08:99:00Z"), None, "分越界");
        assert_eq!(parse_iso8601_utc_ms("2026-10-11T08:00:60Z"), None, "秒越界");
        // 时区偏移形式不支持（网关恒为 Z）：宁可显示「未知」也不错算
        assert_eq!(parse_iso8601_utc_ms("2026-10-11T08:30:00+08:00"), None);
    }

    #[test]
    fn 时长_不足一小时显示y分钟() {
        let now = 1_791_707_400_000;
        assert_eq!(format_uptime(now - 5 * 60_000, now), Some("5分钟".to_string()));
        assert_eq!(format_uptime(now - 1, now), Some("0分钟".to_string()), "刚启动不足 1 分钟");
        assert_eq!(format_uptime(now - 59 * 60_000, now), Some("59分钟".to_string()));
    }

    #[test]
    fn 时长_满一小时起显示x小时y分() {
        let now = 1_791_707_400_000;
        assert_eq!(format_uptime(now - 60 * 60_000, now), Some("1小时0分".to_string()));
        assert_eq!(format_uptime(now - (2 * 3600 + 5 * 60) * 1000, now), Some("2小时5分".to_string()));
        assert_eq!(format_uptime(now - 26 * 3600_000, now), Some("26小时0分".to_string()), "跨天照常累计小时");
    }

    #[test]
    fn 时长_时钟偏差与非法输入的兜底() {
        let now = 1_791_707_400_000;
        // startedAt 在未来（宿主机时钟偏差）：按 0 计，不出现负数文本
        assert_eq!(format_uptime(now + 30_000, now), Some("0分钟".to_string()));
        assert_eq!(format_uptime(0, now), None, "startedAt 无效值");
        assert_eq!(format_uptime(-5, now), None);
    }

    // ------------------------------------------------------------------
    // 宽松解析：四卡快照
    // ------------------------------------------------------------------

    /// 独立真值：与 T2 冒烟测试同款的网关脚本 sha256（loadedVersion 的真实形状）
    const VERSION_FULL: &str =
        "e8395e0f90fe46bf1059bfcbac77e65d887d09d4ed5057490f85847e5cde85dc";

    /// 网关 status 响应体形状（relay-retry-proxy.mjs 第 1344-1351 行的子集，
    /// 未涉及字段原样保留）。
    fn full_body() -> String {
        format!(
            r#"{{"loadedVersion":"{VERSION_FULL}","startedAt":"2026-10-11T08:30:00.000Z","pid":123,"supervisorEnabled":true,"restarting":false,"draining":false,"activeRequests":2,"config":{{}},"requests":[]}}"#
        )
    }

    #[test]
    fn 解析_正常返回_四卡齐全() {
        // now = startedAt + 2小时5分30秒 → 「2小时5分」
        let now = 1_791_707_400_000 + (2 * 3600 + 5 * 60 + 30) * 1000;
        let snap = parse_status_snapshot_with_now(&full_body(), now).expect("正常体应可解析");
        assert!(snap.available);
        assert_eq!(snap.version.as_deref(), Some(&VERSION_FULL[..8]), "版本显示前 8 位短哈希");
        assert_eq!(snap.active_requests, Some(2));
        assert_eq!(snap.uptime_text.as_deref(), Some("2小时5分"));
    }

    #[test]
    fn 解析_字段缺失_该卡未知_其余正常() {
        let body = r#"{"loadedVersion":"abcd1234ffff","pid":1}"#;
        let snap = parse_status_snapshot_with_now(body, 1_000_000).expect("合法 JSON 应可解析");
        assert!(snap.available);
        assert_eq!(snap.version.as_deref(), Some("abcd1234"));
        assert_eq!(snap.active_requests, None, "activeRequests 缺失 → 未知");
        assert_eq!(snap.uptime_text, None, "startedAt 缺失 → 未知");
    }

    #[test]
    fn 解析_类型错乱_对应卡未知_互不牵连() {
        let body = format!(
            r#"{{"loadedVersion":123,"startedAt":null,"activeRequests":"很多","draining":false}}"#
        );
        let snap = parse_status_snapshot_with_now(&body, 1_000_000).expect("合法 JSON 应可解析");
        assert!(snap.available);
        assert_eq!(snap.version, None, "版本是数字 → 未知");
        assert_eq!(snap.uptime_text, None, "startedAt 是 null → 未知");
        assert_eq!(snap.active_requests, None, "在途数是字符串 → 未知");
    }

    #[test]
    fn 解析_版本不足8位_原样显示() {
        let body = r#"{"loadedVersion":"abc"}"#;
        let snap = parse_status_snapshot_with_now(body, 0).expect("应可解析");
        assert_eq!(snap.version.as_deref(), Some("abc"));
    }

    #[test]
    fn 解析_半截json_整体按离线处理() {
        assert_eq!(parse_status_snapshot_with_now(r#"{"loadedVersion":"e83"#, 0), None);
        assert_eq!(parse_status_snapshot_with_now("", 0), None);
    }

    #[test]
    fn 解析_合法但非对象_整体按离线处理() {
        assert_eq!(parse_status_snapshot_with_now("null", 0), None);
        assert_eq!(parse_status_snapshot_with_now("[1,2]", 0), None);
        assert_eq!(parse_status_snapshot_with_now("\"gateway\"", 0), None);
    }

    #[test]
    fn 解析_时区偏移形式的startedAt_该卡未知_其余正常() {
        let body = r#"{"loadedVersion":"abcd1234","startedAt":"2026-10-11T16:30:00+08:00","activeRequests":0}"#;
        let snap = parse_status_snapshot_with_now(body, 1_000_000).expect("应可解析");
        assert_eq!(snap.uptime_text, None, "非 Z 形式不猜时区 → 未知");
        assert_eq!(snap.version.as_deref(), Some("abcd1234"));
        assert_eq!(snap.active_requests, Some(0));
    }

    // ------------------------------------------------------------------
    // 外部停止链起点：status → 网关 pid（F1）
    // ------------------------------------------------------------------

    #[test]
    fn pid解析_正常体取pid_畸形体一律None() {
        assert_eq!(parse_status_pid(&full_body()), Some(123), "full_body 里的 pid");
        assert_eq!(parse_status_pid(r#"{"pid":7}"#), Some(7));
        // pid 0 不是合法进程 id（System Idle）→ 不可识别
        assert_eq!(parse_status_pid(r#"{"pid":0}"#), None);
        // 缺字段 / 类型不对 / 非 JSON / 半截 JSON
        assert_eq!(parse_status_pid(r#"{"loadedVersion":"abc"}"#), None);
        assert_eq!(parse_status_pid(r#"{"pid":null}"#), None);
        assert_eq!(parse_status_pid(r#"{"pid":"123"}"#), None);
        assert_eq!(parse_status_pid(r#"{"pid":-5}"#), None);
        // u64 超出 u32（防溢出误杀无关 pid）
        assert_eq!(parse_status_pid(r#"{"pid":99999999999}"#), None);
        assert_eq!(parse_status_pid("not json"), None);
        assert_eq!(parse_status_pid(r#"{"pid":12"#), None);
    }

    // ------------------------------------------------------------------
    // HTTP 接缝：mock 本地随机端口服务（只应答一次，返回捕获到的请求文本）
    // ------------------------------------------------------------------

    /// 起 mock HTTP 服务：bind 127.0.0.1:0（随机端口），收一个连接、读请求、
    /// 写 canned 响应后关闭。返回 (端口, 捕获请求的 JoinHandle)。
    fn spawn_mock(response: String) -> (u16, std::thread::JoinHandle<Option<String>>) {
        use std::io::{Read as _, Write as _};
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock bind 随机端口");
        let port = listener.local_addr().expect("local_addr").port();
        let handle = std::thread::spawn(move || {
            let mut captured = None;
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                captured = Some(String::from_utf8_lossy(&buf[..n]).into_owned());
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            } // 连接随 drop 关闭（配合 HTTP/1.0 请求 read_to_string 到 EOF）
            captured
        });
        (port, handle)
    }

    fn http200(body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body)
    }

    /// 找一个「绑定后释放、连接必被拒」的端口（避免端口被复用的偶发，最多换 5 个）。
    fn find_refused_port() -> u16 {
        for _ in 0..5 {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
            let port = listener.local_addr().expect("local_addr").port();
            drop(listener);
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
                return port;
            }
        }
        panic!("连续 5 次找不到「连接被拒」的端口，环境异常");
    }

    #[test]
    fn fetch_正常返回_请求行与host头符合网关本地端点要求() {
        let (port, join) = spawn_mock(http200(&full_body()));
        let body = fetch_status_body("127.0.0.1", port).expect("200 响应应成功");
        let captured = join.join().expect("mock 线程").expect("应捕获到请求");
        assert!(captured.starts_with("GET /_relay/status HTTP/1.0"), "请求行：{captured:?}");
        assert!(captured.contains(&format!("Host: 127.0.0.1:{port}\r\n")), "Host 必须带精确端口，否则网关 403");
        assert!(!captured.contains("Origin"), "带 Origin 会被网关当跨站 403");
        assert_eq!(body, full_body());
    }

    #[test]
    fn fetch_连接被拒_报错() {
        let port = find_refused_port();
        let err = fetch_status_body("127.0.0.1", port).expect_err("无监听端口应报错");
        assert!(!err.is_empty());
    }

    #[test]
    fn fetch_非200响应_报错() {
        let (port, join) = spawn_mock("HTTP/1.1 500 Internal Server Error\r\n\r\n".to_string());
        assert!(fetch_status_body("127.0.0.1", port).is_err(), "非 200 必须报错");
        let _ = join.join();
    }

    #[test]
    fn 命令体_正常返回_映射为四卡快照() {
        let (port, join) = spawn_mock(http200(&full_body()));
        let snap = snapshot_via_http("127.0.0.1", port);
        let _ = join.join();
        assert!(snap.available);
        assert_eq!(snap.version.as_deref(), Some(&VERSION_FULL[..8]));
        assert_eq!(snap.active_requests, Some(2));
        assert!(snap.uptime_text.is_some(), "合法 startedAt 相对当前时钟必能换算出时长");
    }

    #[test]
    fn 命令体_连接被拒_离线快照() {
        let port = find_refused_port();
        assert_eq!(snapshot_via_http("127.0.0.1", port), StatusSnapshot::offline());
    }

    #[test]
    fn 命令体_半截json_离线快照() {
        // 200 但响应体被截断：fetch 成功、解析失败 → 整体按未运行显示
        let (port, join) = spawn_mock(http200(r#"{"loadedVersion":"e83"#));
        assert_eq!(snapshot_via_http("127.0.0.1", port), StatusSnapshot::offline());
        let _ = join.join();
    }

    #[test]
    fn 命令体_非200_离线快照() {
        let (port, join) = spawn_mock("HTTP/1.1 403 Forbidden\r\n\r\n".to_string());
        assert_eq!(snapshot_via_http("127.0.0.1", port), StatusSnapshot::offline());
        let _ = join.join();
    }
}
