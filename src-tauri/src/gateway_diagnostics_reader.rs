//! 诊断文件只读服务：文件身份 + 完整行偏移的增量读取、首次快照、有界最近缓冲与分页懒读。
//! 原生通知由 gateway_activity 驱动本模块，不在这里定时轮询。
//! 快照和查询均捕获文件句柄/长度；轮转补读旧句柄与尚存的中间 .1，再接新 live。
//! 核心只保留一份 .1；若消费者停顿到某个从未打开的中间世代已被删除，无法凭空恢复它。
//! I/O 失败向调用方返回明确错误，翻译复用 gateway_diagnostics::localize_line。
//! 不写诊断文件，不触碰任何网关控制接口。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::gateway_diagnostics::{localize_line, ActivityEvent, ActivityLevel};
// 诊断文件名的单一真值在 gateway_process（与 resolve_paths 同源），本模块复用以防两处漂移。
use crate::gateway_process::DIAGNOSTICS_FILE_NAME;

/// 「最近活动」事件条数上限（复用分页服务的首页大小）。
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

/// 临时文件测试断言用；生产读取不吞 I/O 错误。
#[cfg(test)]
fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

// ───────────────────────── 增量 tail 读取器（有状态：偏移 + 有界最近缓冲） ─────────────────────────

/// 增量 tail 读取器：记 live 文件已消费的字节偏移，每次 `poll` 只读自偏移以来新增的**完整**行，
/// 感知轮转/截断。只读，绝不触碰网关进程。「最近活动」的内容查询走 `query_activity_in`（分页服务），
/// 本读取器只负责推进偏移并报告新增条数，不保留内存缓冲。
pub struct TailReader {
    live: PathBuf,
    rotated: PathBuf,
    offset: u64,
    current: Option<OpenedDiagnostic>,
    archive_identity: Option<same_file::Handle>,
}

impl TailReader {
    pub fn new(live: PathBuf) -> Self {
        let rotated = rotated_path_of(&live);
        Self {
            live,
            rotated,
            offset: 0,
            current: None,
            archive_identity: None,
        }
    }

    /// 建立首次快照基线；返回快照起点已存在的完整行（仅供测试/诊断校验，生产忽略其内容）。
    /// 后续 poll 仅报告快照之后的新活动事件。
    pub fn snapshot(&mut self) -> Result<Vec<ActivityEvent>, String> {
        let (mut live, mut rotated) = open_pair(&self.live, &self.rotated)?;
        let mut events = Vec::new();
        let mut old_offset = 0;
        if let Some(old) = &mut rotated {
            let (consumed, older) = read_new_complete_lines(&mut old.file, 0, old.len)?;
            old_offset = consumed;
            events.extend(older);
        }
        let mut offset = 0;
        if let Some(current) = &mut live {
            let (consumed, newer) = read_new_complete_lines(&mut current.file, 0, current.len)?;
            offset = consumed;
            events.extend(newer);
        }
        // 若正处于 rename→create 间隙，继续持有旧档以接续，而非下次重读旧档。
        self.offset = if live.is_some() { offset } else { old_offset };
        if live.is_some() {
            self.archive_identity = rotated.map(|old| old.identity);
            self.current = live;
        } else {
            self.archive_identity = None;
            self.current = rotated;
        }
        Ok(events)
    }

    /// 已消费的 live 字节偏移（测试 / 诊断用）。
    pub fn byte_offset(&self) -> u64 {
        self.offset
    }

