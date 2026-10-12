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
