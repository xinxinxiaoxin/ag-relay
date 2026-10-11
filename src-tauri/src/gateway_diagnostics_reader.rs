//! 诊断读取层（T5 工单）：壳侧只读消费者的**文件 I/O + 查询**层。
//!
//! 分层（沿用 backend-map「纯逻辑在 gateway_*」铁律）：本模块的读取/查询核心是**纯函数**，
//! 可在临时文件上脱离 tauri 运行时 `cargo test`；两条 `#[tauri::command]` 只做薄封装。
//! 「一条诊断记录 → 可选活动事件」的提炼复用 T4 的 `gateway_diagnostics::localize_line`，本模块不重做。
//!
//! 三件事（ticket #14 验收）：
//! 1. 增量 tail 读取器 `TailReader`：记 live 文件字节偏移，每次 `poll` 只读新增的**完整**行，
//!    感知轮转/截断（live 变短 ⇒ 先从 `.1` 捞回漏读尾部、再从新 live 头读）。
//! 2. 有界最近缓冲：`TailReader` 内存只保留最近 `RECENT_CAPACITY` 条活动事件，供总览「最近活动」。
//! 3. 从文件尾向前的分页懒读 `query_activity_in`：诊断日志页用，带级别筛选 + 关键词搜索。
//!
//! 只读、绝不触碰网关进程（不 spawn/不发信号，只读文件），不拖慢网关。

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::gateway_diagnostics::{localize_line, ActivityEvent, ActivityLevel};
// 诊断文件名的单一真值在 gateway_process（与 resolve_paths 同源），本模块复用以防两处漂移。
use crate::gateway_process::DIAGNOSTICS_FILE_NAME;

/// 「最近活动」内存缓冲容量（只保留最近这么多条活动事件）。
const RECENT_CAPACITY: usize = 50;
/// 分页默认页大小。
const DEFAULT_PAGE_SIZE: usize = 50;
/// 分页页大小上限（防前端请求过大页，保护懒读与内存）。
const MAX_PAGE_SIZE: usize = 500;
/// 反向分块读取块大小（64 KiB）。
const REVERSE_CHUNK: usize = 64 * 1024;

// ───────────────────────── 查询 IPC 形状（命令 ② 的请求/响应） ─────────────────────────

/// 级别筛选（诊断日志页「全部 / 信息 / 警告 / 错误」）。serde 小写，与 ActivityLevel 对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LevelFilter {
    /// 全部（不筛级别）
    #[default]
    All,
    Info,
    Warn,
    Error,
}

/// 分页查询参数。前端 `invoke('query_activity_events', { query: { level, keyword, page, pageSize } })`，
/// 字段 camelCase；全部可缺省（level=all、keyword=空、page=0、pageSize=默认）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityQuery {
    #[serde(default)]
    pub level: LevelFilter,
    #[serde(default)]
    pub keyword: String,
    #[serde(default)]
    pub page: usize,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page_size() -> usize {
    DEFAULT_PAGE_SIZE
}

/// 分页查询结果（返回前端；字段 camelCase）。`events` 最新在前；`hasMore` 表示本页之后还有匹配。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub events: Vec<ActivityEvent>,
    pub page: usize,
    pub page_size: usize,
    pub has_more: bool,
}

// ───────────────────────── 路径派生（纯函数） ─────────────────────────

/// 由 live 诊断文件路径派生轮转旧档路径（追加 `.1`）：
/// `relay-diagnostics.jsonl` → `relay-diagnostics.jsonl.1`。
pub fn rotated_path_of(live: &Path) -> PathBuf {
    match live.file_name() {
        Some(name) => {
            let mut rotated = name.to_os_string();
            rotated.push(".1");
            live.with_file_name(rotated)
        }
        None => live.to_path_buf(),
    }
}

/// 文件字节长度；文件不存在 / 不可读 ⇒ 0。
fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

// ───────────────────────── 增量 tail 读取器（有状态：偏移 + 有界最近缓冲） ─────────────────────────

/// 增量 tail 读取器：记 live 文件已消费的字节偏移，每次 `poll` 只读自偏移以来新增的**完整**行，
/// 感知轮转/截断。内存只留最近 `capacity` 条活动事件（有界环形缓冲）。只读，绝不触碰网关进程。
pub struct TailReader {
    live: PathBuf,
    rotated: PathBuf,
    offset: u64,
    recent: VecDeque<ActivityEvent>,
    capacity: usize,
}

