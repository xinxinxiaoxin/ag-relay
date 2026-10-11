//! Tauri 命令层：把网关进程生命周期暴露给前端按钮（与 UI 同一命令路径）。
//! 业务逻辑全部在 gateway_process（纯函数，可脱离 tauri 运行时测试），本层只做薄封装。

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::gateway_control::{self, ControlError, DEFAULT_RESTART_TIMEOUT};
use crate::gateway_process::{
    resolve_paths, stop_managed, GatewayError, GatewayPaths, ManagedChild, GATEWAY_HOST,
    GATEWAY_PORT, STOP_TIMEOUT,
};

/// 壳的全局网关状态：当前受管的 supervisor 子进程（None = 未由本壳启动）。
pub struct GatewayState {
    running: Mutex<Option<ManagedChild>>,
}

impl Default for GatewayState {
    fn default() -> Self {
        Self {
            running: Mutex::new(None),
        }
    }
}

impl GatewayState {
    fn take_for_stop(&self) -> Option<ManagedChild> {
        self.running.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    fn store(&self, child: ManagedChild) {
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    }
}

/// 命令层结构化结果（前端按 kind 分支给中文提示；失败必带 reason 枚举，不用裸字符串）。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GatewayActionResult {
    /// 网关已拉起（supervisor PID）
    Started { supervisor_pid: u32 },
    /// 本壳受管的网关已停止（graceful=优雅排空；false 表示 10 秒超时后强杀）
    Stopped { graceful: bool, exit_code: Option<i32> },
    /// 面板外启动的外部网关已停止（F1：本壳未受管但 8787 上核实到了可停的 supervisor）
    StoppedExternal { graceful: bool },
    /// 检测到疑似外部网关，但 pid 链无法核实（宁可不杀不可误杀），未执行停止
    ExternalStopAbandoned,
    /// 当前没有可识别的网关在运行（重复停止是安全的幂等操作）
    NotRunning,
    /// 启动/停止失败，reason 为机器可读枚举
    Failed { reason: GatewayFailureReason },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayFailureReason {
    NodeMissing,
    GatewayScriptMissing,
    PortAlreadyInUse,
    SpawnFailed,
    JobSetupFailed,
}

impl From<&GatewayError> for GatewayFailureReason {
    fn from(e: &GatewayError) -> Self {
        match e {
            GatewayError::NodeNotFound(_) => GatewayFailureReason::NodeMissing,
            GatewayError::SupervisorScriptNotFound(_) => GatewayFailureReason::GatewayScriptMissing,
            GatewayError::PortAlreadyInUse { .. } => GatewayFailureReason::PortAlreadyInUse,
            GatewayError::SpawnFailed(_) => GatewayFailureReason::SpawnFailed,
            GatewayError::JobSetupFailed(_) => GatewayFailureReason::JobSetupFailed,
        }
    }
}

fn failed(e: &GatewayError) -> GatewayActionResult {
    GatewayActionResult::Failed { reason: e.into() }
}

/// 启动网关（与 UI「启动网关」按钮同一命令）。
/// 端口被占（含已有实例在跑）时返回明确失败，不重复拉起。
#[tauri::command]
pub async fn start_gateway(app: AppHandle) -> GatewayActionResult {
    let state: State<GatewayState> = app.state();
    let paths = match resolve_runtime_paths(&app) {
        Ok(p) => p,
        Err(e) => return failed(&e),
    };

    match crate::gateway_process::start_gateway_process(&paths) {
        Ok(child) => {
            let supervisor_pid = child.supervisor_pid;
            state.store(child);
            GatewayActionResult::Started { supervisor_pid }
        }
        Err(e) => failed(&e),
    }
}

/// 停止网关（与 UI「停止网关」按钮同一命令）：CTRL_C 优雅排空 ≤10 秒，超时强杀整树。
/// F1：本壳没有受管网关时，8787 上若有外部网关（面板外启动的实例）同样优雅停止——
/// 「关了就彻底关了」。重复调用安全（无可识别网关时返回 NotRunning）。
#[tauri::command]
pub async fn stop_gateway(app: AppHandle) -> GatewayActionResult {
    let state: State<GatewayState> = app.state();
    stop_and_clear(&app, &state)
}

fn stop_and_clear(app: &AppHandle, state: &State<GatewayState>) -> GatewayActionResult {
    match state.take_for_stop() {
        Some(mut gw) => match stop_managed(&mut gw, STOP_TIMEOUT) {
            crate::gateway_process::StopOutcome::Graceful { exit_code } => {
                GatewayActionResult::Stopped { graceful: true, exit_code }
            }
            crate::gateway_process::StopOutcome::ForceKilled => {
                GatewayActionResult::Stopped { graceful: false, exit_code: None }
            }
        },
        // F1：无受管网关——8787 上有可核实的外部网关则优雅停止
        None => stop_external(app),
    }
}

/// 外部网关停止路径（F1）：识别→广播 CTRL_C→等≤10s→taskkill 兜底，
/// 结果按白名单记入壳侧日志（shell-exit.log 的 event=external_stop_*）。
fn stop_external(app: &AppHandle) -> GatewayActionResult {
    use crate::app_exit::ExternalStopEvent;
    use crate::gateway_process::ExternalStopOutcome;

    match crate::gateway_process::stop_external_gateway(GATEWAY_PORT, STOP_TIMEOUT) {
        ExternalStopOutcome::NoGateway => GatewayActionResult::NotRunning,
        ExternalStopOutcome::StoppedGraceful => {
            crate::app_exit::log_external_stop(app, ExternalStopEvent::Graceful);
            GatewayActionResult::StoppedExternal { graceful: true }
        }
        ExternalStopOutcome::ForceKilled => {
            crate::app_exit::log_external_stop(app, ExternalStopEvent::Forced);
            GatewayActionResult::StoppedExternal { graceful: false }
        }
        ExternalStopOutcome::Abandoned => {
            crate::app_exit::log_external_stop(app, ExternalStopEvent::Abandoned);
            GatewayActionResult::ExternalStopAbandoned
        }
    }
}

/// 安全重启结果（前端按 kind 给中文提示）。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RestartResult {
    /// 网关已受理重启（排空后由 supervisor 重新拉起）
    Restarted,
    /// 8787 上没有网关
    NotRunning,
    /// 等待网关响应超时（可能仍在排空在途请求）
    Timeout,
    /// 其他失败，detail 为人可读原因
    Failed { detail: String },
}

