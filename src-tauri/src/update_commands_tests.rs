//! 真实 updater 下载验签接缝：仅随机本地 HTTP、公开测试公钥和非安装器文本；不启动桌面/网关。
use super::*;
use base64::Engine;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

struct ReleaseEndpoint {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl ReleaseEndpoint {
    fn new(manifest_version: &str, signature: &str, payload: Vec<u8>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        assert!(![8787, 3000, 3001, 4000].contains(&address.port()));
        let url = format!("http://{address}/latest.json");
        let manifest = serde_json::json!({
            "version": manifest_version,
            "notes": "隔离签名测试",
            "platforms": { "windows-x86_64-nsis": { "signature": signature, "url": format!("http://{address}/payload") } }
        }).to_string();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let worker = std::thread::spawn(move || {
            while !signal.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = [0; 4096];
                        let Ok(n) = socket.read(&mut request) else {
                            continue;
                        };
                        let body = if request[..n].starts_with(b"GET /latest.json ") {
                            manifest.as_bytes()
                        } else {
                            &payload
                        };
                        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n", body.len());
                        let _ = socket
                            .write_all(header.as_bytes())
                            .and_then(|_| socket.write_all(body));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for ReleaseEndpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

struct UntouchedGateway;
impl UpdateGateway for UntouchedGateway {
    fn identity(&mut self) -> Result<GatewayIdentity, String> {
        Ok(GatewayIdentity {
            running: true,
            token: "测试网关".into(),
        })
    }
    fn safe_shutdown(&mut self, _: Duration) -> Result<bool, ShutdownFailure> {
        panic!("坏签名不能进入排空")
    }
    fn restore(&mut self) -> Result<(), String> {
        panic!("坏签名不能改动网关")
    }
}
impl GatewayHost for UntouchedGateway {
    type Gateway = Self;
    fn with_gateway<T>(&self, f: impl FnOnce(&mut Self) -> T) -> Result<T, String> {
        Ok(f(&mut UntouchedGateway))
    }
}

#[test]
fn 插件真实下载_完整签名可读_篡改与版本冒充在完成回调后仍拒绝() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../tests/fixtures/update-signature.json")).unwrap();
    let payload = base64::engine::general_purpose::STANDARD
        .decode(fixture["payloadBase64"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        payload,
        b"AG Relay isolated updater test fixture - NOT AN INSTALLER"
    );
    // 只建立 Tauri 内存 MockRuntime，不调用本产品 run/setup，不写注册表或真实 AppData。
    for (version, tampered) in [("0.3.1", false), ("0.3.1", true), ("0.3.2", false)] {
        let mut bytes = payload.clone();
        if tampered {
            bytes[0] ^= 1;
        }
        let server = ReleaseEndpoint::new(version, fixture["signature"].as_str().unwrap(), bytes);
        let config = serde_json::json!({
            "pubkey": fixture["pubkey"], "endpoints": [server.url],
            "requireSignedVersion": true, "dangerousInsecureTransportProtocol": true
        });
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context
            .config_mut()
            .plugins
            .0
            .insert("updater".into(), config);
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        let provider = InstalledRelease(app.handle().clone());
        let updater = app
            .updater_builder()
            .target("windows-x86_64-nsis")
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let candidate = tauri::async_runtime::block_on(updater.check())
            .unwrap()
            .unwrap();
        if version == "0.3.1" && !tampered {
            assert_eq!(
                tauri::async_runtime::block_on(provider.download(&candidate)).unwrap(),
                payload
            );
        } else {
            let finished = AtomicBool::new(false);
            assert!(tauri::async_runtime::block_on(candidate.download(
                |_, _| {},
                || {
                    finished.store(true, Ordering::Release);
                }
            ))
            .is_err());
            assert!(finished.load(Ordering::Acquire));
            struct Announced<'a> {
                candidate: Update,
                provider: &'a InstalledRelease<tauri::test::MockRuntime>,
            }
            impl UpdateProvider for Announced<'_> {
                type Candidate = Update;
                async fn check(&self) -> Result<Option<Release<Update>>, String> {
                    Ok(Some(Release {
                        version: self.candidate.version.clone(),
                        notes: String::new(),
                        candidate: self.candidate.clone(),
                    }))
                }
                async fn download(&self, candidate: &Update) -> Result<Vec<u8>, String> {
                    self.provider.download(candidate).await
                }
                fn install(&self, _: &Update, _: &[u8]) -> Result<(), String> {
                    panic!("测试绝不安装")
                }
            }
            let release = Announced {
                candidate,
                provider: &provider,
            };
            let updates = UpdateService::new(true, "0.2.0".into());
            let id = tauri::async_runtime::block_on(updates.check(&release))
                .unwrap()
                .release
                .unwrap()
                .id;
            assert!(tauri::async_runtime::block_on(updates.confirm(
                &release,
                &UntouchedGateway,
                &id
            ))
            .is_err());
            assert_eq!(updates.view().phase, crate::app_update::UpdatePhase::Failed);
        }
    }
}