impl TailReader {
    pub fn new(live: PathBuf, capacity: usize) -> Self {
        let rotated = rotated_path_of(&live);
        Self {
            live,
            rotated,
            offset: 0,
            recent: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// 已消费的 live 字节偏移（测试 / 诊断用）。
    pub fn byte_offset(&self) -> u64 {
        self.offset
    }

    /// 增量轮询：只读自上次偏移以来新增的完整行，感知轮转。返回本次新增的活动事件数。
    pub fn poll(&mut self) -> usize {
        let mut added = 0;
        if file_len(&self.live) < self.offset {
            // live 变短 = 轮转或截断：先从 .1 捞回漏读尾部（best-effort），再从头读新 live。
            added += self.drain_rotated_tail();
            self.offset = 0;
        }
        added += self.drain_live();
        added
    }

    /// 最近活动事件快照，**最新在前**（供总览「最近活动」）。
    pub fn recent_newest_first(&self) -> Vec<ActivityEvent> {
        self.recent.iter().rev().cloned().collect()
    }

    fn push_event(&mut self, ev: ActivityEvent) {
        if self.recent.len() >= self.capacity {
            self.recent.pop_front();
        }
        self.recent.push_back(ev);
    }

    /// 读 live[offset..]，消费到最后一个换行为止（保留未完成的尾行），推进 offset。
    fn drain_live(&mut self) -> usize {
        let (consumed, events) = read_new_complete_lines(&self.live, self.offset);
        self.offset += consumed;
        let n = events.len();
        for ev in events {
            self.push_event(ev);
        }
        n
    }

    /// 轮转时从 .1 的 [offset..] 捞回漏读尾部（.1 对齐旧 live 时才有意义；不对齐则放弃，不崩）。
    fn drain_rotated_tail(&mut self) -> usize {
        if file_len(&self.rotated) < self.offset {
            return 0; // .1 不含我们的旧尾部（已被覆盖 / 不对齐）：接受缺口，best-effort
        }
        let (_, events) = read_new_complete_lines(&self.rotated, self.offset);
        let n = events.len();
        for ev in events {
            self.push_event(ev);
        }
        n
    }
}

/// 从 `path` 的 `from` 偏移起读取**完整行**（到最后一个 `\n`），逐行 `localize_line`。
/// 返回 (本次消费的字节数, 活动事件列表)。文件缺失 / 不可读 ⇒ (0, 空)。
/// 未完成的尾行（无结尾 `\n`）不消费、留待下次——这是增量 tail 的关键（不处理半截行）。
fn read_new_complete_lines(path: &Path, from: u64) -> (u64, Vec<ActivityEvent>) {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (0, Vec::new()),
    };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return (0, Vec::new());
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return (0, Vec::new());
    }
    let last_nl = match buf.iter().rposition(|&b| b == b'\n') {
        Some(p) => p,
        None => return (0, Vec::new()), // 尚无完整行
    };
    let mut events = Vec::new();
    for line in buf[..=last_nl].split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(line);
        if let Some(ev) = localize_line(text.trim()) {
            events.push(ev);
        }
    }
    ((last_nl as u64) + 1, events)
}

// ───────────────────────── 分页懒读（无状态：从尾向前反向读 live→.1） ─────────────────────────

/// 分页查询（纯函数）：从 live 尾向前、再接 .1 尾向前（全局最新→最旧）反向懒读，
/// `localize_line` 后按 level + keyword 过滤，切出第 `page` 页（页大小 `page_size`），**最新在前**。
/// 懒读：填满本页并多探到「下一条匹配」即停读，不扫全量（无筛选时首页只读文件尾部一小段）。
pub fn query_activity_in(live: &Path, rotated: &Path, query: &ActivityQuery) -> ActivityPage {
    let page_size = query.page_size.clamp(1, MAX_PAGE_SIZE);
    let skip = query.page.saturating_mul(page_size);
    let end = skip.saturating_add(page_size); // 本页匹配下标区间 [skip, end)
    let mut events: Vec<ActivityEvent> = Vec::new();
    let mut seen = 0usize;
    let mut has_more = false;

    let mut consume = |line: &str| -> bool {
        // 返回 true 继续、false 停读
        let Some(ev) = localize_line(line) else {
            return true;
        };
        if !event_matches(&ev, query) {
            return true;
        }
        if seen >= end {
            has_more = true; // 已越过本页末尾 → 确认还有更多
            return false;
        }
        if seen >= skip {
            events.push(ev);
        }
        seen += 1;
        true
    };

    // 全局最新在 live 尾部：先反向读 live；未被 consume 叫停则接着反向读 .1。
    if for_each_line_rev(live, &mut consume) {
        for_each_line_rev(rotated, &mut consume);
    }

    ActivityPage {
        events,
        page: query.page,
        page_size,
        has_more,
    }
}

