//! AG Relay 更新的应用服务层。候选与已验签字节只保留在 Rust，前端只拿展示信息和确认编号。

pub use crate::update_journal::BootPlan;
use crate::update_journal::{Journal, Stage};
use serde::Serialize;
use std::future::Future;
use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Mutex,
};
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayIdentity {
    pub running: bool,
    pub token: String,
}
#[derive(Debug)]
pub enum ShutdownFailure {
    Timeout,
    Refused(String),
}

/// 进程 adapter 必须调用公共安全排空，不能以活动请求数为零或 202 代替完整退出。
pub trait UpdateGateway {
    fn identity(&mut self) -> Result<GatewayIdentity, String>;
    fn safe_shutdown(&mut self, timeout: Duration) -> Result<bool, ShutdownFailure>;
    fn restore(&mut self) -> Result<(), String>;
}
pub trait GatewayHost {
    type Gateway: UpdateGateway;
    /// 持生命周期独占直到 action 返回；忙时拒绝，不排队执行过期操作。
    fn with_gateway<T>(&self, action: impl FnOnce(&mut Self::Gateway) -> T) -> Result<T, String>;
}

pub struct Release<C> {
    pub candidate: C,
    pub version: String,
    pub notes: String,
}

pub enum DownloadEvent {
    Chunk { len: usize, total: Option<u64> },
    Finished,
}

/// 发布端点/签名下载/安装器交接的 adapter。download 只能在完整验签成功后返回字节。
pub trait UpdateProvider {
    type Candidate: Clone + Send + Sync;
    fn check(
        &self,
    ) -> impl Future<Output = Result<Option<Release<Self::Candidate>>, String>> + Send;
    fn download(
        &self,
        candidate: &Self::Candidate,
        progress: impl Fn(DownloadEvent) + Send + Sync,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + Send;
    fn install(&self, candidate: &Self::Candidate, bytes: &[u8]) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Disabled,
    Idle,
    Checking,
    Available,
    Downloading,
    Verifying,
    Verified,
    Quiescing,
    InstallPending,
    Failed,
    Deferred,
    NeedsAttention,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    pub id: String,
    pub version: String,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    pub phase: UpdatePhase,
    pub current_version: String,
    pub release: Option<ReleaseInfo>,
    pub download: Option<DownloadProgress>,
    pub detail: String,
    pub recovery: Option<String>,
}

struct UpdateState<C> {
    view: UpdateView,
    candidate: Option<C>,
}

// 更新/退出操作的原子闸门状态。以 AtomicU8 承载保持无锁原子语义，枚举化取代表意不明的 0/1/2/3。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum OperationState {
    /// 可操作：没有更新或退出在进行。
    Idle = 0,
    /// 检查/确认/退出排空中：新操作被拒绝，控件封锁。
    Busy = 1,
    /// 已安全停止并允许退出。
    ExitAuthorized = 2,
    /// 安装器已交接：即使假适配器返回也必须保持封锁，不能重新开启控制。
    Committing = 3,
}

