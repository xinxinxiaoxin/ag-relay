use ag_relay_lib::{
    connection_settings::ConnectionSettingsStore, gateway_connection::GatewayConnection,
    gateway_process::GatewayPaths,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    time::Duration,
};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn request(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(stream, "GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer fixture-only-not-a-key\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

struct Fixture {
    dir: PathBuf,
    pub gateway: GatewayConnection,
}
impl Fixture {
    fn new(name: &str, upstream: &str, port: u16) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ag-relay-connection-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("gateway")).unwrap();
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // 完整资源闭包，包含后继日志模块；核心绝不从真实安装目录运行。
        for entry in std::fs::read_dir(manifest.join("resources/gateway")).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                std::fs::copy(entry.path(), dir.join("gateway").join(entry.file_name())).unwrap();
            }
        }
        let store = ConnectionSettingsStore::new(dir.join("connection.json"));
        store.save(upstream, port.into()).unwrap();
        let paths = GatewayPaths {
            node_exe: manifest.join("binaries/node-x86_64-pc-windows-msvc.exe"),
            gateway_dir: dir.join("gateway"),
            diagnostics_file: dir.join("logs/diagnostics.jsonl"),
        };
        Self {
            dir,
            gateway: GatewayConnection::new(paths, store),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // 正常完成先走安全停机；断言 panic 时只回收自己持有的 Job，不按端口杀进程。
        let _ = self.gateway.safe_shutdown(Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn 安全排空_请求自然完成后才停止整个网关() {
    use std::sync::mpsc;
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let (seen_tx, seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = upstream.accept().unwrap();
        let mut bytes = [0; 8192];
        stream.read(&mut bytes).unwrap();
        seen_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}").unwrap();
    });
    let port = free_port();
    let mut fixture = Fixture::new("drain-completion", &format!("http://127.0.0.1:{upstream_port}"), port);
    fixture.gateway.start().unwrap();
    let client = std::thread::spawn(move || request(port, "/hold"));
    seen_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let release = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let body = ag_relay_lib::gateway_status::fetch_status_body("127.0.0.1", port).unwrap();
            let status: serde_json::Value = serde_json::from_str(&body).unwrap();
            if status["draining"] == true { break; }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        release_tx.send(()).unwrap();
    });
    assert!(fixture.gateway.safe_shutdown(Duration::from_secs(5)).unwrap());
    assert!(client.join().unwrap().contains("{\"ok\":true}"));
    release.join().unwrap();
    server.join().unwrap();
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn 安全停机_响应中断不能当作退出成功() {
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", upstream.local_addr().unwrap().port());
    let port = free_port();
    let mut fixture = Fixture::new("broken-response", &url, port);
    fixture.gateway.start().unwrap();
    let body = ag_relay_lib::gateway_status::fetch_status_body("127.0.0.1", port).unwrap();
    let broken = TcpListener::bind("127.0.0.1:0").unwrap();
    let broken_port = broken.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (mut stream, _) = broken.accept().unwrap();
            let mut bytes = [0; 8192];
            stream.read(&mut bytes).unwrap();
            if index == 0 {
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            } else {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 200\r\nConnection: close\r\n\r\n{\"drained\":").unwrap();
            }
        }
    });
    assert!(ag_relay_lib::gateway_safe_shutdown::safe_shutdown_at(broken_port, fixture.gateway.managed_supervisor_pid(), Duration::from_millis(200)).is_err());
    server.join().unwrap();
    assert!(fixture.gateway.holds_managed());
    assert!(ag_relay_lib::gateway_status::fetch_status_body("127.0.0.1", port).is_ok());
    assert!(fixture.gateway.safe_shutdown(Duration::from_secs(3)).unwrap());
}