/// 级别 + 关键词匹配。关键词对 title+detail 做大小写不敏感子串匹配（空关键词恒真）。
fn event_matches(ev: &ActivityEvent, query: &ActivityQuery) -> bool {
    let level_ok = match query.level {
        LevelFilter::All => true,
        LevelFilter::Info => ev.level == ActivityLevel::Info,
        LevelFilter::Warn => ev.level == ActivityLevel::Warn,
        LevelFilter::Error => ev.level == ActivityLevel::Error,
    };
    if !level_ok {
        return false;
    }
    let needle = query.keyword.trim();
    if needle.is_empty() {
        return true;
    }
    let needle = needle.to_lowercase();
    ev.title.to_lowercase().contains(&needle) || ev.detail.to_lowercase().contains(&needle)
}

/// 反向逐行读 `path`（最新行在前），对每行调用 `f`；`f` 返回 false 即停。
/// 返回 true 表示整文件读完（调用方可继续下一个文件），false 表示 `f` 要求停。
/// 文件缺失 / 读错 ⇒ 视作无行、返回 true（继续）。
fn for_each_line_rev(path: &Path, f: &mut impl FnMut(&str) -> bool) -> bool {
    let mut file = match File::open(path) {
        Ok(opened) => opened,
        Err(_) => return true,
    };
    let mut pos = match file.seek(SeekFrom::End(0)) {
        Ok(p) => p,
        Err(_) => return true,
    };
    // leftover：已读区中最靠前、尚未定界完成的行片段（其更早字节还在 pos 之前未读的块里）。
    let mut leftover: Vec<u8> = Vec::new();
    while pos > 0 {
        let read_size = std::cmp::min(REVERSE_CHUNK as u64, pos) as usize;
        pos -= read_size as u64;
        if file.seek(SeekFrom::Start(pos)).is_err() {
            return true;
        }
        let mut block = vec![0u8; read_size];
        if file.read_exact(&mut block).is_err() {
            return true;
        }
        block.extend_from_slice(&leftover); // block=更早字节，leftover=更后字节 ⇒ 拼成连续区间
        match block.iter().position(|&b| b == b'\n') {
            Some(idx) => {
                // idx 之后是完整行（可能多行），逆序发出；idx 之前（不含 \n）留作 leftover。
                if !emit_lines_rev(&block[idx + 1..], &mut *f) {
                    return false;
                }
                leftover = block[..idx].to_vec();
            }
            None => leftover = block, // 整块无 \n：并入 leftover，继续向前
        }
    }
    // 到文件头：leftover 即最早的一行
    if !leftover.is_empty() {
        let text = String::from_utf8_lossy(&leftover);
        let t = text.trim();
        if !t.is_empty() && !f(t) {
            return false;
        }
    }
    true
}

/// 把一段「完整行」字节块（以 `\n` 分隔）按**逆序**发给 `f`（最后一行先发）。
/// 返回 false 表示 `f` 要求停。空段（连续 `\n` 或结尾 `\n`）跳过。
fn emit_lines_rev(bytes: &[u8], f: &mut impl FnMut(&str) -> bool) -> bool {
    let lines: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
    for line in lines.into_iter().rev() {
        if line.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(line);
        let t = text.trim();
        if t.is_empty() {
            continue;
        }
        if !f(t) {
            return false;
        }
    }
    true
}

// ───────────────────────── 托管状态 + 命令层（薄封装；逻辑在上面的纯函数 / 读取器） ─────────────────────────

