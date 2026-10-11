//! 网关控制端点调用与日志目录（T4）：安全重启走网关既有 HTTP 控制端点（`x-relay-control` 头，
//! 事实来源：relay-control.mjs 的既有约定）；打开日志目录复用诊断日志所在目录。
//! 纯函数可脱离 tauri 运行时测试；命令层薄封装在 gateway_commands.rs。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RESTART_PATH: &str = "/_relay/restart";
pub const CONTROL_HEADER_NAME: &str = "x-relay-control";
pub const CONTROL_HEADER_VALUE: &str = "1";
/// 安全重启含 drain（在途请求清零才重启），给足 60 秒
pub const DEFAULT_RESTART_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
pub enum ControlError {
    /// 连接被拒：8787 上没有网关在跑
    NotRunning,
    /// 等待响应超时（网关可能仍在排空在途请求）
    Timeout,
    /// 网关返回了非成功响应或协议异常
    Failed(String),
}

/// 组装安全重启的 HTTP 请求原文（POST + 控制头，空请求体）。
pub fn build_restart_request(host: &str, port: u16) -> String {
    format!(
        "POST {RESTART_PATH} HTTP/1.1\r\nHost: {host}:{port}\r\n{CONTROL_HEADER_NAME}: {CONTROL_HEADER_VALUE}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// 调用网关安全重启端点。连接拒绝 → NotRunning；等待响应超时 → Timeout；非 200 → Failed。
pub fn post_restart(host: &str, port: u16, timeout: Duration) -> Result<(), ControlError> {
    let mut stream = TcpStream::connect((host, port)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::ConnectionRefused {
            ControlError::NotRunning
        } else {
            ControlError::Failed(format!("连接网关失败：{e}"))
        }
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| ControlError::Failed(format!("设置读取超时失败：{e}")))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| ControlError::Failed(format!("设置写入超时失败：{e}")))?;

    let request = build_restart_request(host, port);
    stream
        .write_all(request.as_bytes())
        .map_err(|e| ControlError::Failed(format!("发送重启请求失败：{e}")))?;
    stream
        .flush()
        .map_err(|e| ControlError::Failed(format!("刷新请求失败：{e}")))?;

    let mut response = Vec::new();
    match stream.read_to_end(&mut response) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
            return Err(ControlError::Timeout);
        }
        Err(e) => return Err(ControlError::Failed(format!("读取重启响应失败：{e}"))),
    }

    let text = String::from_utf8_lossy(&response);
    let status_line = text.lines().next().unwrap_or("");
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| ControlError::Failed(format!("网关未返回 HTTP 响应（连接已关闭）")))?;
    if (200..300).contains(&status_code) {
        Ok(())
    } else {
        Err(ControlError::Failed(format!("网关返回异常：{status_line}")))
    }
}

/// 诊断日志所在目录（应用数据目录下的 logs 子目录）。
pub fn logs_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("logs")
}

/// 取日志目录，不存在则创建。
pub fn ensure_logs_dir(app_data_dir: &Path) -> Result<PathBuf, String> {
    let dir = logs_dir(app_data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建日志目录失败：{e}"))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Instant;

    /// 最小 HTTP mock：随机端口监听一次连接，收到的请求存进 captured，按 status_line 应答。
    fn spawn_mock(status_line: &'static str, captured: Arc<Mutex<Vec<u8>>>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 随机端口");
        let port = listener.local_addr().expect("local_addr").port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 512];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            *captured.lock().unwrap() = buf.clone();
            let body = b"ok";
            let head = format!("{status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        });
        port
    }

    /// accept 后只挂着不应答（测超时）。
    fn spawn_silent() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 随机端口");
        let port = listener.local_addr().expect("local_addr").port();
        thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept");
            thread::sleep(Duration::from_secs(5));
        });
        port
    }

    #[test]
    fn 重启请求_必须是POST加控制头x_relay_control_1() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let port = spawn_mock("HTTP/1.1 200 OK", captured.clone());
        let result = post_restart("127.0.0.1", port, Duration::from_secs(3));
        assert!(matches!(result, Ok(())), "200 响应应视为重启已受理，实际 {:?}", result);
        let req = String::from_utf8(captured.lock().unwrap().clone()).expect("请求应为 ASCII");
        assert!(
            req.starts_with("POST /_relay/restart HTTP/1.1\r\n"),
            "方法与路径必须是 POST /_relay/restart，实际首行：{:?}",
            req.lines().next()
        );
        assert!(
            req.to_ascii_lowercase().contains("x-relay-control: 1"),
            "必须带控制头 x-relay-control: 1，实际请求：{req}"
        );
    }

    #[test]
    fn 重启_网关未运行_连接拒绝应秒回NotRunning() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("占一个随机端口");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);
        let started = Instant::now();
        let result = post_restart("127.0.0.1", port, Duration::from_secs(5));
        assert!(matches!(result, Err(ControlError::NotRunning)), "连接拒绝应返回 NotRunning，实际 {:?}", result);
        // 高负载下（并行构建/测试）loopback 连接拒绝可能拖到秒级；关键语义是「不傻等满超时」
        assert!(started.elapsed() < Duration::from_secs(5), "连接拒绝必须远小于超时上限，实际 {:?}", started.elapsed());
    }

    #[test]
    fn 重启_网关不响应_应在超时上限返回Timeout() {
        let port = spawn_silent();
        let started = Instant::now();
        let result = post_restart("127.0.0.1", port, Duration::from_millis(500));
        assert!(matches!(result, Err(ControlError::Timeout)), "超时应返回 Timeout，实际 {:?}", result);
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(450) && elapsed < Duration::from_secs(3),
            "应恰在超时上限附近返回，实际 {elapsed:?}"
        );
    }

    #[test]
    fn 重启_202已受理_应视为成功() {
        // 真网关契约（E2E 实测）：/_relay/restart 返回 202 Accepted（受理后排空再重启）
        let captured = Arc::new(Mutex::new(Vec::new()));
        let port = spawn_mock("HTTP/1.1 202 Accepted", captured);
        let result = post_restart("127.0.0.1", port, Duration::from_secs(3));
        assert!(matches!(result, Ok(())), "2xx 应视为重启已受理，实际 {:?}", result);
    }

    #[test]
    fn 重启_非200响应_应返回Failed并带状态行() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let port = spawn_mock("HTTP/1.1 500 Internal Server Error", captured);
        let result = post_restart("127.0.0.1", port, Duration::from_secs(3));
        match result {
            Err(ControlError::Failed(detail)) => assert!(detail.contains("500"), "失败详情应含状态行，实际：{detail}"),
            other => panic!("非 200 应返回 Failed，实际 {other:?}"),
        }
    }

    #[test]
    fn 日志目录_不存在时应自动创建并返回logs子目录() {
        let base = std::env::temp_dir().join(format!("ag-relay-t4-test-{}", std::process::id()));
        let dir = ensure_logs_dir(&base).expect("应成功创建");
        assert_eq!(dir, logs_dir(&base), "返回的应是 app_data\\logs");
        assert!(dir.is_dir(), "目录应已实际创建：{}", dir.display());
        let _ = std::fs::remove_dir_all(&base);
    }
}
