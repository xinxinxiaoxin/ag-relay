//! 关闭选择与应用退出接线。关窗先保留请求供面板选择，不停止网关。
//! 真正退出统一经过更新互斥模块；排空失败保留窗口与受管句柄。

use crate::app_update::ExitFailureReason;
use crate::bounded_log::{append_record, LogPolicy};
use crate::exit_requests::{CloseRequests, CloseWindow, ExitRequests};

use tauri::{AppHandle, Emitter, Manager};

use crate::gateway_control::logs_dir;
use crate::timeutil::{civil_from_days, now_unix_secs};

/// 壳侧退出来源日志文件名（与网关诊断日志同目录，便于一并查看）。
pub const EXIT_LOG_FILE: &str = "shell-exit.log";
pub const WINDOW_CLOSE_REQUESTED: &str = "window-close-requested";
pub const APP_EXIT_FAILED: &str = "app-exit-failed";

#[derive(Default)]
pub struct AppExitState {
    close: CloseRequests,
    pub(crate) exits: ExitRequests,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseRequest {
    pub id: u64,
    pub tray_available: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    Cancel,
    Tray,
    Quit,
}

struct MainWindow<'a>(&'a AppHandle);

impl CloseWindow for MainWindow<'_> {
    fn tray_available(&self) -> bool {
        self.0.tray_by_id(crate::tray::TRAY_ID).is_some()
    }

    fn hide(&self) -> Result<(), String> {
        self.0
            .get_webview_window("main")
            .ok_or("主窗口不可用，未隐藏面板。")?
            .hide()
            .map_err(|_| "无法隐藏面板，请重试或选择取消。".into())
    }
}

fn close_request(app: &AppHandle, id: u64) -> CloseRequest {
    CloseRequest {
        id,
        tray_available: MainWindow(app).tray_available(),
    }
}

/// CloseRequested 已 prevent_close 后调用；先保存再通知，监听尚未建立也不丢请求。
pub fn request_window_close(app: &AppHandle) {
    let (id, is_new) = app.state::<AppExitState>().close.request();
    crate::tray::show_main_window(app);
    if is_new {
        let _ = app.emit(WINDOW_CLOSE_REQUESTED, close_request(app, id));
    }
}

#[tauri::command]
pub fn get_close_request(app: AppHandle) -> Option<CloseRequest> {
    app.state::<AppExitState>()
        .close
        .get()
        .map(|id| close_request(&app, id))
}

#[tauri::command]
pub async fn respond_to_close(
    app: AppHandle,
    request_id: u64,
    action: CloseAction,
) -> Result<(), String> {
    let state = app.state::<AppExitState>();
    match action {
        CloseAction::Cancel => state.close.cancel(request_id),
        CloseAction::Tray => state.close.send_to_tray(request_id, &MainWindow(&app)),
        CloseAction::Quit => {
            state
                .close
                .quit(request_id, || request_exit(&app, ExitSource::WindowClose))
                .await
        }
    }
}

/// 退出来源白名单：日志只允许出现这三个枚举值，不写任何其他内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitSource {
    /// 点窗口 ×（含等价的系统关窗：Alt+F4 / 任务栏右键关闭）
    WindowClose,
    /// 托盘右键菜单「退出」
    TrayMenu,
    /// 面板「退出」按钮
    PanelButton,
}

impl ExitSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ExitSource::WindowClose => "window_close",
            ExitSource::TrayMenu => "tray_menu",
            ExitSource::PanelButton => "panel_button",
        }
    }
}

/// Unix 秒 → UTC 时间字符串（RFC3339 Z 形，秒精度）。
/// 历法换算复用共享模块 timeutil（civil_from_days）；纪元 / 闰日 / 远期锚点由单测锁定。
pub fn format_utc_timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// 一条退出来源日志行的完整格式（唯一内容：UTC 时间 + 白名单枚举值）。
pub fn format_exit_record(unix_secs: u64, source: ExitSource) -> String {
    format!(
        "{} exit_source={}",
        format_utc_timestamp(unix_secs),
        source.as_str()
    )
}