/// 壳的诊断读取状态：持有增量 tail 读取器（首次命令调用时惰性初始化——那时才能从
/// AppHandle 解析 %APPDATA% 路径）。只读，绝不触碰网关进程。
#[derive(Default)]
pub struct DiagnosticsState {
    reader: Mutex<Option<TailReader>>,
}

/// 由 AppHandle 解析 (live, rotated) 诊断文件路径。
/// logs 目录知识复用 `gateway_control::logs_dir`（与「打开日志文件夹」/ 网关落位同一真值）。
fn resolve_diagnostics_paths(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("解析应用数据目录失败：{e}"))?;
    let live = crate::gateway_control::logs_dir(&app_data).join(DIAGNOSTICS_FILE_NAME);
    let rotated = rotated_path_of(&live);
    Ok((live, rotated))
}

/// 命令 ①：取最近活动事件（供总览「最近活动」）。
/// 惰性初始化 tail 读取器并增量轮询，返回有界最近缓冲（**最新在前**）。
/// 路径不可解析等异常返回空列表而非报错——「暂无活动」是合理的用户态（诊断读取不崩不阻塞）。
#[tauri::command]
pub async fn get_recent_activity(
    app: AppHandle,
    state: State<'_, DiagnosticsState>,
) -> Result<Vec<ActivityEvent>, String> {
    let mut guard = state.reader.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        match resolve_diagnostics_paths(&app) {
            Ok((live, _)) => {
                let mut reader = TailReader::new(live, RECENT_CAPACITY);
                reader.poll(); // 首次：读入现有 live 尾部，点亮最近活动
                *guard = Some(reader);
            }
            Err(_) => return Ok(Vec::new()),
        }
    }
    let reader = guard.as_mut().expect("reader 已惰性初始化");
    reader.poll();
    Ok(reader.recent_newest_first())
}