impl OperationState {
    fn load(atomic: &AtomicU8) -> Self {
        match atomic.load(Ordering::Acquire) {
            1 => Self::Busy,
            2 => Self::ExitAuthorized,
            3 => Self::Committing,
            _ => Self::Idle,
        }
    }
    fn store(self, atomic: &AtomicU8) {
        atomic.store(self as u8, Ordering::Release);
    }
    /// 仅当当前为 Idle 时置为 Busy；失败表示已有操作在进行。
    fn try_begin(atomic: &AtomicU8) -> bool {
        atomic
            .compare_exchange(
                Self::Idle as u8,
                Self::Busy as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    /// 仅当当前为 Busy 时置回 Idle。
    fn end(atomic: &AtomicU8) {
        let _ = atomic.compare_exchange(
            Self::Busy as u8,
            Self::Idle as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

struct Operation<'a>(&'a AtomicU8);
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        OperationState::end(self.0);
    }
}
pub struct UpdateService<C> {
    state: Mutex<UpdateState<C>>,
    journal: Option<Journal>,
    operation: AtomicU8,
    enabled: bool,
}

impl<C: Clone + Send + Sync> UpdateService<C> {
    pub fn new(enabled: bool, current_version: String) -> Self {
        Self {
            state: Mutex::new(UpdateState {
                view: UpdateView {
                    phase: if enabled {
                        UpdatePhase::Idle
                    } else {
                        UpdatePhase::Disabled
                    },
                    current_version,
                    release: None,
                    download: None,
                    detail: if enabled {
                        "尚未检查更新。"
                    } else {
                        "当前构建未配置更新源"
                    }
                    .into(),
                    recovery: None,
                },
                candidate: None,
            }),
            journal: None,
            operation: AtomicU8::new(OperationState::Idle as u8),
            enabled,
        }
    }

    fn begin_operation(&self) -> Result<Operation<'_>, String> {
        if OperationState::try_begin(&self.operation) {
            Ok(Operation(&self.operation))
        } else {
            Err("更新或退出正在进行，请等待当前操作完成。".into())
        }
    }

    pub fn controls_blocked(&self) -> bool {
        OperationState::load(&self.operation) != OperationState::Idle
    }
    pub fn exit_authorized(&self) -> bool {
        OperationState::load(&self.operation) == OperationState::ExitAuthorized
    }

    pub fn prepare_exit(&self, host: &impl GatewayHost) -> Result<(), String> {
        let _operation = self.begin_operation()?;
        let outcome = host.with_gateway(|g| g.safe_shutdown(Duration::from_secs(10)))?;
        match outcome {
            Ok(_) => {
                OperationState::ExitAuthorized.store(&self.operation);
                Ok(())
            }
            Err(_) => {
                let detail = "退出已取消：安全排空尚未完成，未强制结束网关，请稍后重试。";
                self.set_phase(UpdatePhase::Failed, detail);
                Err(detail.into())
            }
        }
    }

    pub fn with_journal(
        enabled: bool,
        version: String,
        installation: String,
        path: PathBuf,
    ) -> Self {
        let mut service = Self::new(enabled, version.clone());
        service.journal = Some(Journal::new(path, version, installation));
        service
    }

    /// 仅显式「启动网关」调用，用户意图取代更新前停止覆盖；失败时不得继续启动。
    pub fn acknowledge_manual_start(&self) -> Result<(), String> {
        let _operation = self.begin_operation()?;
        if let Some(journal) = &self.journal {
            journal.clear()?;
        }
        self.state.lock().unwrap().view.recovery = None;
        Ok(())
    }

    pub fn boot_plan(&self) -> BootPlan {
        self.journal
            .as_ref()
            .map_or(BootPlan::NeedsAttention, Journal::boot_plan)
    }

    /// 必须先于默认启动调用。版本/安装位置不匹配及未知阶段都保守禁止自动启动。
    pub fn recover(&self, host: &impl GatewayHost) -> BootPlan {
        let plan = self.boot_plan();
        if plan == BootPlan::Normal {
            return plan;
        }
        let result = (|| -> Result<(), String> {
            if plan == BootPlan::NeedsAttention {
                return Err("上次更新结果尚未确认（版本、安装位置或恢复记录不匹配），未自动启动网关。请检查后手动启动。".into());
            }
            let _operation = self.begin_operation()?;
            let journal = self.journal.as_ref().ok_or("更新恢复记录不可用。")?;
            let mut record = journal.read()?.ok_or("更新恢复记录不存在。")?;
            if plan == BootPlan::RestoreRunning {
                host.with_gateway(|gateway| {
                    if !gateway.identity()?.running {
                        gateway.restore()?;
                    }
                    Ok::<_, String>(())
                })??;
            }
            if record.stage == Stage::InstallPending {
                record.stage = Stage::Restored;
                journal.write(&record)?;
            }
            let note = if record.stage == Stage::Aborted {
                "上次更新未安装，已保留原来的网关停止状态。".into()
            } else {
                format!(
                    "已更新到 v{}，{}。",
                    self.view().current_version,
                    if record.was_running {
                        "已恢复网关运行"
                    } else {
                        "保持网关停止，不执行默认启动"
                    }
                )
            };
            self.state.lock().unwrap().view.recovery = Some(note);
            Ok(())
        })();
        if let Err(detail) = result {
            self.state.lock().unwrap().view.recovery = Some(detail.clone());
            self.set_phase(UpdatePhase::NeedsAttention, detail);
            BootPlan::NeedsAttention
        } else {
            plan
        }
    }

    fn set_phase(&self, phase: UpdatePhase, detail: impl Into<String>) -> UpdateView {
        let mut state = self.state.lock().unwrap();
        state.view.phase = phase;
        state.view.detail = detail.into();
        state.view.clone()
    }

    pub async fn confirm(
        &self,
        provider: &impl UpdateProvider<Candidate = C>,
        host: &impl GatewayHost,
        id: &str,
    ) -> Result<UpdateView, String> {
        let _operation = self.begin_operation()?;
        if !matches!(
            self.view().phase,
            UpdatePhase::Available | UpdatePhase::Deferred
        ) {
            return Err("更新候选不可确认，请重新检查。".into());
        }
        let result = self.confirm_inner(provider, host, id).await;
        if let Err(detail) = &result {
            if !matches!(
                self.view().phase,
                UpdatePhase::Failed | UpdatePhase::NeedsAttention
            ) {
                self.set_phase(UpdatePhase::Failed, detail.clone());
            }
        }
        result
    }

    async fn confirm_inner(
        &self,
        provider: &impl UpdateProvider<Candidate = C>,
        host: &impl GatewayHost,
        id: &str,
    ) -> Result<UpdateView, String> {
        let (candidate, info) = {
            let state = self.state.lock().unwrap();
            let info = state
                .view
                .release
                .as_ref()
                .filter(|r| r.id == id)
                .ok_or("更新候选已失效，请重新检查。")?;
            (
                state.candidate.clone().ok_or("没有可下载的更新。")?,
                info.clone(),
            )
        };
        let original = host.with_gateway(|g| g.identity())??;
        {
            let mut state = self.state.lock().unwrap();
            state.view.phase = UpdatePhase::Downloading;
            state.view.detail = "正在下载更新包；网关保持原状。".into();
            state.view.download = Some(DownloadProgress {
                downloaded_bytes: 0,
                total_bytes: None,
            });
        }
        let bytes = match provider
            .download(&candidate, |event| {
                let mut state = self.state.lock().unwrap();
                match event {
                    DownloadEvent::Chunk { len, total } => {
                        let progress = state.view.download.as_mut().unwrap();
                        progress.downloaded_bytes += len as u64;
                        progress.total_bytes =
                            total.filter(|bytes| *bytes > 0 && *bytes >= progress.downloaded_bytes);
                    }
                    DownloadEvent::Finished => {
                        state.view.phase = UpdatePhase::Verifying;
                        state.view.detail = "下载完成，正在校验更新包签名；网关保持原状。".into();
                    }
                }
            })
            .await
        {
            Ok(bytes) => bytes,
            Err(_) => {
                let view = self.set_phase(
                    UpdatePhase::Failed,
                    "下载或签名校验失败，未停止网关；请重新检查更新。",
                );
                return Err(view.detail);
            }
        };
        self.set_phase(
            UpdatePhase::Verified,
            "更新包已完整验签，正在核实安全停机条件。",
        );
        let journal = self.journal.as_ref().ok_or("无法持久化更新恢复记录。")?;
        host.with_gateway(|gateway| {
            if gateway.identity()? != original {
                return Err("下载期间网关身份已变化，请重新确认。".into());
            }
            let mut transaction = journal.transaction(info.version, original.running);
            journal.write(&transaction)?;
            self.set_phase(
                UpdatePhase::Quiescing,
                "正在优雅排空在途请求，超时将取消安装。",
            );
            // 原来已停止时绝不调用可发现外部实例的停机链，避免停掉随后出现的新实例。
            let stopped = if original.running {
                gateway.safe_shutdown(Duration::from_secs(10))
            } else {
                Ok(false)
            };
            match stopped {
                Ok(was_running) if was_running == original.running => {}
                Ok(_) => return Err("未证实原网关完整退出，拒绝安装。".into()),
                Err(ShutdownFailure::Timeout) => {
                    transaction.stage = Stage::Aborted;
                    journal.write(&transaction)?;
                    return Ok(self.set_phase(
                        UpdatePhase::Deferred,
                        "排空超时，网关已恢复接单；未安装，请稍后重试。",
                    ));
                }
                Err(ShutdownFailure::Refused(detail)) => return Err(detail),
            }
            if gateway.identity()?.running {
                return Err("网关仍在运行，拒绝安装。".into());
            }
            transaction.stage = Stage::InstallPending;
            if journal.write(&transaction).is_err() {
                if original.running {
                    let _ = gateway.restore();
                }
                let view = self.set_phase(
                    UpdatePhase::NeedsAttention,
                    "安全停机后的恢复记录写入失败，未安装；已尝试恢复原运行态，请检查网关状态。",
                );
                return Err(view.detail);
            }
            self.set_phase(
                UpdatePhase::InstallPending,
                "网关已安全停止，正在交接安装器；安装完成后请重开应用。",
            );
            if provider.install(&candidate, &bytes).is_err() {
                transaction.stage = Stage::Aborted;
                let persisted = journal.write(&transaction);
                let restored = if original.running {
                    gateway.restore()
                } else {
                    Ok(())
                };
                let view = if persisted.is_err() || restored.is_err() {
                    self.set_phase(
                        UpdatePhase::NeedsAttention,
                        "安装器未启动，恢复原状态未完整完成；请检查网关状态后手动处理。",
                    )
                } else {
                    self.set_phase(
                        UpdatePhase::Failed,
                        "安装器未启动，已恢复更新前的网关状态；请重新检查更新。",
                    )
                };
                return Err(view.detail);
            }
            // Windows 真适配器成功后直接 process::exit；假适配器返回时也必须保持封锁。
            OperationState::Committing.store(&self.operation);
            Ok(self.view())
        })?
    }

    pub fn view(&self) -> UpdateView {
        self.state.lock().unwrap().view.clone()
    }

    pub async fn check(
        &self,
        provider: &impl UpdateProvider<Candidate = C>,
    ) -> Result<UpdateView, String> {
        let _operation = self.begin_operation()?;
        if !self.enabled {
            return Ok(self.set_phase(UpdatePhase::Disabled, "当前构建未配置更新源"));
        }
        self.set_phase(UpdatePhase::Checking, "正在检查 AG Relay 更新…");
        {
            let mut state = self.state.lock().unwrap();
            state.candidate = None;
            state.view.release = None;
            state.view.download = None;
        }
        let result = match provider.check().await {
            Ok(result) => result,
            Err(detail) => {
                self.set_phase(UpdatePhase::Failed, detail.clone());
                return Err(detail);
            }
        };
        let mut state = self.state.lock().unwrap();
        if let Some(release) = result {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            state.view.release = Some(ReleaseInfo {
                id: NEXT.fetch_add(1, Ordering::Relaxed).to_string(),
                version: release.version,
                notes: release.notes,
            });
            state.candidate = Some(release.candidate);
            state.view.phase = UpdatePhase::Available;
            state.view.detail = "发现新版，确认后才会下载与安装。".into();
        } else {
            state.view.phase = UpdatePhase::Idle;
            state.view.detail = "当前已是最新版本。".into();
        }
        Ok(state.view.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ReleaseServer {
        downloads: AtomicUsize,
    }

    impl UpdateProvider for ReleaseServer {
        type Candidate = ();
        async fn check(&self) -> Result<Option<Release<()>>, String> {
            Ok(Some(Release {
                candidate: (),
                version: "0.3.0".into(),
                notes: "安全更新说明".into(),
            }))
        }
        async fn download(
            &self,
            _: &(),
            _: impl Fn(DownloadEvent) + Send + Sync,
        ) -> Result<Vec<u8>, String> {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            Ok(vec![1, 2, 3])
        }
        fn install(&self, _: &(), _: &[u8]) -> Result<(), String> {
            panic!("未确认绝不能安装")
        }
    }

    struct SandboxGateway {
        running: bool,
    }
    impl UpdateGateway for SandboxGateway {
        fn identity(&mut self) -> Result<GatewayIdentity, String> {
            Ok(GatewayIdentity {
                running: self.running,
                token: "受管测试实例".into(),
            })
        }
        fn safe_shutdown(&mut self, _: std::time::Duration) -> Result<bool, ShutdownFailure> {
            let running = self.running;
            self.running = false;
            Ok(running)
        }
        fn restore(&mut self) -> Result<(), String> {
            self.running = true;
            Ok(())
        }
    }
    struct Sandbox<G = SandboxGateway>(Mutex<G>);
    impl<G: UpdateGateway> GatewayHost for Sandbox<G> {
        type Gateway = G;
        fn with_gateway<T>(&self, f: impl FnOnce(&mut G) -> T) -> Result<T, String> {
            Ok(f(&mut self.0.lock().unwrap()))
        }
    }

    #[test]
    fn 下载进度来自分块字节_下载完成只进入验签且不停止网关() {
        struct Observed<'a> {
            updates: &'a UpdateService<()>,
            host: &'a Sandbox,
        }
        impl UpdateProvider for Observed<'_> {
            type Candidate = ();
            async fn check(&self) -> Result<Option<Release<()>>, String> {
                ReleaseServer {
                    downloads: AtomicUsize::new(0),
                }
                .check()
                .await
            }
            async fn download(
                &self,
                _: &(),
                progress: impl Fn(DownloadEvent) + Send + Sync,
            ) -> Result<Vec<u8>, String> {
                assert_eq!(self.updates.view().download.unwrap().downloaded_bytes, 0);
                progress(DownloadEvent::Chunk {
                    len: 25,
                    total: Some(100),
                });
                progress(DownloadEvent::Chunk {
                    len: 15,
                    total: Some(100),
                });
                let view = self.updates.view();
                assert_eq!(view.phase, UpdatePhase::Downloading);
                assert_eq!(
                    view.download,
                    Some(DownloadProgress {
                        downloaded_bytes: 40,
                        total_bytes: Some(100)
                    })
                );
                progress(DownloadEvent::Chunk {
                    len: 60,
                    total: Some(100),
                });
                progress(DownloadEvent::Finished);
                let view = self.updates.view();
                assert_eq!(view.phase, UpdatePhase::Verifying);
                assert_eq!(view.download.unwrap().downloaded_bytes, 100);
                assert!(self.host.0.lock().unwrap().running);
                Err("隔离测试：验签失败".into())
            }
            fn install(&self, _: &(), _: &[u8]) -> Result<(), String> {
                panic!("验签失败不能安装")
            }
        }
        let updates = UpdateService::new(true, "0.3.3".into());
        let host = Sandbox(Mutex::new(SandboxGateway { running: true }));
        let provider = Observed {
            updates: &updates,
            host: &host,
        };
        let view = tauri::async_runtime::block_on(updates.check(&provider)).unwrap();
        assert!(tauri::async_runtime::block_on(updates.confirm(
            &provider,
            &host,
            &view.release.unwrap().id
        ))
        .is_err());
        assert_eq!(updates.view().phase, UpdatePhase::Failed);
        assert!(host.0.lock().unwrap().running);
    }

    #[test]
    fn 未知与不可信总量不提供比例_重查和重试清空旧进度() {
        struct Unknown<'a> {
            updates: &'a UpdateService<()>,
            total: Option<u64>,
        }
        impl UpdateProvider for Unknown<'_> {
            type Candidate = ();
            async fn check(&self) -> Result<Option<Release<()>>, String> {
                ReleaseServer {
                    downloads: AtomicUsize::new(0),
                }
                .check()
                .await
            }
            async fn download(
                &self,
                _: &(),
                progress: impl Fn(DownloadEvent) + Send + Sync,
            ) -> Result<Vec<u8>, String> {
                assert_eq!(
                    self.updates.view().download,
                    Some(DownloadProgress {
                        downloaded_bytes: 0,
                        total_bytes: None
                    })
                );
                progress(DownloadEvent::Chunk {
                    len: 3,
                    total: self.total,
                });
                assert_eq!(
                    self.updates.view().download,
                    Some(DownloadProgress {
                        downloaded_bytes: 3,
                        total_bytes: None
                    })
                );
                Err("隔离测试：下载中断".into())
            }
            fn install(&self, _: &(), _: &[u8]) -> Result<(), String> {
                panic!("下载中断不能安装")
            }
        }
        let updates = UpdateService::new(true, "0.3.3".into());
        let host = Sandbox(Mutex::new(SandboxGateway { running: true }));
        for total in [None, Some(0), Some(2)] {
            let provider = Unknown {
                updates: &updates,
                total,
            };
            let view = tauri::async_runtime::block_on(updates.check(&provider)).unwrap();
            assert!(view.download.is_none());
            assert!(tauri::async_runtime::block_on(updates.confirm(
                &provider,
                &host,
                &view.release.unwrap().id
            ))
            .is_err());
            assert!(host.0.lock().unwrap().running);
        }
    }

    #[test]
    fn 用户确认后完整验签再停机_安装前持久化恢复运行态() {
        struct Installer;
        impl UpdateProvider for Installer {
            type Candidate = ();
            async fn check(&self) -> Result<Option<Release<()>>, String> {
                ReleaseServer {
                    downloads: AtomicUsize::new(0),
                }
                .check()
                .await
            }
            async fn download(
                &self,
                _: &(),
                _: impl Fn(DownloadEvent) + Send + Sync,
            ) -> Result<Vec<u8>, String> {
                Ok(vec![1, 2, 3])
            }
            fn install(&self, _: &(), bytes: &[u8]) -> Result<(), String> {
                assert_eq!(bytes, [1, 2, 3]);
                Ok(()) // 仅模拟安装器交接，不代表安装成功。
            }
        }
        let root =
            std::env::temp_dir().join(format!("ag-relay-update-{}-happy", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let updates = UpdateService::with_journal(
            true,
            "0.2.0".into(),
            "隔离安装".into(),
            root.join("update.json"),
        );
        let host = Sandbox(Mutex::new(SandboxGateway { running: true }));
        let view = tauri::async_runtime::block_on(updates.check(&Installer)).unwrap();
        let view = tauri::async_runtime::block_on(updates.confirm(
            &Installer,
            &host,
            &view.release.unwrap().id,
        ))
        .unwrap();
        assert_eq!(view.phase, UpdatePhase::InstallPending);
        assert!(!host.0.lock().unwrap().running);
        let reopened = UpdateService::<()>::with_journal(
            true,
            "0.3.0".into(),
            "隔离安装".into(),
            root.join("update.json"),
        );
        assert_eq!(reopened.boot_plan(), BootPlan::RestoreRunning);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 下载完成但验签失败_拒绝安装且网关继续运行() {
        struct Tampered;
        impl UpdateProvider for Tampered {
            type Candidate = ();
            async fn check(&self) -> Result<Option<Release<()>>, String> {
                ReleaseServer {
                    downloads: AtomicUsize::new(0),
                }
                .check()
                .await
            }
            async fn download(
                &self,
                _: &(),
                _: impl Fn(DownloadEvent) + Send + Sync,
            ) -> Result<Vec<u8>, String> {
                // 与插件时序一致：下载完成通知并不代表验签成功。
                Err("签名校验失败".into())
            }
            fn install(&self, _: &(), _: &[u8]) -> Result<(), String> {
                panic!("篡改包不能安装")
            }
        }
        let updates = UpdateService::new(true, "0.2.0".into());
        let host = Sandbox(Mutex::new(SandboxGateway { running: true }));
        let view = tauri::async_runtime::block_on(updates.check(&Tampered)).unwrap();
        let result = tauri::async_runtime::block_on(updates.confirm(
            &Tampered,
            &host,
            &view.release.unwrap().id,
        ));
        assert!(result.is_err());
        assert!(host.0.lock().unwrap().running);
        assert_eq!(updates.view().phase, UpdatePhase::Failed);
        assert!(updates.view().detail.contains("未停止网关"));
    }

    #[test]
    fn 排空超时保留网关_稍后重试且不留下安装许可() {
        struct LongRequest;
        impl UpdateGateway for LongRequest {
            fn identity(&mut self) -> Result<GatewayIdentity, String> {
                Ok(GatewayIdentity {
                    running: true,
                    token: "长请求实例".into(),
                })
            }
            fn safe_shutdown(&mut self, _: Duration) -> Result<bool, ShutdownFailure> {
                Err(ShutdownFailure::Timeout)
            }
            fn restore(&mut self) -> Result<(), String> {
                panic!("超时已恢复接单，不应重新拉起")
            }
        }
        let root = std::env::temp_dir().join(format!(
            "ag-relay-update-{}",
            crate::update_journal::unique_id()
        ));
        let updates = UpdateService::with_journal(
            true,
            "0.2.0".into(),
            "隔离安装".into(),
            root.join("update.json"),
        );
        let server = ReleaseServer {
            downloads: AtomicUsize::new(0),
        };
        let view = tauri::async_runtime::block_on(updates.check(&server)).unwrap();
        let view = tauri::async_runtime::block_on(updates.confirm(
            &server,
            &Sandbox(Mutex::new(LongRequest)),
            &view.release.unwrap().id,
        ))
        .unwrap();
        assert_eq!(view.phase, UpdatePhase::Deferred);
        assert!(view.detail.contains("恢复接单"));
        assert_eq!(updates.boot_plan(), BootPlan::Normal);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 安装器启动失败恢复原运行态_保留设置与日志() {
        struct LaunchDenied;
        impl UpdateProvider for LaunchDenied {
            type Candidate = ();
            async fn check(&self) -> Result<Option<Release<()>>, String> {
                ReleaseServer {
                    downloads: AtomicUsize::new(0),
                }
                .check()
                .await
            }
            async fn download(
                &self,
                _: &(),
                _: impl Fn(DownloadEvent) + Send + Sync,
            ) -> Result<Vec<u8>, String> {
                Ok(vec![1, 2, 3])
            }
            fn install(&self, _: &(), _: &[u8]) -> Result<(), String> {
                Err("安装器启动失败".into())
            }
        }
        let root = std::env::temp_dir().join(format!(
            "ag-relay-update-{}",
            crate::update_journal::unique_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("connection.json"), b"retained settings").unwrap();
        std::fs::write(root.join("diagnostics.jsonl"), b"retained logs").unwrap();
        let updates = UpdateService::with_journal(
            true,
            "0.2.0".into(),
            "隔离安装".into(),
            root.join("update.json"),
        );
        let host = Sandbox(Mutex::new(SandboxGateway { running: true }));
        let view = tauri::async_runtime::block_on(updates.check(&LaunchDenied)).unwrap();
        assert!(tauri::async_runtime::block_on(updates.confirm(
            &LaunchDenied,
            &host,
            &view.release.unwrap().id
        ))
        .is_err());
        assert!(host.0.lock().unwrap().running);
        assert_eq!(updates.boot_plan(), BootPlan::Normal);
        assert_eq!(
            std::fs::read(root.join("connection.json")).unwrap(),
            b"retained settings"
        );
        assert_eq!(
            std::fs::read(root.join("diagnostics.jsonl")).unwrap(),
            b"retained logs"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 停机前后写盘失败都不安装_停机后失败补偿恢复() {
        struct DiskFaultGateway {
            path: PathBuf,
            running: bool,
            break_after_stop: bool,
        }
        impl UpdateGateway for DiskFaultGateway {
            fn identity(&mut self) -> Result<GatewayIdentity, String> {
                Ok(GatewayIdentity {
                    running: self.running,
                    token: "隔离受管实例".into(),
                })
            }
            fn safe_shutdown(&mut self, _: Duration) -> Result<bool, ShutdownFailure> {
                self.running = false;
                if self.break_after_stop {
                    std::fs::remove_file(&self.path).unwrap();
                    std::fs::create_dir(&self.path).unwrap();
                }
                Ok(true)
            }
            fn restore(&mut self) -> Result<(), String> {
                self.running = true;
                Ok(())
            }
        }
        for break_after_stop in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "ag-relay-update-{}",
                crate::update_journal::unique_id()
            ));
            let path = root.join("update.json");
            if !break_after_stop {
                std::fs::create_dir_all(&path).unwrap();
            }
            let updates =
                UpdateService::with_journal(true, "0.2.0".into(), "隔离安装".into(), path.clone());
            let host = Sandbox(Mutex::new(DiskFaultGateway {
                path,
                running: true,
                break_after_stop,
            }));
            let server = ReleaseServer {
                downloads: AtomicUsize::new(0),
            };
            let view = tauri::async_runtime::block_on(updates.check(&server)).unwrap();
            assert!(tauri::async_runtime::block_on(updates.confirm(
                &server,
                &host,
                &view.release.unwrap().id
            ))
            .is_err());
            assert!(host.0.lock().unwrap().running);
            assert!(matches!(
                updates.view().phase,
                UpdatePhase::Failed | UpdatePhase::NeedsAttention
            ));
            assert_eq!(updates.boot_plan(), BootPlan::NeedsAttention);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn 并发重复确认与退出被拒绝_不会再下载或提前停机() {
        use std::sync::mpsc;
        struct Waiting {
            started: mpsc::Sender<()>,
            resume: Mutex<mpsc::Receiver<()>>,
        }
        impl UpdateProvider for Waiting {
            type Candidate = ();
            async fn check(&self) -> Result<Option<Release<()>>, String> {
                ReleaseServer {
                    downloads: AtomicUsize::new(0),
                }
                .check()
                .await
            }
            async fn download(
                &self,
                _: &(),
                _: impl Fn(DownloadEvent) + Send + Sync,
            ) -> Result<Vec<u8>, String> {
                self.started.send(()).unwrap();
                self.resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
                Err("模拟下载中断".into())
            }
            fn install(&self, _: &(), _: &[u8]) -> Result<(), String> {
                panic!("未完成下载不可安装")
            }
        }
        let (started, ready) = mpsc::channel();
        let (resume, wait) = mpsc::channel();
        let server = Waiting {
            started,
            resume: Mutex::new(wait),
        };
        let duplicate = ReleaseServer {
            downloads: AtomicUsize::new(0),
        };
        let updates = UpdateService::new(true, "0.2.0".into());
        let host = Sandbox(Mutex::new(SandboxGateway { running: true }));
        let id = tauri::async_runtime::block_on(updates.check(&server))
            .unwrap()
            .release
            .unwrap()
            .id;
        std::thread::scope(|scope| {
            let first = scope
                .spawn(|| tauri::async_runtime::block_on(updates.confirm(&server, &host, &id)));
            ready.recv_timeout(Duration::from_secs(10)).unwrap();
            let blocked = updates.controls_blocked();
            let second = tauri::async_runtime::block_on(updates.confirm(&duplicate, &host, &id));
            let exit = updates.prepare_exit(&host);
            resume.send(()).unwrap();
            assert!(first.join().unwrap().is_err());
            assert!(blocked);
            assert!(second.is_err());
            assert!(exit.is_err());
        });
        assert_eq!(duplicate.downloads.load(Ordering::SeqCst), 0);
        assert!(host.0.lock().unwrap().running);
        assert!(!updates.controls_blocked());
        assert!(!updates.exit_authorized());
    }

    struct SuccessfulInstaller;
    impl UpdateProvider for SuccessfulInstaller {
        type Candidate = ();
        async fn check(&self) -> Result<Option<Release<()>>, String> {
            ReleaseServer {
                downloads: AtomicUsize::new(0),
            }
            .check()
            .await
        }
        async fn download(
            &self,
            _: &(),
            _: impl Fn(DownloadEvent) + Send + Sync,
        ) -> Result<Vec<u8>, String> {
            Ok(vec![1, 2, 3])
        }
        fn install(&self, _: &(), _: &[u8]) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn 版本匹配才恢复_停止态不被默认启动覆盖_重复恢复幂等() {
        for running in [true, false] {
            let root = std::env::temp_dir().join(format!(
                "ag-relay-update-{}",
                crate::update_journal::unique_id()
            ));
            let path = root.join("update.json");
            let updates =
                UpdateService::with_journal(true, "0.2.0".into(), "隔离安装".into(), path.clone());
            let host = Sandbox(Mutex::new(SandboxGateway { running }));
            let id = tauri::async_runtime::block_on(updates.check(&SuccessfulInstaller))
                .unwrap()
                .release
                .unwrap()
                .id;
            tauri::async_runtime::block_on(updates.confirm(&SuccessfulInstaller, &host, &id))
                .unwrap();
            assert_eq!(updates.boot_plan(), BootPlan::NeedsAttention); // 仍是旧版：不能冒充成功。
            assert!(updates.controls_blocked()); // 模拟交接返回也不能重新开启控制。
            let wrong_install = UpdateService::<()>::with_journal(
                true,
                "0.3.0".into(),
                "另一个安装".into(),
                path.clone(),
            );
            assert_eq!(wrong_install.recover(&host), BootPlan::NeedsAttention);
            assert!(!host.0.lock().unwrap().running);
            let reopened =
                UpdateService::<()>::with_journal(true, "0.3.0".into(), "隔离安装".into(), path);
            let expected = if running {
                BootPlan::RestoreRunning
            } else {
                BootPlan::KeepStopped
            };
            assert_eq!(reopened.recover(&host), expected);
            assert_eq!(host.0.lock().unwrap().running, running);
            host.0.lock().unwrap().running = false; // 模拟之后用户主动停止，消费过的恢复不可再次拉起。
            assert_eq!(
                reopened.recover(&host),
                if running {
                    BootPlan::Normal
                } else {
                    BootPlan::KeepStopped
                }
            );
            assert!(!host.0.lock().unwrap().running);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn 未证实完整停机不能进入安装_拒绝仅受理或仍存活实例() {
        struct StillAlive;
        impl UpdateGateway for StillAlive {
            fn identity(&mut self) -> Result<GatewayIdentity, String> {
                Ok(GatewayIdentity {
                    running: true,
                    token: "仍存活".into(),
                })
            }
            fn safe_shutdown(&mut self, _: Duration) -> Result<bool, ShutdownFailure> {
                Ok(false)
            }
            fn restore(&mut self) -> Result<(), String> {
                panic!("未完整退出不能重复启动")
            }
        }
        let root = std::env::temp_dir().join(format!(
            "ag-relay-update-{}",
            crate::update_journal::unique_id()
        ));
        let updates = UpdateService::with_journal(
            true,
            "0.2.0".into(),
            "隔离安装".into(),
            root.join("update.json"),
        );
        let server = ReleaseServer {
            downloads: AtomicUsize::new(0),
        };
        let id = tauri::async_runtime::block_on(updates.check(&server))
            .unwrap()
            .release
            .unwrap()
            .id;
        assert!(tauri::async_runtime::block_on(updates.confirm(
            &server,
            &Sandbox(Mutex::new(StillAlive)),
            &id
        ))
        .is_err());
        assert_eq!(updates.boot_plan(), BootPlan::NeedsAttention);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 重新检查清除旧候选_网络错误不是最新版且允许重试() {
        struct NoRelease(bool);
        impl UpdateProvider for NoRelease {
            type Candidate = ();
            async fn check(&self) -> Result<Option<Release<()>>, String> {
                if self.0 {
                    Err("无法检查更新".into())
                } else {
                    Ok(None)
                }
            }
            async fn download(
                &self,
                _: &(),
                _: impl Fn(DownloadEvent) + Send + Sync,
            ) -> Result<Vec<u8>, String> {
                panic!("检查不可下载")
            }
            fn install(&self, _: &(), _: &[u8]) -> Result<(), String> {
                panic!("检查不可安装")
            }
        }
        let updates = UpdateService::new(true, "0.2.0".into());
        let server = SuccessfulInstaller;
        tauri::async_runtime::block_on(updates.check(&server)).unwrap();
        let view = tauri::async_runtime::block_on(updates.check(&NoRelease(false))).unwrap();
        assert!(view.release.is_none());
        assert_eq!(view.phase, UpdatePhase::Idle);
        assert!(tauri::async_runtime::block_on(updates.check(&NoRelease(true))).is_err());
        assert_eq!(updates.view().phase, UpdatePhase::Failed);
        assert!(!updates.controls_blocked());
        assert!(tauri::async_runtime::block_on(updates.check(&server))
            .unwrap()
            .release
            .is_some());
    }

    #[test]
    fn 原来停止的更新不发送停机请求_不接管突然出现的外部实例() {
        struct Stopped;
        impl UpdateGateway for Stopped {
            fn identity(&mut self) -> Result<GatewayIdentity, String> {
                Ok(GatewayIdentity {
                    running: false,
                    token: "停止态".into(),
                })
            }
            fn safe_shutdown(&mut self, _: Duration) -> Result<bool, ShutdownFailure> {
                panic!("停止态不应扫描并停止外部实例")
            }
            fn restore(&mut self) -> Result<(), String> {
                panic!("停止态不应自启")
            }
        }
        let root = std::env::temp_dir().join(format!(
            "ag-relay-update-{}",
            crate::update_journal::unique_id()
        ));
        let updates = UpdateService::with_journal(
            true,
            "0.2.0".into(),
            "隔离安装".into(),
            root.join("update.json"),
        );
        let id = tauri::async_runtime::block_on(updates.check(&SuccessfulInstaller))
            .unwrap()
            .release
            .unwrap()
            .id;
        let view = tauri::async_runtime::block_on(updates.confirm(
            &SuccessfulInstaller,
            &Sandbox(Mutex::new(Stopped)),
            &id,
        ))
        .unwrap();
        assert_eq!(view.phase, UpdatePhase::InstallPending);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 用户明确选择启动后不再保留更新前的停止覆盖() {
        let root = std::env::temp_dir().join(format!(
            "ag-relay-update-{}",
            crate::update_journal::unique_id()
        ));
        let path = root.join("update.json");
        let updates =
            UpdateService::with_journal(true, "0.2.0".into(), "隔离安装".into(), path.clone());
        let host = Sandbox(Mutex::new(SandboxGateway { running: false }));
        let id = tauri::async_runtime::block_on(updates.check(&SuccessfulInstaller))
            .unwrap()
            .release
            .unwrap()
            .id;
        tauri::async_runtime::block_on(updates.confirm(&SuccessfulInstaller, &host, &id)).unwrap();
        let reopened =
            UpdateService::<()>::with_journal(true, "0.3.0".into(), "隔离安装".into(), path);
        assert_eq!(reopened.recover(&host), BootPlan::KeepStopped);
        reopened.acknowledge_manual_start().unwrap();
        assert_eq!(reopened.boot_plan(), BootPlan::Normal);
        assert!(reopened.view().recovery.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 未配置更新源不访问发布端点_不冒充最新版() {
        let server = ReleaseServer {
            downloads: AtomicUsize::new(0),
        };
        let updates = UpdateService::new(false, "0.2.0".into());
        let view = tauri::async_runtime::block_on(updates.check(&server)).unwrap();
        assert_eq!(view.phase, UpdatePhase::Disabled);
        assert_eq!(view.detail, "当前构建未配置更新源");
        assert!(view.release.is_none());
    }

    #[test]
    fn 检查只展示版本与说明_未确认不下载不安装() {
        let server = ReleaseServer {
            downloads: AtomicUsize::new(0),
        };
        let updates = UpdateService::new(true, "0.2.0".into());
        let view = tauri::async_runtime::block_on(updates.check(&server)).unwrap();
        assert_eq!(view.phase, UpdatePhase::Available);
        let release = view.release.unwrap();
        assert_eq!(release.version, "0.3.0");
        assert_eq!(release.notes, "安全更新说明");
        assert!(!release.id.is_empty());
        assert_eq!(server.downloads.load(Ordering::SeqCst), 0);
    }
}
