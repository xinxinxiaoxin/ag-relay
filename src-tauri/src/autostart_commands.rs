//! Tauri 命令层：开机自启状态查询与手动开关（与 UI 同一命令路径）。
//! 业务逻辑全部在 autostart 模块（纯函数，可脱离 tauri 运行时测试），本层只做薄封装。

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::autostart::{self, AutostartStore};

/// 应用启动编排产生的提示消息（前端加载时取走展示；旧 lnk「未动」等结论在这里）。
#[derive(Default)]
pub struct AutostartUiState {
    notes: Mutex<Vec<String>>,
}

impl AutostartUiState {
    fn store_notes(&self, notes: Vec<String>) {
        *self.notes.lock().unwrap_or_else(|e| e.into_inner()) = notes;
    }

    fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut *self.notes.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// 给前端的自启状态（enabled 与注册表实况一致，不是前端自己记的开关位）。
#[derive(Debug, Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    /// 当前自启项指向的 exe（未开启时为 null）
    pub exe_path: Option<String>,
    /// 启动编排的提示（旧 lnk 处置结论等）；仅首次获取返回，之后为空
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutostartSetResult {
    /// 设置成功，返回最新状态
    Ok { status: AutostartStatus },
    /// 设置失败（注册表读写异常），reason 为中文原因
    Failed { reason: String },
}

fn status_of(app: &AppHandle, notes: Vec<String>) -> AutostartStatus {
    let store = AutostartStore::production();
    let exe_path = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let enabled = autostart::current_enabled(&store).unwrap_or(false);
    let _ = app; // 预留：后续需要 app_data_dir 时使用
    AutostartStatus { enabled, exe_path, notes }
}

/// 应用启动钩子（lib.rs setup 调用）：幂等编排自启项 + 旧 lnk 清理，
/// 提示消息暂存到托管状态，等前端首次 get_autostart 取走。
pub fn sync_on_launch(app: &AppHandle) {
    let store = AutostartStore::production();
    let report = match autostart::user_startup_dir() {
        Ok(dir) => {
            let exe = std::env::current_exe().unwrap_or_else(|_| "ag-relay.exe".into());
            autostart::sync_on_launch(&store, &dir, autostart::LEGACY_LNK_NAME, &exe)
        }
        Err(e) => autostart::LaunchSyncReport {
            enabled: false,
            notes: vec![format!("开机自启：解析启动文件夹失败：{e}")],
        },
    };
    let state: State<AutostartUiState> = app.state();
    state.store_notes(report.notes);
}

/// 查询自启状态（含启动编排提示，仅首次返回）。
#[tauri::command]
pub fn get_autostart(app: AppHandle) -> AutostartStatus {
    let state: State<AutostartUiState> = app.state();
    let notes = state.take_notes();
    status_of(&app, notes)
}

/// 手动开关开机自启，即时生效（写注册表）。
#[tauri::command]
pub fn set_autostart_enabled(app: AppHandle, enabled: bool) -> AutostartSetResult {
    let store = AutostartStore::production();
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return AutostartSetResult::Failed {
                reason: format!("解析当前程序路径失败：{e}"),
            }
        }
    };
    match autostart::set_enabled(&store, &exe, enabled) {
        Ok(_) => AutostartSetResult::Ok {
            status: status_of(&app, Vec::new()),
        },
        Err(e) => AutostartSetResult::Failed { reason: e.to_string() },
    }
}
