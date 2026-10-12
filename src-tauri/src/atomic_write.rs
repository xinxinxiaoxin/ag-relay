//! 同目录原子替换的共享实现：更新恢复凭据与连接设置共用一套「临时文件 → fsync → rename」。
//! 两者此前的实现逐字重复，抽到此处单一真值，避免任一处单独漂移出半写文件。
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// 以原子替换方式写入 `path`：先在目标同目录以 `create_new` 建唯一临时文件，
/// 写入并 `sync_all` 后 `rename` 覆盖目标（Windows std 的 rename 即 replace-existing，
/// 同目录同一卷因此是原子替换）。任一步失败都清理临时文件并返回错误，绝不留下半写文件替换目标。
///
/// 调用方负责：先创建父目录、把内容序列化成 `bytes`，并把返回的错误映射成各自的文案；
/// 本函数不改动这些既有错误语义。
pub fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("atomic");
    let temp = parent.join(format!(
        ".{stem}.{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
