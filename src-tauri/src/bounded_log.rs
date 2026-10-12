//! 自有文件日志：当前文件与一份旧档共用容量预算，不在退出时清空。
//! 进程内多线程写入串行化；不同面板进程通过稳定的空 `.lock` 文件互斥。
//! 清理只涉及显式路径及其 `.1`：到限淘汰最旧档；期限按文件 mtime 惰性检查。
//! 旧版超大文件仅读取有界尾部，保留完整行；新记录超过单档预算则拒绝，不截断。
//! I/O 错误交给调用方按尽力而为处理，不尝试无界追加作为降级。

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Clone, Copy)]
pub struct LogPolicy {
    pub max_total_bytes: u64,
    pub retention: Option<std::time::Duration>,
}

/// 追加完整记录；路径由调用方注入，不访问默认应用目录。
pub fn append_record(path: &Path, record: &str, policy: LogPolicy) -> io::Result<()> {
    // 退出事件与停止事件可能来自不同线程；整个清理/轮转/追加串行化。
    static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITER.lock().unwrap_or_else(|error| error.into_inner());
    let limit = policy.max_total_bytes / 2;
    let bytes = record.len() as u64 + 1;
    if limit == 0 || bytes > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "日志记录超过单档预算",
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    // 稳定的空锁文件不随轮转替换；锁忙时跳过此次记录，不能拖住退出流程。
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock()?;
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".1");
    for file in [path, Path::new(&backup)] {
        let metadata = match fs::metadata(file) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if let Some(retention) = policy.retention {
            if metadata
                .modified()?
                .elapsed()
                .is_ok_and(|age| age > retention)
            {
                fs::remove_file(file)?;
                continue;
            }
        }
        if metadata.len() > limit {
            // 旧版日志可能无界；只读末尾预算 + 1 字节，按换行截掉最旧的半行。
            let mut file = OpenOptions::new().read(true).write(true).open(file)?;
            file.seek(SeekFrom::Start(metadata.len() - limit - 1))?;
            let mut tail = Vec::new();
            (&mut file).take(limit + 1).read_to_end(&mut tail)?;
            let start = tail
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(tail.len(), |i| i + 1);
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&tail[start..])?;
            file.set_len((tail.len() - start) as u64)?;
        }
    }
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    if size + bytes > limit {
        match fs::remove_file(&backup) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        fs::rename(path, backup)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{record}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "ag-relay-bounded-log-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // 由 Rust 应用层运行纯文件写入模块；不 import 主网关、不监听端口。
    fn run_file_logger(script: &str) {
        let dir = TempDir::new();
        let module =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/gateway/bounded-file-log.mjs");
        let script = format!(
            r#"
            import * as fs from 'node:fs';
            import assert from 'node:assert/strict';
            import {{ join }} from 'node:path';
            import {{ pathToFileURL }} from 'node:url';
            const {{ createFileLogger }} = await import(pathToFileURL(process.argv[1]));
            const root = process.argv[2];
            const path = join(root, 'ordinary.log');
            {script}
        "#
        );
        let node =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries/node-x86_64-pc-windows-msvc.exe");
        let output = std::process::Command::new(node)
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .current_dir(&dir.0)
            .args(["--input-type=module", "-e", &script])
            .arg(module)
            .arg(&dir.0)
            .output()
            .expect("需要 Node 运行纯文件日志测试");
        assert!(
            output.status.success(),
            "普通文件日志测试失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn 普通日志_默认关闭_超长记录不写入_非法预算回退有界默认() {
        run_file_logger(
            r#"
            createFileLogger({})('默认不落盘');
            assert.deepEqual(fs.readdirSync(root), []);
            const write = createFileLogger({ PROXY_LOG_FILE: path, PROXY_LOG_MAX_BYTES: '16' });
            write('中文'); // 6 字节 + 换行
            write('超出预算的中文');
            assert.equal(fs.readFileSync(path, 'utf8'), '中文\n');
            assert.ok(!fs.existsSync(path + '.1'));
            for (const value of [undefined, '0', '-1', 'NaN', 'Infinity', '1', '2.5']) {
              const file = path + '-' + value;
              const fallback = createFileLogger({ PROXY_LOG_FILE: file, PROXY_LOG_MAX_BYTES: value });
              for (let i = 0; i < 5; i++) fallback('x'.repeat(512 * 1024 - 1));
              assert.equal(fs.statSync(file + '.1').size, 1024 * 1024);
              assert.ok(fs.statSync(file).size + fs.statSync(file + '.1').size <= 2 * 1024 * 1024);
            }
        "#,
        );
    }

    #[test]
    fn 自有日志_超长记录和非法预算不破坏既有文件或诊断记录() {
        let dir = TempDir::new();
        let path = dir.0.join("shell-exit.log");
        let diagnostics = dir.0.join("relay-diagnostics.jsonl");
        fs::write(&diagnostics, "诊断记录不归此模块管理\n").unwrap();
        let policy = LogPolicy {
            max_total_bytes: 16,
            retention: None,
        };
        append_record(&path, "中文", policy).unwrap();
        assert_eq!(
            append_record(&path, "这条太长", policy).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            append_record(
                &path,
                "",
                LogPolicy {
                    max_total_bytes: 0,
                    retention: None
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "中文\n");
        assert_eq!(
            fs::read_to_string(&diagnostics).unwrap(),
            "诊断记录不归此模块管理\n"
        );
    }

    #[test]
    fn 普通日志_写入失败不阻断请求且恢复后继续记录() {
        run_file_logger(
            r#"
            fs.mkdirSync(path);
            const write = createFileLogger({ PROXY_LOG_FILE: path });
            assert.doesNotThrow(() => write('文件不可写'));
            fs.rmdirSync(path);
            write('已恢复');
            assert.equal(fs.readFileSync(path, 'utf8'), '已恢复\n');
        "#,
        );
    }

    #[test]
    fn 普通日志_旧版超大文件收敛并保留最新中文整行() {
        run_file_logger(
            r#"
            fs.writeFileSync(path, '最早记录\n旧\n新\n');
            fs.writeFileSync(path + '.1', '更早更早更早\n');
            createFileLogger({ PROXY_LOG_FILE: path, PROXY_LOG_MAX_BYTES: '16' })('末');
            assert.equal(fs.readFileSync(path, 'utf8'), '末\n');
            assert.equal(fs.readFileSync(path + '.1', 'utf8'), '旧\n新\n');
            assert.ok(fs.readdirSync(root).reduce((sum, f) => sum + fs.statSync(join(root, f)).size, 0) <= 16);
        "#,
        );
    }

    #[test]
    fn 普通日志_期限可选且重新创建写入器不清空历史() {
        run_file_logger(
            r#"
            fs.writeFileSync(path, '上次启动\n');
            fs.writeFileSync(path + '.1', '早期记录\n');
            fs.utimesSync(path + '.1', new Date(0), new Date(0));
            createFileLogger({ PROXY_LOG_FILE: path })('新启动');
            assert.equal(fs.readFileSync(path + '.1', 'utf8'), '早期记录\n');
            const write = createFileLogger({ PROXY_LOG_FILE: path, PROXY_LOG_RETENTION_DAYS: '1' });
            write('最新');
            assert.ok(!fs.existsSync(path + '.1'));
            assert.equal(fs.readFileSync(path, 'utf8'), '上次启动\n新启动\n最新\n');
            fs.utimesSync(path, new Date(0), new Date(0));
            write('过期后新记录');
            assert.equal(fs.readFileSync(path, 'utf8'), '过期后新记录\n');
        "#,
        );
    }

    #[test]
    fn 普通日志_显式启用后到限淘汰最旧档() {
        run_file_logger(
            r#"
            const write = createFileLogger({ PROXY_LOG_FILE: path, PROXY_LOG_MAX_BYTES: '16' });
            for (const line of ['111', '222', '333', '444', '555']) write(line);
            assert.equal(fs.readFileSync(path, 'utf8'), '555\n');
            assert.equal(fs.readFileSync(path + '.1', 'utf8'), '333\n444\n');
            assert.ok(fs.readdirSync(root).reduce((sum, f) => sum + fs.statSync(join(root, f)).size, 0) <= 16);
        "#,
        );
    }

    #[test]
    fn 自有日志_其他进程持锁时不改文件也不阻塞退出() {
        let dir = TempDir::new();
        let path = dir.0.join("shell-exit.log");
        fs::write(&path, "旧\n").unwrap();
        let lock = fs::File::create(dir.0.join("shell-exit.log.lock")).unwrap();
        lock.lock().unwrap();
        let policy = LogPolicy {
            max_total_bytes: 16,
            retention: None,
        };
        assert!(append_record(&path, "新", policy).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "旧\n");
        drop(lock);
        append_record(&path, "新", policy).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "旧\n新\n");
    }

    #[test]
    fn 自有日志_并发退出记录仍遵守总预算() {
        let dir = TempDir::new();
        let path = dir.0.join("shell-exit.log");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let writers: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..40 {
                        append_record(
                            &path,
                            "一条记录",
                            LogPolicy {
                                max_total_bytes: 256,
                                retention: None,
                            },
                        )
                        .unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let total: u64 = fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum();
        assert!(total <= 256);
        for entry in fs::read_dir(&dir.0).unwrap() {
            let text = fs::read_to_string(entry.unwrap().path()).unwrap();
            assert!(text.lines().all(|line| line == "一条记录"));
        }
    }

    #[test]
    fn 自有日志_旧版超限文件收敛预算且不切断中文记录() {
        let dir = TempDir::new();
        let path = dir.0.join("shell-exit.log");
        let backup = dir.0.join("shell-exit.log.1");
        fs::write(&path, "最早记录\n旧\n新\n").unwrap();
        fs::write(&backup, "更早更早更早\n").unwrap();
        append_record(
            &path,
            "末",
            LogPolicy {
                max_total_bytes: 16,
                retention: None,
            },
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "末\n");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "旧\n新\n");
        let total: u64 = fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum();
        assert!(total <= 16, "升级遗留的超大文件也须收敛");
    }

    #[test]
    fn 自有日志_启用期限只清理过期档_默认保留上次记录() {
        let dir = TempDir::new();
        let path = dir.0.join("shell-exit.log");
        let backup = dir.0.join("shell-exit.log.1");
        fs::write(&path, "上次退出\n").unwrap();
        fs::write(&backup, "更早退出\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&backup)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        let mut policy = LogPolicy {
            max_total_bytes: 1024,
            retention: None,
        };
        append_record(&path, "这次退出", policy).unwrap();
        assert_eq!(fs::read_to_string(&backup).unwrap(), "更早退出\n");
        policy.retention = Some(std::time::Duration::from_secs(86400));
        append_record(&path, "最新退出", policy).unwrap();
        assert!(!backup.exists(), "期限启用后应淘汰过期旧档");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "上次退出\n这次退出\n最新退出\n"
        );
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        append_record(&path, "过期后新记录", policy).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "过期后新记录\n");
    }

    #[test]
    fn 自有日志_到限淘汰最旧档并保留完整新记录() {
        let dir = TempDir::new();
        let path = dir.0.join("shell-exit.log");
        let policy = LogPolicy {
            max_total_bytes: 16,
            retention: None,
        };
        // 每行 4 字节，每份最多 8 字节，5 行时应淘汰最早两行。
        for record in ["111", "222", "333", "444", "555"] {
            append_record(&path, record, policy).unwrap();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "555\n");
        assert_eq!(
            fs::read_to_string(dir.0.join("shell-exit.log.1")).unwrap(),
            "333\n444\n"
        );
        let total: u64 = fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum();
        assert!(total <= 16);
    }
}
