//! 网关连接设置：只保存单上游地址和本机端口，路径由调用方注入。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// 本机监听地址的单一真值来自 gateway_process：它必须与网关核心默认监听地址一致。
// 此处复用同一常量，避免 host()/endpoint() 各写字面量而与生命周期层漂移。
use crate::gateway_process::GATEWAY_HOST;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectionSettings {
    upstream: String,
    port: u16,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            upstream: "https://agentrouter.org".into(),
            port: 8787,
        }
    }
}

impl ConnectionSettings {
    pub fn new(upstream: &str, port: u32) -> Result<Self, String> {
        let upstream = upstream.trim();
        if upstream
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
            || !(upstream.starts_with("http://") || upstream.starts_with("https://"))
        {
            return Err("上游服务地址须为完整的 http:// 或 https:// 地址".into());
        }
        let url = tauri::Url::parse(upstream).map_err(|_| "上游服务地址无效")?;
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("上游服务地址不能包含账号、密钥、查询参数或片段".into());
        }
        let port = u16::try_from(port)
            .ok()
            .filter(|p| *p > 0)
            .ok_or("本机端口须为 1 到 65535 的整数")?;
        Ok(Self {
            upstream: url.as_str().trim_end_matches('/').into(),
            port,
        })
    }
    pub fn upstream(&self) -> &str {
        &self.upstream
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn host(&self) -> &'static str {
        GATEWAY_HOST
    }
    pub fn endpoint(&self) -> String {
        format!("http://{GATEWAY_HOST}:{}", self.port)
    }
}

pub struct ConnectionSettingsStore {
    path: PathBuf,
}

impl ConnectionSettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<ConnectionSettings, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct StoredSettings {
                    upstream: String,
                    port: u32,
                }
                let saved: StoredSettings = serde_json::from_slice(&bytes)
                    .map_err(|_| "连接设置文件无效，请重新保存设置")?;
                ConnectionSettings::new(&saved.upstream, saved.port)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConnectionSettings::default()),
            Err(_) => Err("无法读取连接设置文件".into()),
        }
    }

    pub fn save(&self, upstream: &str, port: u32) -> Result<ConnectionSettings, String> {
        let settings = ConnectionSettings::new(upstream, port)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| "无法创建连接设置目录")?;
        }
        let bytes = serde_json::to_vec_pretty(&settings).map_err(|_| "无法编码连接设置")?;
        // 同目录原子替换（与更新恢复凭据共享同一实现）：失败清理临时文件，原设置保持不变。
        crate::atomic_write::replace(&self.path, &bytes)
            .map_err(|_| String::from("无法保存连接设置，原设置保持不变"))?;
        Ok(settings)
    }
}
