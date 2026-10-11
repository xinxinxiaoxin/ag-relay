//! 应用退出路径（T5）：× 关窗 / 托盘「退出」/ 面板「退出」按钮三条路收敛为一条——
//! 先把退出来源记入壳侧日志（内容仅「UTC 时间 + 白名单枚举值」），再 app.exit(0)；
//! 三条路都汇入 RunEvent::Exit 的既有钩子（shutdown_blocking_on_exit）优雅停网关。
//! 关窗路径不调 exit：默认链放行（CloseRequested → ExitRequested(None) → Exit），只记日志。

use std::io::Write;

use tauri::{AppHandle, Manager};

use crate::gateway_control::logs_dir;
use crate::timeutil::{civil_from_days, now_unix_secs};

/// 壳侧退出来源日志文件名（与网关诊断日志同目录，便于一并查看）。
pub const EXIT_LOG_FILE: &str = "shell-exit.log";

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
    format!("{} event={}", format_utc_timestamp(unix_secs), event.as_str())
}

/// 把外部停止事件追加到壳侧日志（与退出来源同一文件，便于一并查看）。
pub fn log_external_stop(app: &AppHandle, event: ExternalStopEvent) {
    append_shell_log(app, &format_external_stop_record(now_unix_secs(), event));
}

/// 把退出来源追加到壳侧日志（%APPDATA%\ag-relay\logs\shell-exit.log）。
/// 尽力而为：目录建不了 / 写不进去都不影响退出流程本身。
pub fn log_exit_source(app: &AppHandle, source: ExitSource) {
    append_shell_log(
        app,
        &format_exit_record(now_unix_secs(), source),
    );
}

/// 追加一行到壳侧日志（shell-exit.log）；日志目录复用 gateway_control::logs_dir。
/// 尽力而为：失败静默，绝不影响退出/停止流程。
fn append_shell_log(app: &AppHandle, line: &str) {
    let Ok(app_data) = app.path().app_data_dir() else {
        return;
    };
    let dir = logs_dir(&app_data);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(EXIT_LOG_FILE))
        .and_then(|mut f| writeln!(f, "{line}"));
}

/// 统一退出入口：记来源日志 → app.exit(0)。
/// app.exit 与关窗殊途同归：都会触发 RunEvent::Exit → 优雅停网关钩子。
pub fn request_exit(app: &AppHandle, source: ExitSource) {
    log_exit_source(app, source);
    app.exit(0);
}

/// 面板「退出」按钮命令（与 UI 同一命令路径）；行为与点 × 完全一致。
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    request_exit(&app, ExitSource::PanelButton);
}

#[cfg(test)]
mod tests {
    use super::*;

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