/// 授权成功不等同于桌面进程已经退出；失败只携带封闭原因码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitOutcome {
    Authorized,
    Failed(ExitFailureReason),
}

pub fn format_exit_outcome_record(
    unix_secs: u64,
    source: Option<ExitSource>,
    outcome: ExitOutcome,
) -> String {
    let event = match outcome {
        ExitOutcome::Authorized => "event=safe_exit_authorized".into(),
        ExitOutcome::Failed(reason) => format!("event=safe_exit_failed reason={}", reason.as_str()),
    };
    let source = source
        .map(|source| format!(" exit_source={}", source.as_str()))
        .unwrap_or_default();
    format!("{} {event}{source}", format_utc_timestamp(unix_secs))
}

/// 时间、路径可注入；日志不可写时也不得改变退出的实际结果。
pub fn log_exit_outcome_in(
    app_data: &std::path::Path,
    unix_secs: u64,
    source: Option<ExitSource>,
    outcome: ExitOutcome,
) {
    let _ = append_shell_record_in(
        app_data,
        &format_exit_outcome_record(unix_secs, source, outcome),
    );
}

pub fn log_exit_outcome(app: &AppHandle, source: Option<ExitSource>, outcome: ExitOutcome) {
    let Ok(app_data) = app.path().app_data_dir() else {
        return;
    };
    log_exit_outcome_in(&app_data, now_unix_secs(), source, outcome);
}

/// 外部网关停止事件（F1 白名单扩展；与退出来源写入同一 shell-exit.log，
/// 只允许这三个枚举值，不写任何其他内容）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalStopEvent {
    /// 外部 supervisor 已在预算内优雅退出
    Graceful,
    /// 优雅窗口超时，已 taskkill 强杀 supervisor 进程树
    Forced,
    /// pid 链无法核实，宁可不杀不可误杀，未执行停止
    Abandoned,
}

impl ExternalStopEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            ExternalStopEvent::Graceful => "external_stop_graceful",
            ExternalStopEvent::Forced => "external_stop_forced",
            ExternalStopEvent::Abandoned => "external_stop_abandoned",
        }
    }
}

/// 一条外部停止事件日志行的完整格式（UTC 时间 + event=白名单枚举值）。
pub fn format_external_stop_record(unix_secs: u64, event: ExternalStopEvent) -> String {
    format!(
        "{} event={}",
        format_utc_timestamp(unix_secs),
        event.as_str()
    )
}

/// 把外部停止事件追加到壳侧日志（与退出来源同一文件，便于一并查看）。
pub fn log_external_stop(app: &AppHandle, event: ExternalStopEvent) {
    append_shell_log(app, &format_external_stop_record(now_unix_secs(), event));
}

/// 把退出来源追加到壳侧日志（%APPDATA%\ag-relay\logs\shell-exit.log）。
/// 尽力而为：目录建不了 / 写不进去都不影响退出流程本身。
pub fn log_exit_source(app: &AppHandle, source: ExitSource) {
    append_shell_log(app, &format_exit_record(now_unix_secs(), source));
}

/// 追加一行到壳侧日志（shell-exit.log）；日志目录复用 gateway_control::logs_dir。
/// 尽力而为：失败静默，绝不影响退出/停止流程。
fn append_shell_log(app: &AppHandle, line: &str) {
    let Ok(app_data) = app.path().app_data_dir() else {
        return;
    };
    let _ = append_shell_record_in(&app_data, line);
}

/// 应用层文件接缝：调用方只传入已格式化的白名单记录，路径可注入。
pub fn append_shell_record_in(app_data: &std::path::Path, line: &str) -> std::io::Result<()> {
    append_record(
        &logs_dir(app_data).join(EXIT_LOG_FILE),
        line,
        LogPolicy {
            max_total_bytes: 1024 * 1024,
            retention: None,
        },
    )
}