#[test]
fn 安全停机_外部实例也等待完整父链退出_新端口冲突不停止旧实例() {
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", upstream.local_addr().unwrap().port());
    let port = free_port();
    let mut owner = Fixture::new("external-owner", &url, port);
    owner.gateway.start().unwrap();
    let original = owner.gateway.managed_supervisor_pid();
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    owner
        .gateway
        .save(&url, occupied.local_addr().unwrap().port().into())
        .unwrap();
    assert!(owner.gateway.restart(Duration::from_millis(200)).is_err());
    assert_eq!(owner.gateway.managed_supervisor_pid(), original);
    assert!(ag_relay_lib::gateway_status::fetch_status_body("127.0.0.1", port).is_ok());
    let mut external = Fixture::new("external-observer", &url, port);
    assert!(!external.gateway.holds_managed());
    assert!(external
        .gateway
        .safe_shutdown(Duration::from_secs(3))
        .unwrap());
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
    assert!(!owner.gateway.safe_shutdown(Duration::from_secs(3)).unwrap());
}

#[test]
fn 安全排空_控制请求断连撤销排空_在途响应保持完整() {
    use std::sync::mpsc;
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let (seen_tx, seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = upstream.accept().unwrap();
        let mut bytes = [0; 8192];
        stream.read(&mut bytes).unwrap();
        seen_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}").unwrap();
    });
    let port = free_port();
    let mut fixture = Fixture::new(
        "disconnect",
        &format!("http://127.0.0.1:{upstream_port}"),
        port,
    );
    fixture.gateway.start().unwrap();
    let client = std::thread::spawn(move || request(port, "/hold"));
    seen_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let status = || -> serde_json::Value {
        serde_json::from_str(
            &ag_relay_lib::gateway_status::fetch_status_body("127.0.0.1", port).unwrap(),
        )
        .unwrap()
    };
    let pid = status()["pid"].as_u64().unwrap();
    let mut control = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(control, "POST /_relay/shutdown HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nx-relay-control: 1\r\nx-relay-pid: {pid}\r\nx-relay-timeout-ms: 5000\r\nContent-Length: 0\r\n\r\n").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while status()["draining"] != true {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        request(port, "/new-request").contains("503"),
        "排空期间不得接入新请求"
    );
    drop(control);
    while status()["draining"] != false {
        assert!(
            std::time::Instant::now() < deadline,
            "调用者断连后应撤销排空"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    release_tx.send(()).unwrap();
    assert!(client.join().unwrap().contains("{\"ok\":true}"));
    server.join().unwrap();
    assert_eq!(status()["pid"], pid);
    assert!(fixture
        .gateway
        .safe_shutdown(Duration::from_secs(3))
        .unwrap());
}

#[test]
fn 安全排空_在途请求超时不强杀_恢复接单且不会稍后偷偷退出() {
    use std::sync::mpsc;
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let (seen_tx, seen_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (mut stream, _) = upstream.accept().unwrap();
            let mut bytes = [0; 8192];
            stream.read(&mut bytes).unwrap();
            if index == 0 {
                seen_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(15)).unwrap();
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}").unwrap();
        }
    });
    let port = free_port();
    let mut fixture = Fixture::new(
        "timeout",
        &format!("http://127.0.0.1:{upstream_port}"),
        port,
    );
    fixture.gateway.start().unwrap();
    let original_pid = fixture.gateway.managed_supervisor_pid();
    let client = std::thread::spawn(move || request(port, "/hold"));
    seen_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let outcome = fixture.gateway.restart(Duration::from_millis(150));
    assert_eq!(
        outcome,
        Err(ag_relay_lib::gateway_connection::ConnectionError::Timeout)
    );
    assert_eq!(
        fixture.gateway.managed_supervisor_pid(),
        original_pid,
        "超时不能丢失或换掉句柄"
    );
    let status: serde_json::Value = serde_json::from_str(
        &ag_relay_lib::gateway_status::fetch_status_body("127.0.0.1", port).unwrap(),
    )
    .unwrap();
    assert_eq!(status["draining"], false);
    assert_eq!(status["activeRequests"], 1);
    release_tx.send(()).unwrap();
    assert!(
        client.join().unwrap().contains("{\"ok\":true}"),
        "原请求必须完整完成"
    );
    assert!(
        request(port, "/health").contains("{\"ok\":true}"),
        "完成后仍接受新请求，不能延迟退出"
    );
    server.join().unwrap();
    assert!(
        fixture
            .gateway
            .safe_shutdown(Duration::from_secs(3))
            .unwrap(),
        "稍后可再次安全停机"
    );
}