/// 安全重启（与 UI「安全重启」按钮同一命令）：POST /_relay/restart + 控制头，60 秒上限。
/// 不依赖网关是否由本壳启动——只要是 8787 上的网关即可重启。
#[tauri::command]
pub async fn restart_gateway() -> RestartResult {
    // 同步阻塞调用（最长 60 秒）放进阻塞线程池，避免占用命令执行器
    match tauri::async_runtime::spawn_blocking(move || {
        gateway_control::post_restart(GATEWAY_HOST, GATEWAY_PORT, DEFAULT_RESTART_TIMEOUT)
    })
    .await
    {
        Ok(Ok(())) => RestartResult::Restarted,
        Ok(Err(ControlError::NotRunning)) => RestartResult::NotRunning,
        Ok(Err(ControlError::Timeout)) => RestartResult::Timeout,
        Ok(Err(ControlError::Failed(detail))) => RestartResult::Failed { detail },
        Err(e) => RestartResult::Failed { detail: format!("内部任务失败：{e}") },
    }
}

/// 打开日志文件夹结果。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogsDirResult {
    /// 已在资源管理器中打开
    Opened { path: String },
    Failed { detail: String },
}

/// 打开日志文件夹（与 UI「打开日志文件夹」按钮同一命令）：
/// 目录与诊断日志一致（%APPDATA%\ag-relay\logs），不存在则先创建。
#[tauri::command]
pub async fn open_logs_dir(app: AppHandle) -> LogsDirResult {
    let app_data = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(e) => return LogsDirResult::Failed { detail: format!("解析应用数据目录失败：{e}") },
    };
    let dir = match gateway_control::ensure_logs_dir(&app_data) {
        Ok(d) => d,
        Err(detail) => return LogsDirResult::Failed { detail },
    };
    match std::process::Command::new("explorer").arg(&dir).spawn() {
        Ok(_) => LogsDirResult::Opened { path: dir.display().to_string() },
        Err(e) => LogsDirResult::Failed { detail: format!("打开资源管理器失败：{e}") },
    }
}

/// 应用退出钩子（RunEvent::Exit）：同步阻塞地停掉网关，≤10 秒。
/// 幂等：按钮已停过 / 从未启动过都是 no-op。F1：本壳没有受管网关但 8787 上有
/// 外部网关时同样优雅停止（关了就彻底关了）。刻意在 Exit 事件内做——
/// tauri 事件循环回调之后直接 std::process::exit，`.run()` 之后的代码不保证执行。
pub fn shutdown_blocking_on_exit(app: &AppHandle) {
    let state: State<GatewayState> = app.state();
    let _ = stop_and_clear(app, &state);
}

/// 应用启动钩子（lib.rs setup 调用）：规范生命周期「软件启动→网关未运行则拉起」。
/// 自启换新（T6）的目的就是「开机自启的是软件本身，它再拉起网关」，故随本票落地。
/// 8787 已被占（含面板外已有实例/别的壳实例）则直接接管：不重复拉起、不报错；
/// 失败静默（面板显示已停止，用户可手动启动），绝不阻塞窗口出现。
pub fn auto_start_on_launch(app: &AppHandle) {
    if crate::gateway_process::port_in_use(
        crate::gateway_process::GATEWAY_HOST,
        crate::gateway_process::GATEWAY_PORT,
    ) {
        return; // 已有网关在跑：接管语义，不重复拉起
    }
    let state: State<GatewayState> = app.state();
    let paths = match resolve_runtime_paths(app) {
        Ok(p) => p,
        Err(_) => return,
    };
    if let Ok(child) = crate::gateway_process::start_gateway_process(&paths) {
        state.store(child);
    }
}

/// 运行期路径解析：node.exe / gateway 脚本目录都在主程序 exe 旁，
/// 诊断日志在 %APPDATA%\ag-relay\logs\（identifier=ag-relay 决定）。
fn resolve_runtime_paths(app: &AppHandle) -> Result<GatewayPaths, GatewayError> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| GatewayError::SpawnFailed(format!("解析主程序目录失败：{e}")))?
        .parent()
        .ok_or_else(|| GatewayError::SpawnFailed("主程序目录异常".to_string()))?
        .to_path_buf();
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| GatewayError::SpawnFailed(format!("解析应用数据目录失败：{e}")))?;
    Ok(resolve_paths(&exe_dir, &app_data))
}
