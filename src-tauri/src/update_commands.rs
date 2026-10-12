//! Tauri 更新接线。唯一安装入口始终经过 app_update；不向前端开放 updater 插件权限。
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{
    app_update::{
        BootPlan, GatewayHost, GatewayIdentity, Release, ShutdownFailure, UpdateGateway,
        UpdateProvider, UpdateService, UpdateView,
    },
    gateway_commands::with_connection,
    gateway_connection::{ConnectionError, GatewayConnection},
    gateway_process::{self, ExternalStopTarget},
    gateway_status::{fetch_status_body, parse_status_pid},
};

type Updates = UpdateService<Update>;

#[cfg(all(test, feature = "updater-download-tests"))]
#[path = "update_commands_tests.rs"]
mod tests;

struct InstalledRelease<R: tauri::Runtime = tauri::Wry>(AppHandle<R>);
impl<R: tauri::Runtime> UpdateProvider for InstalledRelease<R> {
    type Candidate = Update;
    async fn check(&self) -> Result<Option<Release<Update>>, String> {
        use tauri::utils::config::BundleType;
        let target = match tauri::utils::platform::bundle_type() {
            Some(BundleType::Nsis) => "windows-x86_64-nsis",
            Some(BundleType::Msi) => "windows-x86_64-msi",
            _ => return Err("当前程序不是受支持的 Windows 安装版；请先手动安装一次。".into()),
        };
        let updater = self
            .0
            .updater_builder()
            .target(target)
            .timeout(Duration::from_secs(120))
            // 默认回调在 ShellExecuteW 之前销毁 Tauri；若安装器启动失败，补偿已无法使用应用。
            // 本模块在 install 之前完成停机和事务落盘，成功由插件直接 process::exit，失败保留旧应用。
            .on_before_exit(|| {})
            .build()
            .map_err(|_| "更新源配置无效，无法检查更新。")?;
        updater
            .check()
            .await
            .map(|release| {
                release.map(|candidate| Release {
                    version: candidate.version.clone(),
                    notes: candidate.body.clone().unwrap_or_default(),
                    candidate,
                })
            })
            .map_err(|_| "无法检查更新，请检查网络或稍后重试。".into())
    }
    async fn download(&self, candidate: &Update) -> Result<Vec<u8>, String> {
        // finish 回调在验签前发生！不得在回调中排空，只有这个 await 成功才返回字节。
        candidate
            .download(|_, _| {}, || {})
            .await
            .map_err(|_| "更新下载或签名校验失败。".into())
    }
    fn install(&self, candidate: &Update, bytes: &[u8]) -> Result<(), String> {
        candidate
            .install(bytes)
            .map_err(|_| "安装器启动失败。".into())
    }
}

struct ConnectedGateway(AppHandle);
impl GatewayHost for ConnectedGateway {
    type Gateway = GatewayConnection;
    fn with_gateway<T>(
        &self,
        action: impl FnOnce(&mut GatewayConnection) -> T,
    ) -> Result<T, String> {
        with_connection(&self.0, |gateway| Ok(action(gateway))).map_err(|e| e.to_string())
    }
}

impl UpdateGateway for GatewayConnection {
    fn identity(&mut self) -> Result<GatewayIdentity, String> {
        let settings = self.settings().map_err(|e| e.to_string())?;
        if settings.restart_required {
            return Err("连接设置尚未生效，请先安全重启网关后重新检查更新。".into());
        }
        let port = self.port().map_err(|e| e.to_string())?;
        let configuration =
            serde_json::to_string(&settings.saved).map_err(|_| "无法核实连接设置。")?;
        let Some(supervisor) = self.managed_supervisor_pid() else {
            // 状态请求失败不是停机证明；没有句柄时仅接受确实可绑定的本机端口。
            let listener = std::net::TcpListener::bind((gateway_process::GATEWAY_HOST, port))
                .map_err(|_| "端口上存在外部实例或无法核实归属；更新不会停止或管理外部安装。")?;
            drop(listener);
            return Ok(GatewayIdentity {
                running: false,
                token: configuration,
            });
        };
        let body = fetch_status_body(gateway_process::GATEWAY_HOST, port)
            .map_err(|_| "无法核实受管网关状态，未获准更新。")?;
        let core = parse_status_pid(&body).ok_or("无法核实网关进程身份。")?;
        if gateway_process::resolve_external_stop_target(core)
            != (ExternalStopTarget::Supervisor { pid: supervisor })
        {
            return Err("网关进程身份已变化，请稍后重新检查更新。".into());
        }
        Ok(GatewayIdentity {
            running: true,
            token: format!("{supervisor}:{core}:{configuration}"),
        })
    }
    fn safe_shutdown(&mut self, timeout: Duration) -> Result<bool, ShutdownFailure> {
        GatewayConnection::safe_shutdown(self, timeout).map_err(|error| match error {
            ConnectionError::Timeout => ShutdownFailure::Timeout,
            ConnectionError::Failed(detail) => ShutdownFailure::Refused(detail),
        })
    }
    fn restore(&mut self) -> Result<(), String> {
        // start 自己拒绝端口冲突，绝不杀掉更新期间出现的其它实例。
        self.start().map(|_| ()).map_err(|e| e.to_string())
    }
}