#[test]
fn 连接切换_保存不打断旧实例_明确重启才注入并释放旧端口() {
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for expected_path in ["/health", "/replacement/health"] {
            let (mut stream, _) = upstream.accept().unwrap();
            let mut bytes = [0; 8192];
            let count = stream.read(&mut bytes).unwrap();
            let received = String::from_utf8_lossy(&bytes[..count]);
            assert!(received.contains("fixture-only-not-a-key"));
            assert!(received.starts_with(&format!("GET {expected_path} HTTP/")));
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true}").unwrap();
        }
    });
    let first = free_port();
    let second = free_port();
    let upstream_url = format!("http://127.0.0.1:{upstream_port}");
    let mut fixture = Fixture::new("switch", &upstream_url, first);
    fixture.gateway.start().unwrap();
    let saved = fixture.gateway.save(&format!("{upstream_url}/replacement"), second.into()).unwrap();
    assert!(saved.restart_required);
    assert_eq!(saved.endpoint, format!("http://127.0.0.1:{first}"));
    assert!(request(first, "/health").contains("{\"ok\":true}"));
    assert!(fixture.gateway.restart(Duration::from_secs(3)).unwrap());
    assert!(!fixture.gateway.settings().unwrap().restart_required);
    assert!(TcpListener::bind(("127.0.0.1", first)).is_ok());
    assert!(request(second, "/health").contains("{\"ok\":true}"));
    assert!(fixture
        .gateway
        .safe_shutdown(Duration::from_secs(3))
        .unwrap());
    assert!(!fixture.gateway.holds_managed());
    server.join().unwrap();
    let diagnostics = std::fs::read_to_string(fixture.dir.join("logs/diagnostics.jsonl")).unwrap();
    assert!(!diagnostics.contains("fixture-only-not-a-key"));
    let settings = std::fs::read_to_string(fixture.dir.join("connection.json")).unwrap();
    assert!(!settings.contains("fixture-only-not-a-key"));
}