/// 统一退出入口：同步抢占退出资格，调用方必须等待返回值，不能把排空失败当成成功。
pub fn request_exit(
    app: &AppHandle,
    source: ExitSource,
) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
    crate::update_commands::request_exit(app, Some(source))
}

#[derive(Clone, serde::Serialize)]
struct ExitFailure {
    detail: String,
}

/// 系统与托盘回调不能 await；先同步抢占，再等待同一结果并把失败呈现在主窗口。
/// None 仅供没有可审计用户来源的 RunEvent::ExitRequested 使用。
pub fn request_exit_from_event(app: &AppHandle, source: Option<ExitSource>) {
    let exit = crate::update_commands::request_exit(app, source);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(detail) = exit.await {
            crate::tray::show_main_window(&app);
            let _ = app.emit(APP_EXIT_FAILED, ExitFailure { detail });
        }
    });
}

/// 面板「退出」命令等待真实安全停机结果；不同于关窗，不再次询问关闭方式。
#[tauri::command]
pub async fn quit_app(app: AppHandle) -> Result<(), String> {
    request_exit(&app, ExitSource::PanelButton).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 关闭请求与退出失败事件字段遵循前端契约() {
        assert_eq!(
            serde_json::to_value(CloseRequest {
                id: 7,
                tray_available: true
            })
            .unwrap(),
            serde_json::json!({ "id": 7, "trayAvailable": true })
        );
        assert_eq!(
            serde_json::to_value(ExitFailure {
                detail: "排空超时".into()
            })
            .unwrap(),
            serde_json::json!({ "detail": "排空超时" })
        );
    }

    #[test]
    fn 关闭选择只接受约定的三个小写动作() {
        assert!(matches!(
            serde_json::from_str::<CloseAction>("\"cancel\""),
            Ok(CloseAction::Cancel)
        ));
        assert!(matches!(
            serde_json::from_str::<CloseAction>("\"tray\""),
            Ok(CloseAction::Tray)
        ));
        assert!(matches!(
            serde_json::from_str::<CloseAction>("\"quit\""),
            Ok(CloseAction::Quit)
        ));
        assert!(serde_json::from_str::<CloseAction>("\"Quit\"").is_err());
        assert!(serde_json::from_str::<CloseAction>("\"force\"").is_err());
    }

    #[test]
    fn 壳侧退出日志_应用入口有界且重开后保留此前记录() {
        // 只在独立临时目录验证，不构造 AppHandle，不实际退出或启动网关。
        let base = std::env::temp_dir().join(format!(
            "ag-relay-exit-log-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(logs_dir(&base)).unwrap();
        let path = logs_dir(&base).join(EXIT_LOG_FILE);
        let old = "old\n".repeat(131072); // 512 KiB：达到单档上限
        std::fs::write(&path, &old).unwrap();
        append_shell_record_in(&base, &format_exit_record(0, ExitSource::WindowClose)).unwrap();
        append_shell_record_in(&base, &format_exit_record(1, ExitSource::PanelButton)).unwrap();
        log_exit_outcome_in(
            &base,
            2,
            Some(ExitSource::PanelButton),
            ExitOutcome::Failed(ExitFailureReason::SupervisorChainUnverified),
        );
        let result = std::fs::read_to_string(&path).unwrap();
        let backup = std::fs::read_to_string(logs_dir(&base).join("shell-exit.log.1"));
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(result, "1970-01-01T00:00:00Z exit_source=window_close\n1970-01-01T00:00:01Z exit_source=panel_button\n1970-01-01T00:00:02Z event=safe_exit_failed reason=supervisor_chain_unverified exit_source=panel_button\n");
        assert_eq!(backup.unwrap(), old);
    }

    #[test]
    fn 退出结果_区分失败与授权且不伪造系统事件来源() {
        assert_eq!(
            format_exit_outcome_record(0, Some(ExitSource::TrayMenu), ExitOutcome::Authorized),
            "1970-01-01T00:00:00Z event=safe_exit_authorized exit_source=tray_menu"
        );
        assert_eq!(
            format_exit_outcome_record(0, Some(ExitSource::WindowClose), ExitOutcome::Failed(ExitFailureReason::ProcessChainExitUnconfirmed)),
            "1970-01-01T00:00:00Z event=safe_exit_failed reason=process_chain_exit_unconfirmed exit_source=window_close"
        );
        assert_eq!(
            format_exit_outcome_record(
                0,
                None,
                ExitOutcome::Failed(ExitFailureReason::DrainTimeoutCancelled)
            ),
            "1970-01-01T00:00:00Z event=safe_exit_failed reason=drain_timeout_cancelled"
        );
    }

    #[test]
    fn 退出结果_路径不可写或锁忙时不阻塞且稍后仍可写入() {
        let base = std::env::temp_dir().join(format!(
            "ag-relay-exit-unwritable-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let blocked = base.join("not-a-directory");
        std::fs::write(&blocked, "keep").unwrap();
        log_exit_outcome_in(&blocked, 0, None, ExitOutcome::Authorized);
        assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "keep");

        log_exit_outcome_in(
            &base,
            0,
            None,
            ExitOutcome::Failed(ExitFailureReason::StatusUnverified),
        );
        let path = logs_dir(&base).join(EXIT_LOG_FILE);
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .open(logs_dir(&base).join("shell-exit.log.lock"))
            .unwrap();
        lock.lock().unwrap();
        log_exit_outcome_in(&base, 1, None, ExitOutcome::Authorized);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "1970-01-01T00:00:00Z event=safe_exit_failed reason=status_unverified\n"
        );
        drop(lock);
        log_exit_outcome_in(&base, 2, None, ExitOutcome::Authorized);
        let result = std::fs::read_to_string(&path);
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(result.unwrap(), "1970-01-01T00:00:00Z event=safe_exit_failed reason=status_unverified\n1970-01-01T00:00:02Z event=safe_exit_authorized\n");
    }

    #[test]
    fn 退出来源白名单_只有三个枚举值() {
        assert_eq!(ExitSource::WindowClose.as_str(), "window_close");
        assert_eq!(ExitSource::TrayMenu.as_str(), "tray_menu");
        assert_eq!(ExitSource::PanelButton.as_str(), "panel_button");
    }

    #[test]
    fn 时间戳_纪元闰日与远期锚点() {
        // 锚点独立真值：node -e "new Date(s*1000).toISOString()" 换算
        assert_eq!(format_utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc_timestamp(951_782_400), "2000-02-29T00:00:00Z"); // 闰日
        assert_eq!(format_utc_timestamp(1_760_000_000), "2025-10-09T08:53:20Z");
        assert_eq!(format_utc_timestamp(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn 日志行_只有时间与白名单枚举_无其他内容() {
        assert_eq!(
            format_exit_record(0, ExitSource::WindowClose),
            "1970-01-01T00:00:00Z exit_source=window_close"
        );
        assert_eq!(
            format_exit_record(1_760_169_059, ExitSource::PanelButton),
            "2025-10-11T07:50:59Z exit_source=panel_button"
        );
        assert_eq!(
            format_exit_record(1_760_169_059, ExitSource::TrayMenu),
            "2025-10-11T07:50:59Z exit_source=tray_menu"
        );
    }

    /// F1：外部停止事件也走白名单（固定枚举值，不携带任何自由文本）。
    #[test]
    fn 外部停止事件行_只有时间与白名单枚举() {
        assert_eq!(
            format_external_stop_record(0, ExternalStopEvent::Graceful),
            "1970-01-01T00:00:00Z event=external_stop_graceful"
        );
        assert_eq!(
            format_external_stop_record(1_760_169_059, ExternalStopEvent::Forced),
            "2025-10-11T07:50:59Z event=external_stop_forced"
        );
        assert_eq!(
            format_external_stop_record(1_760_169_059, ExternalStopEvent::Abandoned),
            "2025-10-11T07:50:59Z event=external_stop_abandoned"
        );
    }
}
