//! 更新恢复凭据独立于连接设置和轮转日志。仅原子替换本模块自己的单个文件。
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootPlan {
    Normal,
    RestoreRunning,
    KeepStopped,
    NeedsAttention,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Prepared,
    InstallPending,
    Aborted,
    Restored,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Transaction {
    schema: u32,
    pub id: String,
    from: String,
    to: String,
    installation: String,
    pub was_running: bool,
    pub stage: Stage,
}

pub(crate) struct Journal {
    path: PathBuf,
    version: String,
    installation: String,
}

impl Journal {
    pub fn new(path: PathBuf, version: String, installation: String) -> Self {
        Self {
            path,
            version,
            installation,
        }
    }

    pub fn transaction(&self, to: String, was_running: bool) -> Transaction {
        Transaction {
            schema: 1,
            id: unique_id(),
            from: self.version.clone(),
            to,
            installation: self.installation.clone(),
            was_running,
            stage: Stage::Prepared,
        }
    }

    pub fn read(&self) -> Result<Option<Transaction>, String> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| "更新恢复记录损坏，已禁止自动启动网关。".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("无法读取更新恢复记录，已禁止自动启动网关。".into()),
        }
    }

    pub fn clear(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("无法清除更新恢复覆盖，未启动网关。".into()),
        }
    }

    pub fn write(&self, record: &Transaction) -> Result<(), String> {
        let parent = self.path.parent().ok_or("更新恢复目录无效。")?;
        fs::create_dir_all(parent).map_err(|_| "无法创建更新恢复目录。")?;
        let bytes = serde_json::to_vec(record)
            .map_err(|_| "无法持久化更新恢复记录；未获准安装。")?;
        // 同目录原子替换（临时文件 → fsync → rename 覆盖），失败清理临时文件。
        crate::atomic_write::replace(&self.path, &bytes)
            .map_err(|_| "无法持久化更新恢复记录；未获准安装。".into())
    }

    pub fn boot_plan(&self) -> BootPlan {
        match self.read() {
            Ok(None) => BootPlan::Normal,
            Ok(Some(r))
                if r.schema == 1
                    && r.installation == self.installation
                    && r.to == self.version
                    && r.stage == Stage::InstallPending =>
            {
                if r.was_running {
                    BootPlan::RestoreRunning
                } else {
                    BootPlan::KeepStopped
                }
            }
            Ok(Some(r))
                if r.schema == 1
                    && r.installation == self.installation
                    && r.to == self.version
                    && r.stage == Stage::Restored =>
            {
                if r.was_running {
                    BootPlan::Normal
                } else {
                    BootPlan::KeepStopped
                }
            }
            Ok(Some(r))
                if r.schema == 1
                    && r.installation == self.installation
                    && r.from == self.version
                    && r.stage == Stage::Aborted =>
            {
                if r.was_running {
                    BootPlan::Normal
                } else {
                    BootPlan::KeepStopped
                }
            }
            _ => BootPlan::NeedsAttention,
        }
    }
}

pub(crate) fn unique_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:x}-{:x}", NEXT.fetch_add(1, Ordering::Relaxed))
}
