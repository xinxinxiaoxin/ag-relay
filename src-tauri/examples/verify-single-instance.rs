//! 隔离单实例验证：不启动网关、不读写用户数据、不初始化更新/自启。
//! 用法：verify-single-instance <唯一标识> <证据文件> [--hold-before-build]
//! 屏障开启时，持有真实启动锁并等待同目录 events.release；最多等待 10 秒。
use ag_relay_lib::single_instance::{ActivationRequests, StartupGuard};
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn record(path: &std::path::Path, event: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    // 一次 append 写完整行，避免多个进程的格式化片段交错。
    file.write_all(format!("{event} {}\n", std::process::id()).as_bytes())
        .unwrap();
}

fn main() {
    let mut args = std::env::args().skip(1);
    let id = args.next().expect("isolated identifier");
    assert!(id.starts_with("ag-relay-single-instance-test-"));
    let evidence = PathBuf::from(args.next().expect("evidence file"));
    let hold_before_build = match args.next().as_deref() {
        None => false,
        Some("--hold-before-build") => true,
        _ => panic!("unknown verification option"),
    };
    assert!(args.next().is_none());
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = id;
    context.config_mut().app.windows.clear();
    record(&evidence, "gate_waiting");
    let startup = StartupGuard::acquire(&context.config().identifier).unwrap();
    record(&evidence, "gate_acquired");
    if hold_before_build {
        record(&evidence, "holding_before_build");
        let release = evidence.with_extension("release");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !release.exists() {
            assert!(Instant::now() < deadline, "startup barrier was not released");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let activation = Arc::new(ActivationRequests::default());
    let callback_activation = activation.clone();
    let callback_evidence = evidence.clone();
    let setup_evidence = evidence.clone();
    record(&evidence, "build_started");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(move |_app, _argv, _cwd| {
            record(&callback_evidence, "callback");
            if callback_activation.request() {
                record(&callback_evidence, "activate");
            }
        }))
        .setup(move |_app| {
            record(&setup_evidence, "setup");
            Ok(())
        })
        .build(context)
        .expect("isolated single instance verification failed");
    record(&evidence, "plugin_ready");
    drop(startup);
    app.run(move |_app, event| {
        if matches!(event, tauri::RunEvent::Ready) {
            record(&evidence, "ready");
            if activation.mark_ready() {
                record(&evidence, "activate");
            }
        }
    });
}
