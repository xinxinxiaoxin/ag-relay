//! Tauri 命令适配：连接模块统一持有生命周期独占，保存不会触发控制请求。
use crate::{
    connection_settings::ConnectionSettingsStore,
    gateway_connection::{ConnectionError, ConnectionSettingsView, GatewayConnection},
    gateway_control,
    gateway_process::{resolve_paths, GatewayPaths, GATEWAY_PORT, STOP_TIMEOUT},
    policy_settings::{PolicySettings, PolicySettingsStore, PolicySettingsView},
};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU16, Ordering},
    Mutex, TryLockError,
};
use tauri::{AppHandle, Manager};

pub struct GatewayState {
    connection: Mutex<Option<GatewayConnection>>,
    managed: AtomicBool,
    port: AtomicU16,
}
impl Default for GatewayState {
    fn default() -> Self {
        Self {
            connection: Mutex::new(None),
            managed: AtomicBool::new(false),
            port: AtomicU16::new(GATEWAY_PORT),
        }
    }
}
impl GatewayState {
    /// 仅供实例展示/诊断路径选择；不是执行更新或停机的身份凭证。
    pub fn holds_managed(&self) -> bool {
        self.managed.load(Ordering::Acquire)
    }
    pub fn current_port(&self) -> u16 {
        self.port.load(Ordering::Acquire)
    }
}

/// 更新可在此闭包内核实身份 → safe_shutdown → install，独占直到闭包返回。
/// 忙时拒绝而不排队，防止超时恢复接单后又执行排队的破坏性操作。不可重入。
pub fn with_connection<T>(
    app: &AppHandle,
    action: impl FnOnce(&mut GatewayConnection) -> Result<T, ConnectionError>,
) -> Result<T, ConnectionError> {
    let state = app.state::<GatewayState>();
    let mut guard = match state.connection.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => {
            return Err(ConnectionError::Failed(
                "网关正在执行另一项操作，请稍后重试".into(),
            ))
        }
    };
    if guard.is_none() {
        let app_data = app
            .path()
            .app_data_dir()
            .map_err(|_| ConnectionError::Failed("无法解析设置目录".into()))?;
        let paths = resolve_runtime_paths(app)?;
        *guard = Some(GatewayConnection::new_with_policy_store(
            paths,
            ConnectionSettingsStore::new(app_data.join("connection-settings.json")),
            PolicySettingsStore::new(app_data.join("policy-settings.json")),
        ));
    }
    let connection = guard.as_mut().unwrap();
    let result = action(connection);
    state
        .managed
        .store(connection.holds_managed(), Ordering::Release);
    if let Ok(port) = connection.port() {
        state.port.store(port, Ordering::Release);
    }
    result
}

fn with_user_connection<T>(
    app: &AppHandle,
    action: impl FnOnce(&mut GatewayConnection) -> Result<T, ConnectionError>,
) -> Result<T, ConnectionError> {
    with_connection(app, |gateway| {
        if crate::update_commands::controls_blocked(app) {
            return Err(ConnectionError::Failed(
                "更新或退出正在进行，请等待当前操作完成".into(),
            ));
        }
        action(gateway)
    })
}