    /// 增量轮询：只读自上次偏移以来新增的完整行，感知轮转。返回本次新增的活动事件数。
    pub fn poll(&mut self) -> Result<usize, String> {
        let (mut next, mut archived) = open_pair(&self.live, &self.rotated)?;
        let replaced = match (&self.current, &next) {
            (Some(old), Some(new)) => old.identity != new.identity,
            (None, Some(_)) => true,
            _ => false,
        };
        if !replaced {
            return self.drain_live();
        }

        // 一次接续全部读成功才推进偏移；失败重试不会漏掉已补读但还未通知的旧尾部。
        let mut events = Vec::new();
        if let Some(old) = &mut self.current {
            let len = old.file.metadata().map_err(read_error)?.len();
            let from = if len < self.offset { 0 } else { self.offset };
            events.extend(read_new_complete_lines(&mut old.file, from, len)?.1);
            if let Some(middle) = &mut archived {
                if middle.identity != old.identity
                    && self.archive_identity.as_ref() != Some(&middle.identity)
                {
                    events.extend(read_new_complete_lines(&mut middle.file, 0, middle.len)?.1);
                }
            }
        }
        let current = next.as_mut().expect("replaced 时新文件存在");
        let (consumed, newer) = read_new_complete_lines(&mut current.file, 0, current.len)?;
        events.extend(newer);
        self.current = next;
        self.archive_identity = archived.map(|old| old.identity);
        self.offset = consumed;
        Ok(events.len())
    }

    /// 读 live[offset..]，消费到最后一个换行为止（保留未完成的尾行），推进 offset。
    fn drain_live(&mut self) -> Result<usize, String> {
        let Some(current) = &mut self.current else {
            return Ok(0);
        };
        let len = current.file.metadata().map_err(read_error)?.len();
        if len < self.offset {
            // 同一文件原地截断；不能把无关的 .1 当成旧文件补读。
            self.offset = 0;
        }
        let (consumed, events) = read_new_complete_lines(&mut current.file, self.offset, len)?;
        self.offset += consumed;
        Ok(events.len())
    }
}

/// 同时持有可读句柄和文件身份（Windows 卷号/文件索引；Unix 设备/inode）。
/// 句柄默认允许共享删除，读取不妨碍核心 rename 轮转。
struct OpenedDiagnostic {
    file: File,
    identity: same_file::Handle,
    len: u64,
}

/// 打开同一轮转世代的只读快照。跨两次 open 的 rename 不能导致查询重复读取同一文件。
fn open_pair(
    live: &Path,
    rotated: &Path,
) -> Result<(Option<OpenedDiagnostic>, Option<OpenedDiagnostic>), String> {
    for _ in 0..4 {
        let current = OpenedDiagnostic::open(live)?;
        let older = OpenedDiagnostic::open(rotated)?;
        let check = OpenedDiagnostic::open(live)?;
        let stable = match (&current, &check) {
            (None, None) => true,
            (Some(a), Some(b)) => a.identity == b.identity,
            _ => false,
        };
        let duplicate =
            matches!((&current, &older), (Some(a), Some(b)) if a.identity == b.identity);
        if stable && !duplicate {
            return Ok((current, older));
        }
    }
    Err("诊断文件正在轮转，暂时无法取得一致快照，请重试。".into())
}

fn read_error(error: std::io::Error) -> String {
    format!(
        "读取诊断记录失败（{:?}），请检查文件访问权限后重试。",
        error.kind()
    )
}

impl OpenedDiagnostic {
    fn open(path: &Path) -> Result<Option<Self>, String> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(read_error(e)),
        };
        let metadata = file.metadata().map_err(read_error)?;
        if !metadata.is_file() {
            return Err("读取诊断记录失败：目标不是普通文件。".into());
        }
        let identity = same_file::Handle::from_file(file.try_clone().map_err(read_error)?)
            .map_err(read_error)?;
        Ok(Some(Self {
            file,
            identity,
            len: metadata.len(),
        }))
    }
}