/// 命令 ②：分页查询活动事件（供诊断日志页；级别筛选 + 关键词搜索）。
/// 从文件尾向前懒读 live→.1，返回译后活动事件（最新在前）。重 I/O 放阻塞线程池，不占命令执行器。
#[tauri::command]
pub async fn query_activity_events(app: AppHandle, query: ActivityQuery) -> ActivityPage {
    let empty = |q: &ActivityQuery| ActivityPage {
        events: Vec::new(),
        page: q.page,
        page_size: q.page_size.clamp(1, MAX_PAGE_SIZE),
        has_more: false,
    };
    let (live, rotated) = match resolve_diagnostics_paths(&app) {
        Ok(p) => p,
        Err(_) => return empty(&query), // 路径不可解析：空页
    };
    tauri::async_runtime::spawn_blocking(move || query_activity_in(&live, &rotated, &query))
        .await
        .unwrap_or_else(|_| ActivityPage {
            events: Vec::new(),
            page: 0,
            page_size: DEFAULT_PAGE_SIZE,
            has_more: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write as _;

    /// 每个测试独享临时目录（进程 id + 唯一 tag，避免并行工单 / 测试互撞；镜像 gateway_control 约定）。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ag-relay-t5-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    fn write_file(path: &Path, content: &str) {
        let mut f = fs::File::create(path).expect("建文件");
        f.write_all(content.as_bytes()).expect("写文件");
        f.flush().expect("flush");
    }

    fn append_file(path: &Path, content: &str) {
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect("打开追加");
        f.write_all(content.as_bytes()).expect("追加");
        f.flush().expect("flush");
    }

    fn query(level: LevelFilter, keyword: &str, page: usize, page_size: usize) -> ActivityQuery {
        ActivityQuery {
            level,
            keyword: keyword.to_string(),
            page,
            page_size,
        }
    }

    // 真实形状的诊断记录行（取自 diagnostics-samples-digest.md §5，裁剪透传槽）。
    const L_STARTED: &str = r#"{"time":"2026-10-11T03:31:06.961Z","event":"gateway_started","loadedVersion":"77934ca7","pid":26368,"supervisorEnabled":true}"#;
    const L_RECEIVED: &str = r#"{"time":"2026-10-11T11:49:17.153Z","requestId":"34d904b7","event":"received","path":"/v1/responses","attempt":0}"#;
    const L_FIN_200: &str = r#"{"time":"2026-10-11T11:49:16.213Z","requestId":"8e60e791","event":"finished","path":"/v1/responses","attempt":1,"status":200}"#;
    const L_FIN_503: &str = r#"{"time":"2026-10-11T03:26:36.919Z","requestId":"0c3987c3","event":"finished","path":"/v1/responses","attempt":50,"status":503}"#;
    const L_ITEM_ERR: &str = r#"{"time":"2026-10-11T03:25:11.905Z","requestId":"0c3987c3","event":"item_error","path":"/v1/responses","attempt":34,"kind":"azure_resource_mismatch","source":"http_error","status":400}"#;
    const L_PROFILE: &str = r#"{"time":"2026-10-11T11:49:17.156Z","requestId":"34d904b7","event":"request_profile","path":"/v1/responses","client":"codex"}"#;

    fn received_line(time: &str, path: &str) -> String {
        format!(r#"{{"time":"{time}","event":"received","path":"{path}","attempt":0}}"#)
    }

    #[test]
    fn 轮转路径_追加点1后缀() {
        let live = Path::new("C:/x/ag-relay/logs/relay-diagnostics.jsonl");
        let rotated = rotated_path_of(live);
        assert_eq!(
            rotated.file_name().unwrap(),
            std::ffi::OsStr::new("relay-diagnostics.jsonl.1")
        );
        assert_eq!(rotated.parent(), live.parent(), "目录不变，仅文件名加 .1");
    }

    #[test]
    fn 诊断文件名_单一真值_钉死在上游固定落位名() {
        // 读取层复用 gateway_process 的 DIAGNOSTICS_FILE_NAME（import），此处把它钉死在
        // 上游网关固定落位的 live 诊断文件名上：resolve_paths 与读取层由此共享同一真值，
        // 任一侧若改回各写字面量而漂移即红（防 finding E 回归）。
        assert_eq!(DIAGNOSTICS_FILE_NAME, "relay-diagnostics.jsonl");
    }

    #[test]
    fn 增量tail_偏移推进_只读新增字节() {
        let dir = temp_dir("tail-inc");
        let live = dir.join("relay-diagnostics.jsonl");
        write_file(&live, &format!("{L_STARTED}\n{L_RECEIVED}\n"));
        let mut reader = TailReader::new(live.clone(), 10);

        assert_eq!(reader.poll(), 2, "首轮应读到 2 条事件");
        assert_eq!(reader.byte_offset(), file_len(&live), "偏移应推进到文件末尾");
        let off1 = reader.byte_offset();

        // 无变化再轮询：0 新增，偏移不动（只读新增字节）
        assert_eq!(reader.poll(), 0, "无新增应为 0");
        assert_eq!(reader.byte_offset(), off1);

        // 追加 1 行：只读新追加那一条
        append_file(&live, &format!("{L_FIN_200}\n"));
        assert_eq!(reader.poll(), 1, "应只读到新追加的 1 条");
        assert_eq!(reader.byte_offset(), file_len(&live));
        assert_eq!(reader.recent_newest_first().len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 增量tail_未完成尾行_暂不消费待补全() {
        let dir = temp_dir("tail-partial");
        let live = dir.join("relay-diagnostics.jsonl");
        // 一条完整 + 一条未完成（无结尾换行）
        write_file(&live, &format!("{L_STARTED}\n{{\"event\":\"recei"));
        let mut reader = TailReader::new(live.clone(), 10);
        assert_eq!(reader.poll(), 1, "只应消费完整的那一行");
        assert_eq!(
            reader.byte_offset(),
            (L_STARTED.len() + 1) as u64,
            "偏移应停在第一行换行之后，不含未完成尾行"
        );
        // 补全尾行（第二行变完整）
        append_file(&live, "ved\",\"time\":\"t\",\"path\":\"/x\"}\n");
        assert_eq!(reader.poll(), 1, "尾行补全后应读到第二条");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 轮转_live变短_从旧档捞回尾部并读新档() {
        let dir = temp_dir("rotate");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);

        write_file(&live, &format!("{L_STARTED}\n{L_RECEIVED}\n"));
        let mut reader = TailReader::new(live.clone(), 20);
        assert_eq!(reader.poll(), 2);
        let off = reader.byte_offset();

        // 追加一行到 live 但不 poll：轮转后它将成为「漏读尾部」
        append_file(&live, &format!("{L_ITEM_ERR}\n"));
        // 轮转：当前 live 整体改名为 .1（含 3 行，前 2 行已读、第 3 行未读），新建更短的 live（1 行）
        fs::rename(&live, &rotated).expect("rename 轮转");
        write_file(&live, &format!("{L_FIN_503}\n"));
        assert!(file_len(&live) < off, "新 live 应短于旧偏移，才触发轮转检测");

        let before = reader.recent_newest_first().len();
        let n = reader.poll();
        assert_eq!(n, 2, "应捞回漏读尾部(item_error)+读新档(finished-503)，实际 {n}");
        assert_eq!(reader.byte_offset(), file_len(&live), "偏移应重置并推进到新 live 末尾");
        let evs = reader.recent_newest_first();
        assert_eq!(evs.len(), before + 2);
        assert_eq!(evs[0].level, ActivityLevel::Error, "最新在前：新 live 的 503 为错误");
        assert!(evs[0].detail.contains("503"), "{}", evs[0].detail);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 截断_live清零后继续读新内容_不崩() {
        let dir = temp_dir("truncate");
        let live = dir.join("relay-diagnostics.jsonl");
        write_file(&live, &format!("{L_STARTED}\n{L_RECEIVED}\n"));
        let mut reader = TailReader::new(live.clone(), 10);
        assert_eq!(reader.poll(), 2);
        // 截断为空（无 .1）
        write_file(&live, "");
        assert_eq!(reader.poll(), 0, "变短+空内容：0 新增，不崩");
        assert_eq!(reader.byte_offset(), 0, "偏移应归零");
        // 写入新内容
        write_file(&live, &format!("{L_FIN_503}\n"));
        assert_eq!(reader.poll(), 1, "归零后应从头读到新内容");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 分页_从尾向前_跨live和旧档边界_页边界与最新在前() {
        let dir = temp_dir("paging");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        // 7 条 received：前 4 条（/req/0..3）放 .1（更旧），后 3 条（/req/4..6）放 live（更新）
        let mut older = String::new();
        for i in 0..4 {
            older.push_str(&received_line(&format!("t{i}"), &format!("/req/{i}")));
            older.push('\n');
        }
        let mut newer = String::new();
        for i in 4..7 {
            newer.push_str(&received_line(&format!("t{i}"), &format!("/req/{i}")));
            newer.push('\n');
        }
        write_file(&rotated, &older);
        write_file(&live, &newer);

        // 全局最新→最旧应为 /req/6,5,4,3,2,1,0
        let p0 = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 3));
        assert_eq!(p0.events.len(), 3);
        assert!(p0.events[0].detail.contains("/req/6"), "最新在前：{}", p0.events[0].detail);
        assert!(p0.events[2].detail.contains("/req/4"));
        assert!(p0.has_more, "后面还有 4 条");

        // 第 1 页跨越 live/.1 边界：/req/3,2,1
        let p1 = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 1, 3));
        assert!(p1.events[0].detail.contains("/req/3"), "{}", p1.events[0].detail);
        assert!(p1.events[2].detail.contains("/req/1"));
        assert!(p1.has_more, "还有 /req/0");

        // 第 2 页：仅剩 /req/0
        let p2 = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 2, 3));
        assert_eq!(p2.events.len(), 1);
        assert!(p2.events[0].detail.contains("/req/0"));
        assert!(!p2.has_more, "已到最旧");

        // 越界页：空
        let p3 = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 3, 3));
        assert!(p3.events.is_empty());
        assert!(!p3.has_more);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 级别筛选与关键词搜索() {
        let dir = temp_dir("filter");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live); // 不建 .1
        // 混合级别：started(info) received(info) item_error(warn) finished-503(error) finished-200(info)
        write_file(
            &live,
            &format!("{L_STARTED}\n{L_RECEIVED}\n{L_ITEM_ERR}\n{L_FIN_503}\n{L_FIN_200}\n"),
        );
        let big = 100;
        assert_eq!(
            query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, big)).events.len(),
            5
        );
        let errs = query_activity_in(&live, &rotated, &query(LevelFilter::Error, "", 0, big));
        assert_eq!(errs.events.len(), 1, "仅 finished-503 为错误");
        assert!(errs.events[0].detail.contains("503"));
        assert_eq!(
            query_activity_in(&live, &rotated, &query(LevelFilter::Warn, "", 0, big)).events.len(),
            1,
            "仅 item_error 为警告"
        );
        assert_eq!(
            query_activity_in(&live, &rotated, &query(LevelFilter::Info, "", 0, big)).events.len(),
            3
        );
        // 关键词：搜 azure（item_error 详情含「跨 Azure 资源引用」，大小写不敏感）
        let kw = query_activity_in(&live, &rotated, &query(LevelFilter::All, "azure", 0, big));
        assert_eq!(kw.events.len(), 1);
        assert_eq!(kw.events[0].level, ActivityLevel::Warn);
        // 关键词+级别：http 且仅错误 → finished-503（item_error 虽含 http 但为警告，被排除）
        let kw2 = query_activity_in(&live, &rotated, &query(LevelFilter::Error, "http", 0, big));
        assert_eq!(kw2.events.len(), 1);
        assert!(kw2.events[0].detail.contains("503"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 畸形与未知行_诚实跳过不崩_读取器与查询两接缝() {
        let dir = temp_dir("malformed");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        // 含：空行×2、垃圾非 JSON、无 event、request_profile(抑制)、未知码、以及 3 条可本地化
        let content = format!(
            "{L_STARTED}\n\n   \nnot a json line\n{{\"no_event\":true}}\n{L_PROFILE}\n{{\"time\":\"t\",\"event\":\"mystery_future_code\"}}\n{L_FIN_200}\n"
        );
        write_file(&live, &content);

        // 读取器接缝
        let mut reader = TailReader::new(live.clone(), 50);
        let n = reader.poll();
        assert_eq!(n, 3, "仅 started/mystery(回退)/finished-200 可本地化，其余诚实跳过，实际 {n}");
        let evs = reader.recent_newest_first();
        assert!(
            evs.iter().any(|e| e.title.contains("mystery_future_code")),
            "未知码应原样回退显示标题"
        );

        // 查询接缝（同一文件）：同样 3 条，最新在前（finished-200）
        let page = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 50));
        assert_eq!(page.events.len(), 3);
        assert_eq!(page.events[0].title, "请求结束", "最新在前");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 有界缓冲_只保留最近容量条_最新在前() {
        let dir = temp_dir("bounded");
        let live = dir.join("relay-diagnostics.jsonl");
        let mut content = String::new();
        for i in 0..10 {
            content.push_str(&received_line(&format!("t{i}"), &format!("/req/{i}")));
            content.push('\n');
        }
        write_file(&live, &content);
        let mut reader = TailReader::new(live.clone(), 3); // 容量 3
        reader.poll();
        let evs = reader.recent_newest_first();
        assert_eq!(evs.len(), 3, "有界：只留最近 3 条");
        assert!(evs[0].detail.contains("/req/9"), "最新在前：{}", evs[0].detail);
        assert!(evs[2].detail.contains("/req/7"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 查询_缺省请求_serde_camelcase解析与页回显() {
        // 前端可只传部分字段：{} / 完整对象
        let q: ActivityQuery = serde_json::from_str("{}").unwrap();
        assert!(matches!(q.level, LevelFilter::All));
        assert_eq!(q.keyword, "");
        assert_eq!(q.page, 0);
        assert_eq!(q.page_size, DEFAULT_PAGE_SIZE, "pageSize 缺省应取默认");

        let q: ActivityQuery =
            serde_json::from_str(r#"{"level":"warn","keyword":"azure","page":2,"pageSize":10}"#)
                .unwrap();
        assert!(matches!(q.level, LevelFilter::Warn));
        assert_eq!((q.keyword.as_str(), q.page, q.page_size), ("azure", 2, 10));

        // ActivityPage 序列化为 camelCase（T6 绑定依据）
        let page = ActivityPage {
            events: vec![],
            page: 1,
            page_size: 10,
            has_more: true,
        };
        let s = serde_json::to_string(&page).unwrap();
        assert!(s.contains("\"pageSize\":10"), "{s}");
        assert!(s.contains("\"hasMore\":true"), "{s}");
    }
}