fn configured(app: &AppHandle) -> bool {
    let Some(config) = app.config().plugins.0.get("updater") else {
        return false;
    };
    config
        .get("pubkey")
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.trim().is_empty())
        && config
            .get("endpoints")
            .and_then(|v| v.as_array())
            .is_some_and(|v| !v.is_empty())
        && config.get("requireSignedVersion").and_then(|v| v.as_bool()) == Some(true)
}

/// setup 的唯一启动编排：先读取恢复凭据，再决定是否允许默认启动。不会改连接设置或日志。
pub fn initialize(app: &AppHandle) -> Result<(), String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|_| "无法解析更新恢复目录。")?;
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| "无法核实当前安装位置。")?;
    let installation = executable.to_string_lossy().to_lowercase();
    let updates = Updates::with_journal(
        configured(app),
        app.package_info().version.to_string(),
        installation,
        data.join("update-state.json"),
    );
    // setup 在默认启动之前完成恢复判断；初始化未完成时控制命令保持拒绝。
    let plan = updates.recover(&ConnectedGateway(app.clone()));
    if plan == BootPlan::Normal {
        crate::gateway_commands::auto_start_on_launch(app);
    }
    app.manage(updates);
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let updates = handle.state::<Updates>();
        let _ = tauri::async_runtime::block_on(updates.check(&InstalledRelease(handle.clone())));
    });
    Ok(())
}

pub fn acknowledge_manual_start(app: &AppHandle) -> Result<(), ConnectionError> {
    app.state::<Updates>()
        .acknowledge_manual_start()
        .map_err(ConnectionError::Failed)
}

pub fn controls_blocked(app: &AppHandle) -> bool {
    app.try_state::<Updates>()
        .is_none_or(|state| state.controls_blocked())
}

pub fn exit_authorized(app: &AppHandle) -> bool {
    app.try_state::<Updates>()
        .is_some_and(|state| state.exit_authorized())
}

/// 关闭窗口、托盘和面板退出均先走此入口；拒绝时保持窗口与进程句柄，不排队重试。
pub fn request_exit(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(updates) = app.try_state::<Updates>() else {
            return;
        };
        if updates.prepare_exit(&ConnectedGateway(app.clone())).is_ok() {
            app.exit(0);
        }
    });
}

#[tauri::command]
pub fn get_update_state(app: AppHandle) -> Result<UpdateView, String> {
    app.try_state::<Updates>()
        .map(|state| state.view())
        .ok_or_else(|| "更新模块初始化失败，请重新打开应用。".into())
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<UpdateView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Updates>();
        tauri::async_runtime::block_on(state.check(&InstalledRelease(app.clone())))
    })
    .await
    .map_err(|_| "更新检查任务失败。")?
}

#[tauri::command]
pub async fn confirm_update(app: AppHandle, candidate_id: String) -> Result<UpdateView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Updates>();
        tauri::async_runtime::block_on(state.confirm(
            &InstalledRelease(app.clone()),
            &ConnectedGateway(app.clone()),
            &candidate_id,
        ))
    })
    .await
    .map_err(|_| "更新任务失败；请核实网关状态。")?
}