/// 只读取捕获长度内的完整行，半行留在偏移之后等待补全。
fn read_new_complete_lines(
    file: &mut File,
    from: u64,
    end: u64,
) -> Result<(u64, Vec<ActivityEvent>), String> {
    file.seek(SeekFrom::Start(from)).map_err(read_error)?;
    let mut buf = Vec::new();
    file.take(end.saturating_sub(from))
        .read_to_end(&mut buf)
        .map_err(read_error)?;
    let last_nl = match buf.iter().rposition(|&b| b == b'\n') {
        Some(p) => p,
        None => return Ok((0, Vec::new())),
    };
    let mut events = Vec::new();
    for line in buf[..=last_nl].split(|&b| b == b'\n') {
        if let Some(ev) = localize_line(String::from_utf8_lossy(line).trim()) {
            events.push(ev);
        }
    }
    Ok(((last_nl as u64) + 1, events))
}

// ───────────────────────── 分页懒读（无状态：从尾向前反向读 live→.1） ─────────────────────────

/// 分页查询（纯函数）：从 live 尾向前、再接 .1 尾向前（全局最新→最旧）反向懒读，
/// `localize_line` 后按 level + keyword 过滤，切出第 `page` 页（页大小 `page_size`），**最新在前**。
/// 懒读：填满本页并多探到「下一条匹配」即停读，不扫全量（无筛选时首页只读文件尾部一小段）。
pub fn query_activity_in(
    live: &Path,
    rotated: &Path,
    query: &ActivityQuery,
) -> Result<ActivityPage, String> {
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
    let (mut current, mut older) = open_pair(live, rotated)?;
    if for_each_line_rev(current.as_mut(), &mut consume)? {
        for_each_line_rev(older.as_mut(), &mut consume)?;
    }

    Ok(ActivityPage {
        events,
        page: query.page,
        page_size,
        has_more,
    })
}

/// 级别 + 关键词匹配：解释、技术信息、事件码及请求编号均可检索。
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
    [&ev.title, &ev.summary, &ev.detail, &ev.technical_detail, &ev.code]
        .iter().any(|field| field.to_lowercase().contains(&needle))
        || ev.request_id.as_deref().is_some_and(|id| id.to_lowercase().contains(&needle))
}