// 重试回归共用既有真实核心/假上游接缝。只绑定动态本机端口，清理自建监听器和受管进程。
struct RetryUpstream {
    url: String,
    count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl RetryUpstream {
    fn new(replies: Vec<String>) -> Self {
        use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        listener.set_nonblocking(true).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, done) = (count.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
                        let mut request = Vec::new();
                        let mut chunk = [0; 4096];
                        // 读取完整 HTTP 请求，便于验证 POST 的兼容修复及保留原始历史。
                        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                            match stream.read(&mut chunk) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => request.extend_from_slice(&chunk[..n]),
                            }
                        }
                        if let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n").map(|n| n + 4) {
                            let headers = String::from_utf8_lossy(&request[..header_end]);
                            let length = headers.lines().find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok()).flatten()
                            }).unwrap_or(0);
                            while request.len() < header_end + length {
                                match stream.read(&mut chunk) {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                                }
                            }
                        }
                        let index = seen.fetch_add(1, Ordering::SeqCst);
                        let reply = &replies[index.min(replies.len() - 1)];
                        let _ = stream.write_all(reply.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
                    Err(e) => panic!("自建假上游监听失败：{e}"),
                }
            }
        });
        Self { url, count, stop, worker: Some(worker) }
    }
    fn attempts(&self) -> usize { self.count.load(std::sync::atomic::Ordering::SeqCst) }
}
impl Drop for RetryUpstream {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn upstream_reply(status: u16, content_type: &str, body: &str) -> String {
    format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}
fn retry_status(port: u16) -> serde_json::Value {
    serde_json::from_str(&ag_relay_lib::gateway_status::fetch_status_body("127.0.0.1", port).unwrap()).unwrap()
}
fn wait_retry_status(port: u16, check: impl Fn(&serde_json::Value) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let status = retry_status(port);
        if check(&status) { return; }
        assert!(std::time::Instant::now() < deadline, "未出现预期的隔离请求状态：{status}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn 重试边界_退避期间取消后不再访问上游() {
    let upstream = RetryUpstream::new(vec![
        upstream_reply(503, "application/json", r#"{"error":{"type":"server_error","message":"fixture temporary outage"}}"#),
        upstream_reply(200, "application/json", r#"{"ok":true}"#),
    ]);
    let port = free_port();
    let mut fixture = Fixture::new("cancel-backoff", &upstream.url, port);
    fixture.gateway.start().unwrap();
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(client, "GET /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer fixture-only-not-a-key\r\n\r\n").unwrap();
    // HTTP 错误响应已读到，第一轮尚未退出；25ms 远小于既有首次退避最短 500ms。
    wait_retry_status(port, |s| s["requests"][0]["phase"] == "reading_body");
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(upstream.attempts(), 1);
    client.shutdown(std::net::Shutdown::Both).unwrap();
    drop(client);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(retry_status(port)["activeRequests"], 0, "取消后应立即释放请求，不继续占着退避计时器");
    std::thread::sleep(Duration::from_millis(1300));
    assert_eq!(upstream.attempts(), 1, "客户端已取消，不能再发起上游请求");
    wait_retry_status(port, |s| s["activeRequests"] == 0);
}


#[test]
fn 重试边界_确定性错误在流式与普通响应中都不重试() {
    let error = r#"{"error":{"code":"context_length_exceeded","message":"maximum context length exceeded"}}"#;
    for (name, status, content_type, body) in [
        ("http", 400, "application/json", error.to_string()),
        ("sse", 200, "text/event-stream", format!("data: {error}\n\n")),
    ] {
        let upstream = RetryUpstream::new(vec![
            upstream_reply(status, content_type, &body),
            upstream_reply(200, "application/json", r#"{"ok":true}"#),
        ]);
        let port = free_port();
        let mut fixture = Fixture::new(&format!("permanent-{name}"), &upstream.url, port);
        fixture.gateway.start().unwrap();
        let response = request(port, "/v1/chat/completions");
        assert_eq!(upstream.attempts(), 1, "{name} 确定性错误不能重复发送");
        assert!(response.starts_with("HTTP/1.1 400"), "{name}: {response}");
        assert!(!response.contains(r#"{"ok":true}"#));
        if name == "sse" {
            let diagnostics = std::fs::read_to_string(fixture.dir.join("logs/diagnostics.jsonl")).unwrap();
            let failure: serde_json::Value = diagnostics.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()).find(|r| r["event"] == "precommit_failure").unwrap();
            assert_eq!(failure["retryable"], false);
            assert!(!diagnostics.contains("maximum context length exceeded"));
        }
    }
}


#[test]
fn 重试边界_确定性错误码与文案保持一致() {
    let errors = [
        ("context", r#"{"error":{"code":"context_length_exceeded","message":"provider error"}}"#, 400),
        ("auth", r#"{"error":{"code":"invalid_api_key"}}"#, 400),
        ("quota", r#"{"error":{"code":"insufficient_quota"}}"#, 402),
        ("model", r#"{"error":{"type":"invalid_request_error","message":"model not found"}}"#, 400),
        ("policy", r#"{"error":{"message":"content filter blocked the request"}}"#, 400),
    ];
    for (name, error, expected) in errors {
        for (carrier, ct, body) in [
            ("sse", "text/event-stream", format!("data: {error}\n\n")),
            ("json", "application/json", error.to_string()),
        ] {
            let upstream = RetryUpstream::new(vec![upstream_reply(200, ct, &body), upstream_reply(200, "application/json", r#"{"ok":true}"#)]);
            let port = free_port();
            let mut fixture = Fixture::new(&format!("codes-{name}-{carrier}"), &upstream.url, port);
            fixture.gateway.start().unwrap();
            let response = request(port, "/v1/chat/completions");
            assert_eq!(upstream.attempts(), 1, "{name}/{carrier} 不应重试");
            assert!(response.starts_with(&format!("HTTP/1.1 {expected}")), "{name}/{carrier}: {response}");
        }
    }
}

#[test]
fn 重试边界_临时错误与部署缺失仍可恢复_未知流错误保留原策略() {
    for (name, status, ct, body) in [
        ("http-429", 429, "application/json", r#"{"error":{"code":"rate_limit_exceeded","message":"try again"}}"#.to_string()),
        ("http-503", 503, "application/json", r#"{"error":{"message":"temporary outage"}}"#.to_string()),
        ("sse-rate", 200, "text/event-stream", "data: {\"error\":{\"code\":\"rate_limit_exceeded\"}}\n\n".to_string()),
        ("sse-unknown", 200, "text/event-stream", "data: {\"error\":{\"code\":\"future_provider_error\",\"message\":\"unclassified fixture\"}}\n\n".to_string()),
        ("sse-generic", 200, "text/event-stream", "data: {\"error\":{\"type\":\"invalid_request_error\",\"message\":\"provider error\"}}\n\n".to_string()),
        ("sse-wrapped-rate", 200, "text/event-stream", "data: {\"error\":{\"type\":\"invalid_request_error\",\"message\":\"rate limit exceeded\"}}\n\n".to_string()),
        ("sse-deployment", 200, "text/event-stream", "data: {\"error\":{\"type\":\"invalid_request_error\",\"message\":\"no deployment to match this model\"}}\n\n".to_string()),
        ("json-deployment", 200, "application/json", r#"{"error":{"type":"invalid_request_error","message":"no deployment to match this model"}}"#.to_string()),
        ("http-deployment", 400, "application/json", r#"{"error":{"type":"invalid_request_error","message":"no deployment to match this model"}}"#.to_string()),
        ("json-temporary", 200, "application/json", r#"{"error":{"code":"server_error","message":"temporary outage"}}"#.to_string()),
    ] {
        let upstream = RetryUpstream::new(vec![upstream_reply(status, ct, &body), upstream_reply(200, "application/json", r#"{"ok":true}"#)]);
        let port = free_port();
        let mut fixture = Fixture::new(name, &upstream.url, port);
        fixture.gateway.start().unwrap();
        let response = request(port, "/v1/chat/completions");
        assert_eq!(upstream.attempts(), 2, "{name} 应保留可恢复重试");
        assert!(response.contains(r#"{"ok":true}"#), "{name}: {response}");
        let diagnostics = std::fs::read_to_string(fixture.dir.join("logs/diagnostics.jsonl")).unwrap();
        assert!(!diagnostics.contains("fixture-only-not-a-key"));
        assert!(!diagnostics.contains("unclassified fixture"));
    }
}

#[test]
fn 重试边界_内容下发后不重新生成_成功流不改写内容() {
    for (name, ending) in [
        ("complete", "data: [DONE]\n\n"),
        ("failed", "data: {\"type\":\"error\",\"error\":{\"code\":\"context_length_exceeded\",\"message\":\"maximum context length exceeded\"}}\n\n"),
    ] {
        let body = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"fixture text\"}}}}]}}\n\n{ending}");
        let upstream = RetryUpstream::new(vec![upstream_reply(200, "text/event-stream", &body)]);
        let port = free_port();
        let mut fixture = Fixture::new(&format!("committed-{name}"), &upstream.url, port);
        fixture.gateway.start().unwrap();
        let response = request(port, "/v1/chat/completions");
        assert!(response.contains(&body), "{name} 必须保留流内容和终止事件");
        assert_eq!(upstream.attempts(), 1, "已下发内容后不得重放整个请求");
    }
}


#[test]
fn 重试边界_流式退避与快速换通道等待期间均可取消() {
    for (name, error) in [
        ("sse-cancel", r#"{"error":{"code":"rate_limit_exceeded"}}"#),
        ("deployment-cancel", r#"{"error":{"message":"no deployment to match"}}"#),
    ] {
        let upstream = RetryUpstream::new(vec![
            upstream_reply(200, "text/event-stream", &format!("data: {error}\n\n")),
            upstream_reply(200, "application/json", r#"{"ok":true}"#),
        ]);
        let port = free_port();
        let mut fixture = Fixture::new(name, &upstream.url, port);
        fixture.gateway.start().unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(client, "GET /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
        wait_retry_status(port, |s| s["requests"][0]["phase"] == "waiting_first_content");
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(upstream.attempts(), 1);
        client.shutdown(std::net::Shutdown::Both).unwrap();
        drop(client);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(retry_status(port)["activeRequests"], 0, "{name} 取消后立即释放在途请求");
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(upstream.attempts(), 1, "{name} 取消后不得建立新尝试");
    }
}

#[test]
fn 重试边界_等待响应头及流式读取期间取消会中断当前上游() {
    for streaming in [false, true] {
        use std::sync::mpsc;
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", upstream.local_addr().unwrap().port());
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 8192];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
            }
            if streaming {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"fixture\"}}]}\n\n").unwrap();
            }
            tx.send(()).unwrap();
            match stream.read(&mut buf) {
                Ok(0) => true,
                Err(e) => matches!(e.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted),
                _ => false,
            }
        });
        let port = free_port();
        let mut fixture = Fixture::new(&format!("cancel-inflight-{streaming}"), &url, port);
        fixture.gateway.start().unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(client, "GET /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        wait_retry_status(port, |s| s["requests"][0]["phase"] == if streaming { "text_output" } else { "waiting_headers" });
        client.shutdown(std::net::Shutdown::Both).unwrap();
        drop(client);
        assert!(server.join().unwrap(), "取消必须断开当前上游，不等待读取超时");
        wait_retry_status(port, |s| s["activeRequests"] == 0);
    }
}


#[test]
fn 重试边界_精确匹配的状态兼容修复优先于确定性分类() {
    let error = r#"{"error":{"type":"invalid_request_error","message":"Unknown parameter: 'input[0].status'."}}"#;
    for (carrier, status, ct, error_body) in [
        ("http", 400, "application/json", error.to_string()),
        ("json", 200, "application/json", error.to_string()),
        ("sse", 200, "text/event-stream", format!("data: {error}\n\n")),
    ] {
        let upstream = RetryUpstream::new(vec![upstream_reply(status, ct, &error_body), upstream_reply(200, "application/json", r#"{"ok":true}"#)]);
        let port = free_port();
        let mut fixture = Fixture::new(&format!("status-repair-{carrier}"), &upstream.url, port);
        fixture.gateway.start().unwrap();
        let body = r#"{"input":[{"type":"message","role":"user","content":"fixture text","status":"completed"}]}"#;
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        write!(client, "POST /v1/responses HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert_eq!(upstream.attempts(), 2, "{carrier} 应先尝试已有的安全兼容修复");
        assert!(response.contains(r#"{"ok":true}"#), "{carrier}: {response}");
        let diagnostics = std::fs::read_to_string(fixture.dir.join("logs/diagnostics.jsonl")).unwrap();
        assert!(diagnostics.contains("status_repair_applied"));
        assert!(!diagnostics.contains("fixture text"));
    }
}

/// 手动性能观察：只发成功请求，基线脚本必须显式提供，绝不连接现用实例。
#[test]
#[ignore = "手动隔离性能对照，不属于默认回归"]
fn 速度观察_成功转发基线对照() {
    let baseline = std::fs::read_to_string(std::env::var_os("AG_RELAY_BENCH_BASELINE").expect("提供基线脚本路径")).unwrap();
    let output = "x".repeat(65536);
    let body = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{output}\"}}}}]}}\n\ndata: [DONE]\n\n");
    for (round, use_baseline) in [true, false, false, true].into_iter().enumerate() {
        let upstream = RetryUpstream::new(vec![upstream_reply(200, "text/event-stream", &body)]);
        let port = free_port();
        let mut fixture = Fixture::new(&format!("speed-{round}"), &upstream.url, port);
        if use_baseline { std::fs::write(fixture.dir.join("gateway/relay-retry-proxy.mjs"), &baseline).unwrap(); }
        fixture.gateway.start().unwrap();
        let mut samples = Vec::new();
        for n in 0..45 {
            let start = std::time::Instant::now();
            let response = request(port, "/v1/chat/completions");
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            assert!(response.contains(&body));
            if n >= 5 { samples.push(elapsed); }
        }
        samples.sort_by(f64::total_cmp);
        println!("{} round={} n=40 median_ms={:.3} p95_ms={:.3}", if use_baseline { "baseline" } else { "candidate" }, round + 1, samples[20], samples[38]);
        assert_eq!(upstream.attempts(), 45, "成功请求不得产生额外上游尝试");
    }
}
