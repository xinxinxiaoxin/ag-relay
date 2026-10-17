//! 单上游连接的可测应用模块。保存与运行配置分离，显式重启才应用；路径全部注入。
use crate::{
    connection_settings::{ConnectionSettings, ConnectionSettingsStore},
    gateway_process::{self, GatewayPaths, ManagedChild},
    gateway_safe_shutdown::safe_shutdown_at,
    policy_settings::{ActivePolicy, PolicySettings, PolicySettingsStore, PolicySettingsView},
};
use serde::Serialize;
use std::{
    fmt,
    time::{Duration, Instant},
};

#[derive(Debug, PartialEq, Eq)]
pub enum ConnectionError {
    Timeout,
    Failed(String),
}
impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "排空超时，已恢复接单，未强杀；请稍后重试"),
            Self::Failed(detail) => write!(f, "{detail}"),
        }
    }
}
impl std::error::Error for ConnectionError {}
impl From<String> for ConnectionError {
    fn from(s: String) -> Self {
        Self::Failed(s)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSettingsView {
    pub saved: ConnectionSettings,
    pub active: Option<ConnectionSettings>,
    pub restart_required: bool,
    pub endpoint: String,
}

pub struct GatewayConnection {
    paths: GatewayPaths,
    store: ConnectionSettingsStore,
    policy_store: Option<PolicySettingsStore>,
    active: Option<ConnectionSettings>,
    managed: Option<ManagedChild>,
}

impl GatewayConnection {
    pub fn new(paths: GatewayPaths, store: ConnectionSettingsStore) -> Self {
        let active = store.load().ok();
        Self {
            paths,
            store,
            policy_store: None,
            active,
            managed: None,
        }
    }
    /// 生产路径显式注入两份独立设置；旧构造入口不注入策略 JSON，保留旧环境配置语义。
    pub fn new_with_policy_store(
        paths: GatewayPaths,
        store: ConnectionSettingsStore,
        policy_store: PolicySettingsStore,
    ) -> Self {
        let mut connection = Self::new(paths, store);
        connection.policy_store = Some(policy_store);
        connection
    }

    fn load_policy(&self) -> Result<Option<PolicySettings>, ConnectionError> {
        self.policy_store
            .as_ref()
            .map(PolicySettingsStore::load)
            .transpose()
            .map_err(ConnectionError::from)
    }

    pub fn policy_settings(&self) -> Result<PolicySettingsView, ConnectionError> {
        let saved = self.load_policy()?.unwrap_or_default();
        let active = self.read_active_policy();
        Ok(PolicySettingsView {
            defaults: PolicySettings::default(),
            restart_required: active
                .as_ref()
                .is_some_and(|active| active.settings != saved),
            active_supported: active.is_some(),
            active_revision: active.as_ref().map(|active| active.revision.clone()),
            active: active.map(|active| active.settings),
            saved,
        })
    }

    pub fn save_policy(
        &mut self,
        settings: &PolicySettings,
    ) -> Result<PolicySettingsView, ConnectionError> {
        self.policy_store
            .as_ref()
            .ok_or_else(|| ConnectionError::Failed("未配置策略设置文件路径".into()))?
            .save(settings)?;
        self.policy_settings()
    }

    fn read_active_policy(&self) -> Option<ActivePolicy> {
        let port = self.port().ok()?;
        let body =
            crate::gateway_status::fetch_status_body(gateway_process::GATEWAY_HOST, port).ok()?;
        let pid = crate::gateway_status::parse_status_pid(&body)?;
        let gateway_process::ExternalStopTarget::Supervisor {
            pid: supervisor_pid,
        } = gateway_process::resolve_external_stop_target(pid)
        else {
            return None;
        };
        // 外部实例也须核实属于本网关；持有句柄时还须与当前 supervisor 一致。
        if self
            .managed_supervisor_pid()
            .is_some_and(|expected| expected != supervisor_pid)
        {
            return None;
        }
        ActivePolicy::from_status(&body)
    }

    pub fn settings(&self) -> Result<ConnectionSettingsView, ConnectionError> {
        let saved = self.store.load()?;
        Ok(ConnectionSettingsView {
            restart_required: self.active.as_ref().is_some_and(|active| *active != saved),
            endpoint: self.active.as_ref().unwrap_or(&saved).endpoint(),
            saved,
            active: self.active.clone(),
        })
    }
    pub fn save(
        &mut self,
        upstream: &str,
        port: u32,
    ) -> Result<ConnectionSettingsView, ConnectionError> {
        self.store.save(upstream, port)?;
        self.settings()
    }
    pub fn port(&self) -> Result<u16, ConnectionError> {
        Ok(match &self.active {
            Some(active) => active.port(),
            None => self.store.load()?.port(),
        })
    }
    pub fn holds_managed(&self) -> bool {
        self.managed.is_some()
    }
    pub fn managed_supervisor_pid(&self) -> Option<u32> {
        self.managed.as_ref().map(|gw| gw.supervisor_pid)
    }

    pub fn start(&mut self) -> Result<u32, ConnectionError> {
        let saved = self.store.load()?;
        let policy = self.load_policy()?;
        self.start_validated(saved, policy)
    }

    // restart 传入停机前已经读取/校验的快照，不在停掉旧网关后重新读盘。
    fn start_validated(
        &mut self,
        saved: ConnectionSettings,
        policy: Option<PolicySettings>,
    ) -> Result<u32, ConnectionError> {
        if let Some(policy) = &policy {
            policy.validate()?;
        }
        if let Some(child) = self.managed.as_mut() {
            if child
                .child
                .try_wait()
                .map_err(|_| ConnectionError::Failed("无法读取受管网关状态".into()))?
                .is_none()
            {
                return Err(ConnectionError::Failed(
                    "网关已经运行，请使用安全重启应用设置".into(),
                ));
            }
            self.managed = None;
        }
        let child = match policy.as_ref() {
            Some(policy) => {
                gateway_process::start_gateway_process_with_policy(&self.paths, &saved, policy)
            }
            None => gateway_process::start_gateway_process_with_settings(&self.paths, &saved),
        }
        .map_err(|e| {
            ConnectionError::Failed(match e {
                gateway_process::GatewayError::PortAlreadyInUse { .. } => {
                    "port_already_in_use".into()
                }
                _ => e.to_string(),
            })
        })?;
        let pid = child.supervisor_pid;
        self.managed = Some(child);
        self.active = Some(saved.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(body) = crate::gateway_status::fetch_status_body(saved.host(), saved.port()) {
                if crate::gateway_status::parse_status_pid(&body)
                    .map(gateway_process::resolve_external_stop_target)
                    == Some(gateway_process::ExternalStopTarget::Supervisor { pid })
                {
                    return Ok(pid);
                }
            }
            if self
                .managed
                .as_mut()
                .unwrap()
                .child
                .try_wait()
                .ok()
                .flatten()
                .is_some()
            {
                self.managed = None;
                return Err(ConnectionError::Failed("网关启动失败，进程已退出".into()));
            }
            if Instant::now() >= deadline {
                // 即使就绪确认失败，也保留句柄；绝不因 UI 等待超时而掐断可能已受理的请求。
                return Err(ConnectionError::Failed(
                    "网关就绪确认超时，已保留进程，请稍后核实".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// 唯一供配置切换与更新调用的安全停机入口。错误时不丢句柄、不发送信号、不强杀。
    pub fn safe_shutdown(&mut self, timeout: Duration) -> Result<bool, ConnectionError> {
        let port = self.port()?;
        if let Some(child) = self.managed.as_mut() {
            if child.child.try_wait().ok().flatten().is_some() {
                if gateway_process::port_in_use(gateway_process::GATEWAY_HOST, port) {
                    return Err(ConnectionError::Failed(
                        "受管网关已退出，但端口出现其它实例；未执行停机".into(),
                    ));
                }
                self.managed = None;
                return Ok(false);
            }
        }
        let was_running = safe_shutdown_at(port, self.managed_supervisor_pid(), timeout)?;
        self.managed = None;
        Ok(was_running)
    }

    pub fn restart(&mut self, timeout: Duration) -> Result<bool, ConnectionError> {
        let saved = self.store.load()?;
        let policy = self.load_policy()?;
        if let Some(policy) = &policy {
            policy.validate()?;
        }
        // 已知启动条件先检查，不为明显不可用的新配置停掉旧网关。
        if !self.paths.node_exe.is_file()
            || !self
                .paths
                .gateway_dir
                .join("relay-supervisor.mjs")
                .is_file()
        {
            return Err(ConnectionError::Failed(
                "网关运行文件缺失，未停止当前实例".into(),
            ));
        }
        if saved.port() != self.port()? && gateway_process::port_in_use(saved.host(), saved.port())
        {
            return Err(ConnectionError::Failed(
                "新端口已被占用，未停止当前网关".into(),
            ));
        }
        if !self.safe_shutdown(timeout)? {
            return Ok(false);
        }
        self.start_validated(saved, policy)?;
        Ok(true)
    }
}