#[tauri::command]
pub fn get_connection_settings(app: AppHandle) -> Result<ConnectionSettingsView, String> {
    with_connection(&app, |gw| gw.settings()).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn save_connection_settings(
    app: AppHandle,
    upstream: String,
    port: u32,
) -> Result<ConnectionSettingsView, String> {
    with_user_connection(&app, |gw| gw.save(&upstream, port)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_policy_settings(app: AppHandle) -> Result<PolicySettingsView, String> {
    with_connection(&app, |gw| gw.policy_settings()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_policy_settings(
    app: AppHandle,
    settings: PolicySettings,
) -> Result<PolicySettingsView, String> {
    with_user_connection(&app, |gw| gw.save_policy(&settings)).map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GatewayActionResult {
    Started {
        supervisor_pid: u32,
    },
    Stopped {
        graceful: bool,
        exit_code: Option<i32>,
    },
    StoppedExternal {
        graceful: bool,
    },
    NotRunning,
    Failed {
        reason: String,
    },
}

#[tauri::command]
pub async fn start_gateway(app: AppHandle) -> GatewayActionResult {
    match tauri::async_runtime::spawn_blocking(move || {
        with_user_connection(&app, |gw| {
            crate::update_commands::acknowledge_manual_start(&app)?;
            gw.start()
        })
    })
    .await
    {
        Ok(Ok(supervisor_pid)) => GatewayActionResult::Started { supervisor_pid },
        Ok(Err(e)) => GatewayActionResult::Failed {
            reason: e.to_string(),
        },
        Err(_) => GatewayActionResult::Failed {
            reason: "网关启动任务失败".into(),
        },
    }
}

/// 与配置重启/更新共用安全停机，不调用旧的超时强杀路径。
#[tauri::command]
pub async fn stop_gateway(app: AppHandle) -> GatewayActionResult {
    let result = tauri::async_runtime::spawn_blocking(move || {
        with_user_connection(&app, |gw| {
            let managed = gw.holds_managed();
            Ok((gw.safe_shutdown(STOP_TIMEOUT)?, managed))
        })
    })
    .await;
    match result {
        Ok(Ok((false, _))) => GatewayActionResult::NotRunning,
        Ok(Ok((true, true))) => GatewayActionResult::Stopped {
            graceful: true,
            exit_code: Some(0),
        },
        Ok(Ok((true, false))) => GatewayActionResult::StoppedExternal { graceful: true },
        Ok(Err(e)) => GatewayActionResult::Failed {
            reason: e.to_string(),
        },
        Err(_) => GatewayActionResult::Failed {
            reason: "网关停机任务失败".into(),
        },
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RestartResult {
    Restarted,
    NotRunning,
    Timeout,
    Failed { detail: String },
}

/// 必须重新创建 supervisor 才能注入新的环境，202 已受理不能代表重启完成。
#[tauri::command]
pub async fn restart_gateway(app: AppHandle) -> RestartResult {
    match tauri::async_runtime::spawn_blocking(move || {
        with_user_connection(&app, |gw| gw.restart(STOP_TIMEOUT))
    })
    .await
    {
        Ok(Ok(true)) => RestartResult::Restarted,
        Ok(Ok(false)) => RestartResult::NotRunning,
        Ok(Err(ConnectionError::Timeout)) => RestartResult::Timeout,
        Ok(Err(e)) => RestartResult::Failed {
            detail: e.to_string(),
        },
        Err(_) => RestartResult::Failed {
            detail: "网关重启任务失败".into(),
        },
    }
}

/// 更新若还需持锁安装，请直接在 with_connection 闭包内调用 gw.safe_shutdown。
pub fn safe_shutdown_blocking(
    app: &AppHandle,
    timeout: std::time::Duration,
) -> Result<bool, ConnectionError> {
    with_connection(app, |gw| gw.safe_shutdown(timeout))
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogsDirResult {
    Opened { path: String },
    Failed { detail: String },
}

#[tauri::command]
pub async fn open_logs_dir(app: AppHandle) -> LogsDirResult {
    let app_data = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(e) => {
            return LogsDirResult::Failed {
                detail: format!("解析应用数据目录失败：{e}"),
            }
        }
    };
    let dir = match gateway_control::ensure_logs_dir(&app_data) {
        Ok(d) => d,
        Err(detail) => return LogsDirResult::Failed { detail },
    };
    match std::process::Command::new("explorer").arg(&dir).spawn() {
        Ok(_) => LogsDirResult::Opened {
            path: dir.display().to_string(),
        },
        Err(e) => LogsDirResult::Failed {
            detail: format!("打开资源管理器失败：{e}"),
        },
    }
}

pub fn auto_start_on_launch(app: &AppHandle) {
    let _ = with_connection(app, |gw| {
        if crate::gateway_process::port_in_use(crate::gateway_process::GATEWAY_HOST, gw.port()?) {
            return Ok(());
        }
        gw.start().map(|_| ())
    });
}

fn resolve_runtime_paths(app: &AppHandle) -> Result<GatewayPaths, ConnectionError> {
    let exe = std::env::current_exe()
        .map_err(|_| ConnectionError::Failed("解析主程序目录失败".into()))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| ConnectionError::Failed("主程序目录异常".into()))?;
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|_| ConnectionError::Failed("解析应用数据目录失败".into()))?;
    Ok(resolve_paths(exe_dir, &app_data))
}