/// 在已打开的文件快照上反向逐完整行读取（最新在前）；`f` 返回 false 即停。
/// 返回 true 表示读完、可继续下个文件；文件缺失可跳过，I/O 失败必须上报。
fn for_each_line_rev(
    opened: Option<&mut OpenedDiagnostic>,
    f: &mut impl FnMut(&str) -> bool,
) -> Result<bool, String> {
    let Some(opened) = opened else {
        return Ok(true);
    };
    let file = &mut opened.file;
    let mut pos = opened.len;
    let mut complete_tail = false;
    // leftover：已读区中最靠前、尚未定界完成的行片段（其更早字节还在 pos 之前未读的块里）。
    let mut leftover: Vec<u8> = Vec::new();
    while pos > 0 {
        let read_size = std::cmp::min(REVERSE_CHUNK as u64, pos) as usize;
        pos -= read_size as u64;
        file.seek(SeekFrom::Start(pos)).map_err(read_error)?;
        let mut block = vec![0u8; read_size];
        file.read_exact(&mut block).map_err(read_error)?;
        if !complete_tail {
            let Some(last_newline) = block.iter().rposition(|&b| b == b'\n') else {
                continue;
            };
            block.truncate(last_newline + 1);
            complete_tail = true;
        }
        block.extend_from_slice(&leftover); // block=更早字节，leftover=更后字节 ⇒ 拼成连续区间
        match block.iter().position(|&b| b == b'\n') {
            Some(idx) => {
                // idx 之后是完整行（可能多行），逆序发出；idx 之前（不含 \n）留作 leftover。
                if !emit_lines_rev(&block[idx + 1..], &mut *f) {
                    return Ok(false);
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
            return Ok(false);
        }
    }
    Ok(true)
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

// ───────────────────────── 命令层（薄封装；逻辑在上面的读取服务） ─────────────────────────

/// 由 AppHandle 解析 (live, rotated) 诊断文件路径。
/// logs 目录知识复用 `gateway_control::logs_dir`（与「打开日志文件夹」/ 网关落位同一真值）。
pub(crate) fn resolve_diagnostics_paths(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("解析应用数据目录失败：{e}"))?;
    let live = crate::gateway_control::logs_dir(&app_data).join(DIAGNOSTICS_FILE_NAME);
    let rotated = rotated_path_of(&live);
    Ok((live, rotated))
}

/// 最近活动复用分页服务的首页，两处的完整行、轮转、错误语义保持一致。
#[tauri::command]
pub async fn get_recent_activity(app: AppHandle) -> Result<Vec<ActivityEvent>, String> {
    query_activity_events(
        app,
        ActivityQuery {
            level: LevelFilter::All,
            keyword: String::new(),
            page: 0,
            page_size: RECENT_CAPACITY,
        },
    )
    .await
    .map(|page| page.events)
}

/// 分页懒读放阻塞线程；失败原样上报，不能伪装成空列表。
#[tauri::command]
pub async fn query_activity_events(
    app: AppHandle,
    query: ActivityQuery,
) -> Result<ActivityPage, String> {
    let (live, rotated) = resolve_diagnostics_paths(&app)?;
    tauri::async_runtime::spawn_blocking(move || query_activity_in(&live, &rotated, &query))
        .await
        .map_err(|_| "诊断读取任务失败，请重试。".to_string())?
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
    fn 按请求编号找到同一次请求的事件() {
        let dir = temp_dir("request-id");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = dir.join("relay-diagnostics.jsonl.1");
        write_file(&live, &format!("{L_RECEIVED}\n{L_ITEM_ERR}\n{L_FIN_503}\n"));
        let page = query_activity_in(&live, &rotated, &query(LevelFilter::All, "0c3987c3", 0, 10)).unwrap();
        assert_eq!(page.events.len(), 2);
        assert!(page.events.iter().all(|e| e.request_id.as_deref() == Some("0c3987c3")));
        let _ = fs::remove_dir_all(&dir);
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
        let mut reader = TailReader::new(live.clone());

        assert_eq!(reader.poll().unwrap(), 2, "首轮应读到 2 条事件");
        assert_eq!(
            reader.byte_offset(),
            file_len(&live),
            "偏移应推进到文件末尾"
        );
        let off1 = reader.byte_offset();

        // 无变化再轮询：0 新增，偏移不动（只读新增字节）
        assert_eq!(reader.poll().unwrap(), 0, "无新增应为 0");
        assert_eq!(reader.byte_offset(), off1);

        // 追加 1 行：只读新追加那一条
        append_file(&live, &format!("{L_FIN_200}\n"));
        assert_eq!(reader.poll().unwrap(), 1, "应只读到新追加的 1 条");
        assert_eq!(reader.byte_offset(), file_len(&live));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 增量tail_未完成尾行_暂不消费待补全() {
        let dir = temp_dir("tail-partial");
        let live = dir.join("relay-diagnostics.jsonl");
        // 一条完整 + 一条未完成（无结尾换行）
        write_file(&live, &format!("{L_STARTED}\n{{\"event\":\"recei"));
        let mut reader = TailReader::new(live.clone());
        assert_eq!(reader.poll().unwrap(), 1, "只应消费完整的那一行");
        assert_eq!(
            reader.byte_offset(),
            (L_STARTED.len() + 1) as u64,
            "偏移应停在第一行换行之后，不含未完成尾行"
        );
        // 补全尾行（第二行变完整）
        append_file(&live, "ved\",\"time\":\"t\",\"path\":\"/x\"}\n");
        assert_eq!(reader.poll().unwrap(), 1, "尾行补全后应读到第二条");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 轮转_live变短_从旧档捞回尾部并读新档() {
        let dir = temp_dir("rotate");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);

        write_file(&live, &format!("{L_STARTED}\n{L_RECEIVED}\n"));
        let mut reader = TailReader::new(live.clone());
        assert_eq!(reader.poll().unwrap(), 2);
        let off = reader.byte_offset();

        // 追加一行到 live 但不 poll：轮转后它将成为「漏读尾部」
        append_file(&live, &format!("{L_ITEM_ERR}\n"));
        // 轮转：当前 live 整体改名为 .1（含 3 行，前 2 行已读、第 3 行未读），新建更短的 live（1 行）
        fs::rename(&live, &rotated).expect("rename 轮转");
        write_file(&live, &format!("{L_FIN_503}\n"));
        assert!(
            file_len(&live) < off,
            "新 live 应短于旧偏移，才触发轮转检测"
        );

        let n = reader.poll().unwrap();
        assert_eq!(
            n, 2,
            "应捞回漏读尾部(item_error)+读新档(finished-503)，实际 {n}"
        );
        assert_eq!(
            reader.byte_offset(),
            file_len(&live),
            "偏移应重置并推进到新 live 末尾"
        );
        // TailReader 只报告新增条数；记录内容由查询服务核对（同一文件、同一完整行语义）。
        let page = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 10)).unwrap();
        assert_eq!(page.events.len(), 4, "两处文件共 4 条完整记录");
        assert_eq!(
            page.events[0].level,
            ActivityLevel::Error,
            "最新在前：新 live 的 503 为错误"
        );
        assert!(page.events[0].detail.contains("503"), "{}", page.events[0].detail);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 轮转_新档不短于旧偏移仍补读且不重放() {
        let dir = temp_dir("rotate-identity");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        write_file(&live, &format!("{L_STARTED}\n"));
        let mut reader = TailReader::new(live.clone());
        assert_eq!(reader.poll().unwrap(), 1);
        append_file(&live, &format!("{L_ITEM_ERR}\n"));
        fs::rename(&live, &rotated).unwrap();
        write_file(&live, &format!("{L_RECEIVED}\n{L_FIN_503}\n"));
        assert!(file_len(&live) >= reader.byte_offset());
        assert_eq!(reader.poll().unwrap(), 3, "文件身份已变：旧尾1条 + 新档2条");
        assert_eq!(reader.poll().unwrap(), 0, "重复通知不能重放旧档");
        let page = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 10)).unwrap();
        assert_eq!(page.events.len(), 4, "历史不重放：两处文件共 4 条完整记录");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn 连续轮转_补读仍保留的中间旧档且不重复() {
        let dir = temp_dir("rotate-twice");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        write_file(&live, &format!("{L_STARTED}\n"));
        let mut reader = TailReader::new(live.clone());
        reader.snapshot().unwrap();
        append_file(&live, &format!("{L_ITEM_ERR}\n"));
        fs::rename(&live, &rotated).unwrap();
        write_file(&live, &format!("{L_RECEIVED}\n"));
        fs::remove_file(&rotated).unwrap();
        fs::rename(&live, &rotated).unwrap();
        write_file(&live, &format!("{L_FIN_503}\n"));
        assert_eq!(reader.poll().unwrap(), 3, "旧句柄尾部 + 中间.1 + 当前文件");
        assert_eq!(reader.poll().unwrap(), 0, "重复通知不能重放旧档");
        // 中间旧档已被删除/覆盖，磁盘上只剩仍存在的两个世代各一条；轮转补读不重复由上面的 poll 计数保证。
        let page = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 10)).unwrap();
        assert_eq!(page.events.len(), 2, "只剩两个仍存在的文件世代");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn 轮转缺口_旧尾先补读_新档同长度也从头接续() {
        let dir = temp_dir("rotate-gap");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        write_file(&live, &format!("{L_STARTED}\n"));
        let mut reader = TailReader::new(live.clone());
        reader.snapshot().unwrap();
        append_file(&live, &format!("{L_ITEM_ERR}\n"));
        fs::rename(&live, &rotated).unwrap();
        assert_eq!(reader.poll().unwrap(), 1, "rename与create间隙仍能补读旧尾");
        assert_eq!(reader.poll().unwrap(), 0);
        write_file(&live, &format!("{L_STARTED}\n"));
        assert_eq!(
            reader.poll().unwrap(),
            1,
            "相同内容仍是一条新的诊断记录，不能按文本去重"
        );
        assert_eq!(reader.poll().unwrap(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn 读取失败后恢复不丢旧尾_截断不重放无关旧档() {
        let dir = temp_dir("tail-recovery");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        write_file(&rotated, &format!("{L_FIN_200}\n"));
        write_file(&live, &format!("{L_STARTED}\n{L_RECEIVED}\n"));
        let mut reader = TailReader::new(live.clone());
        reader.snapshot().unwrap();
        write_file(&live, "");
        assert_eq!(reader.poll().unwrap(), 0, "截断不能把更早的.1错当新活动");
        append_file(&live, &format!("{L_ITEM_ERR}\n"));
        fs::remove_file(&rotated).unwrap();
        fs::rename(&live, &rotated).unwrap();
        fs::create_dir(&live).unwrap();
        assert!(reader.poll().is_err());
        fs::remove_dir(&live).unwrap();
        write_file(&live, &format!("{L_FIN_503}\n"));
        assert_eq!(
            reader.poll().unwrap(),
            2,
            "失败未推进偏移；恢复后旧尾与新档各一次"
        );
        assert_eq!(reader.poll().unwrap(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn 查询_长半行跨块仍只显示完整记录_缺失文件是空页() {
        let dir = temp_dir("long-partial");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        assert!(
            query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 10))
                .unwrap()
                .events
                .is_empty()
        );
        write_file(
            &live,
            &format!("{L_STARTED}\n{}", "x".repeat(REVERSE_CHUNK + 100)),
        );
        let page = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 10)).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].title, "网关已启动");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn 截断_live清零后继续读新内容_不崩() {
        let dir = temp_dir("truncate");
        let live = dir.join("relay-diagnostics.jsonl");
        write_file(&live, &format!("{L_STARTED}\n{L_RECEIVED}\n"));
        let mut reader = TailReader::new(live.clone());
        assert_eq!(reader.poll().unwrap(), 2);
        // 截断为空（无 .1）
        write_file(&live, "");
        assert_eq!(reader.poll().unwrap(), 0, "变短+空内容：0 新增，不崩");
        assert_eq!(reader.byte_offset(), 0, "偏移应归零");
        // 写入新内容
        write_file(&live, &format!("{L_FIN_503}\n"));
        assert_eq!(reader.poll().unwrap(), 1, "归零后应从头读到新内容");
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
        let p0 = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 3)).unwrap();
        assert_eq!(p0.events.len(), 3);
        assert!(
            p0.events[0].technical_detail.contains("/req/6"),
            "最新在前：{}",
            p0.events[0].technical_detail
        );
        assert!(p0.events[2].technical_detail.contains("/req/4"));
        assert!(p0.has_more, "后面还有 4 条");

        // 第 1 页跨越 live/.1 边界：/req/3,2,1
        let p1 = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 1, 3)).unwrap();
        assert!(
            p1.events[0].technical_detail.contains("/req/3"),
            "{}",
            p1.events[0].technical_detail
        );
        assert!(p1.events[2].technical_detail.contains("/req/1"));
        assert!(p1.has_more, "还有 /req/0");

        // 第 2 页：仅剩 /req/0
        let p2 = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 2, 3)).unwrap();
        assert_eq!(p2.events.len(), 1);
        assert!(p2.events[0].technical_detail.contains("/req/0"));
        assert!(!p2.has_more, "已到最旧");

        // 越界页：空
        let p3 = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 3, 3)).unwrap();
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
            query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, big))
                .unwrap()
                .events
                .len(),
            5
        );
        let errs =
            query_activity_in(&live, &rotated, &query(LevelFilter::Error, "", 0, big)).unwrap();
        assert_eq!(errs.events.len(), 1, "仅 finished-503 为错误");
        assert!(errs.events[0].detail.contains("503"));
        assert_eq!(
            query_activity_in(&live, &rotated, &query(LevelFilter::Warn, "", 0, big))
                .unwrap()
                .events
                .len(),
            1,
            "仅 item_error 为警告"
        );
        assert_eq!(
            query_activity_in(&live, &rotated, &query(LevelFilter::Info, "", 0, big))
                .unwrap()
                .events
                .len(),
            3
        );
        // 关键词：搜 azure（item_error 详情含「跨 Azure 资源引用」，大小写不敏感）
        let kw =
            query_activity_in(&live, &rotated, &query(LevelFilter::All, "azure", 0, big)).unwrap();
        assert_eq!(kw.events.len(), 1);
        assert_eq!(kw.events[0].level, ActivityLevel::Warn);
        // 关键词+级别：http 且仅错误 → finished-503（item_error 虽含 http 但为警告，被排除）
        let kw2 =
            query_activity_in(&live, &rotated, &query(LevelFilter::Error, "http", 0, big)).unwrap();
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
        let mut reader = TailReader::new(live.clone());
        let n = reader.poll().unwrap();
        assert_eq!(
            n, 3,
            "仅 started/mystery(回退)/finished-200 可本地化，其余诚实跳过，实际 {n}"
        );

        // 查询接缝（同一文件）：同样 3 条，最新在前（finished-200）
        let page = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 50)).unwrap();
        assert_eq!(page.events.len(), 3);
        assert_eq!(page.events[0].title, "请求结束", "最新在前");
        assert!(
            page.events.iter().any(|e| e.title.contains("mystery_future_code")),
            "未知码应原样回退显示标题"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 首次快照_含旧档且不误报历史_随后补全尾行仅报新增() {
        let dir = temp_dir("snapshot");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        write_file(&rotated, &format!("{L_STARTED}\n"));
        write_file(&live, &format!("{L_RECEIVED}\n{L_FIN_200}"));
        let mut reader = TailReader::new(live.clone());
        let snapshot = reader.snapshot().unwrap();
        assert_eq!(snapshot.len(), 2, "首次快照包含两个文件的完整行");
        assert_eq!(reader.poll().unwrap(), 0, "已有记录不能产生有新事件通知");
        append_file(&live, "\n");
        assert_eq!(reader.poll().unwrap(), 1, "快照后的完整新行才通知");
        assert_eq!(reader.poll().unwrap(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn 查询与最近快照一致_未完成尾行不提前显示() {
        let dir = temp_dir("query-partial");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        write_file(&rotated, &format!("{L_STARTED}\n"));
        write_file(&live, &format!("{L_RECEIVED}\n{L_FIN_200}"));
        let mut reader = TailReader::new(live.clone());
        // 快照按文件先后（旧档→live）逐行返回，倒序后即全局最新在前，与查询一致。
        let mut recent = reader.snapshot().unwrap();
        recent.reverse();
        let page = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 50)).unwrap();
        assert_eq!(
            page.events, recent,
            "同一诊断记录必须在两处一致显示，半行还不是记录"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn 查询_读取失败不能伪装成空列表() {
        let dir = temp_dir("query-error");
        let live = dir.join("relay-diagnostics.jsonl");
        let rotated = rotated_path_of(&live);
        fs::create_dir(&live).unwrap();
        let result = query_activity_in(&live, &rotated, &query(LevelFilter::All, "", 0, 10));
        assert!(
            result.is_err(),
            "目标不可读取时必须返回明确失败，而不是无记录"
        );
        let _ = fs::remove_dir_all(dir);
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
