//! Tauri 更新接线。唯一安装入口始终经过 app_update；不向前端开放 updater 插件权限。
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{
    app_exit::{log_exit_outcome, log_exit_source, ExitOutcome, ExitSource},
    app_update::{
        BootPlan, DownloadEvent, ExitFailureReason, ExitPreparationFailure, GatewayHost,
        GatewayIdentity, Release, ShutdownFailure, UpdateGateway, UpdateProvider, UpdateService,
        UpdateView,
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
    async fn download(
        &self,
        candidate: &Update,
        progress: impl Fn(DownloadEvent) + Send + Sync,
    ) -> Result<Vec<u8>, String> {
        // finish 回调在验签前发生！不得在回调中排空，只有这个 await 成功才返回字节。
        candidate
            .download(
                |len, total| progress(DownloadEvent::Chunk { len, total }),
                || progress(DownloadEvent::Finished),
            )
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

/// 所有真正退出复用此等待型入口。同步抢占后才排入阻塞线程池，拒绝的请求不会稍后重试。
pub fn request_exit(
    app: &AppHandle,
    source: Option<ExitSource>,
) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
    if let Some(source) = source {
        log_exit_source(app, source);
    }
    let admitted = (|| {
        let state = app
            .try_state::<crate::app_exit::AppExitState>()
            .ok_or_else(|| {
                ExitPreparationFailure::new(
                    ExitFailureReason::ExitModuleNotReady,
                    "退出模块尚未就绪，请稍后重试。",
                )
            })?;
        let attempt = state.exits.try_begin().map_err(|detail| {
            ExitPreparationFailure::new(ExitFailureReason::ExitAlreadyRunning, detail)
        })?;
        let updates = app.try_state::<Updates>().ok_or_else(|| {
            ExitPreparationFailure::new(
                ExitFailureReason::UpdateModuleNotReady,
                "更新模块尚未就绪，暂不能安全退出。",
            )
        })?;
        if updates.controls_blocked() {
            return Err(ExitPreparationFailure::new(
                ExitFailureReason::OperationBusy,
                "更新或退出正在进行，请等待当前操作完成。",
            ));
        }
        Ok(attempt)
    })();
    if let Err(failure) = &admitted {
        log_exit_outcome(app, source, ExitOutcome::Failed(failure.reason));
    }
    let app = app.clone();
    async move {
        let attempt = admitted.map_err(|failure| failure.detail)?;
        let worker = app.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            // 资格及结果记录由实际任务持有，不因等待命令取消而提前放行或漏记。
            let _attempt = attempt;
            run_exit_task(
                || {
                    let updates = worker.try_state::<Updates>().ok_or_else(|| {
                        ExitPreparationFailure::new(
                            ExitFailureReason::UpdateModuleNotReady,
                            "更新模块尚未就绪，暂不能安全退出。",
                        )
                    })?;
                    // 原子互斥门裁决与更新之间的竞态，不依赖准入时的快照。
                    updates.prepare_exit(&ConnectedGateway(worker.clone()))?;
                    if !updates.exit_authorized() {
                        return Err(ExitPreparationFailure::new(
                            ExitFailureReason::ExitUnauthorized,
                            "退出未获安全授权，已保留当前窗口。",
                        ));
                    }
                    Ok(())
                },
                |outcome| log_exit_outcome(&worker, source, outcome),
                || worker.exit(0),
            )
        })
        .await
        .unwrap_or_else(|_| {
            log_exit_outcome(
                &app,
                source,
                ExitOutcome::Failed(ExitFailureReason::ExitTaskFailed),
            );
            Err("退出任务失败，已保留当前窗口，请重试。".into())
        })
    }
}

/// 实际退出任务的边界；进程退出与结果记录由外部提供，不依赖等待方存活。
pub(crate) fn run_exit_task(
    prepare: impl FnOnce() -> Result<(), ExitPreparationFailure>,
    mut record: impl FnMut(ExitOutcome),
    exit: impl FnOnce(),
) -> Result<(), String> {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepare()?;
        record(ExitOutcome::Authorized);
        exit();
        Ok(())
    }))
    .unwrap_or_else(|_| {
        Err(ExitPreparationFailure::new(
            ExitFailureReason::ExitTaskFailed,
            "退出任务失败，已保留当前窗口，请重试。",
        ))
    });
    match outcome {
        Ok(()) => Ok(()),
        Err(failure) => {
            record(ExitOutcome::Failed(failure.reason));
            Err(failure.detail)
        }
    }
}

#[cfg(test)]
mod exit_task_tests {
    use super::*;
    use crate::exit_requests::ExitRequests;
    use std::sync::mpsc;

    #[test]
    fn 取消等待后_普通失败与任务异常均留证并释放资格() {
        for panics in [false, true] {
            let exits = ExitRequests::default();
            let attempt = exits.try_begin().unwrap();
            let (started, ready) = mpsc::channel();
            let (resume, wait) = mpsc::channel();
            let (recorded, records) = mpsc::channel();
            let (completed, completion) = mpsc::channel();
            let task = tauri::async_runtime::spawn_blocking(move || {
                let result = {
                    let _attempt = attempt;
                    run_exit_task(
                        || {
                            started.send(()).unwrap();
                            wait.recv_timeout(Duration::from_secs(5)).unwrap();
                            if panics {
                                panic!("fixture-only exit task panic");
                            }
                            Err(ExitPreparationFailure::new(
                                ExitFailureReason::StatusUnverified,
                                "无法核实当前网关状态，未执行停机",
                            ))
                        },
                        |outcome| recorded.send(outcome).unwrap(),
                        || panic!("失败不能关闭应用"),
                    )
                };
                completed.send(result).unwrap();
            });
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(task);
            assert!(
                exits.try_begin().is_err(),
                "取消等待不能提前释放实际任务的资格"
            );
            resume.send(()).unwrap();
            let expected = if panics {
                ExitFailureReason::ExitTaskFailed
            } else {
                ExitFailureReason::StatusUnverified
            };
            assert_eq!(
                records.recv_timeout(Duration::from_secs(5)).unwrap(),
                ExitOutcome::Failed(expected)
            );
            let error = completion
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap_err();
            assert_eq!(
                error,
                if panics {
                    "退出任务失败，已保留当前窗口，请重试。"
                } else {
                    "无法核实当前网关状态，未执行停机"
                }
            );
            assert!(records.try_recv().is_err(), "同一失败不能重复记录");
            assert!(exits.try_begin().is_ok(), "实际任务结束后才允许显式重试");
        }
    }
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
