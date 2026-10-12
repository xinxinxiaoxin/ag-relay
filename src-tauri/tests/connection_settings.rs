use ag_relay_lib::connection_settings::ConnectionSettingsStore;

#[test]
fn 连接设置_并发读取不应遇到写入一半的文件() {
    let dir = std::env::temp_dir().join(format!("ag-relay-settings-atomic-{}", std::process::id()));
    let path = dir.join("connection.json");
    let store = ConnectionSettingsStore::new(path.clone());
    store.save("https://example.test", 18887).unwrap();
    let writer = std::thread::spawn(move || {
        for port in 19000..19500 {
            store.save("https://example.test", port).unwrap();
        }
    });
    let reader = ConnectionSettingsStore::new(path);
    let mut read_error = None;
    while !writer.is_finished() {
        if let Err(error) = reader.load() {
            read_error = Some(error);
        }
    }
    writer.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(read_error, None);
}

#[test]
fn 连接设置_重开后环境注入只覆盖连接项且保持本机监听() {
    let dir = std::env::temp_dir().join(format!("ag-relay-settings-env-{}", std::process::id()));
    let store = ConnectionSettingsStore::new(dir.join("connection.json"));
    store.save("http://127.0.0.1:18999/base", 18998).unwrap();
    let pairs = ag_relay_lib::gateway_env::gateway_env_pairs_with_settings(
        &dir.join("diagnostics.jsonl"),
        &store.load().unwrap(),
    );
    let env: std::collections::HashMap<_, _> = pairs.into_iter().collect();
    assert_eq!(
        env.get("PROXY_UPSTREAM").unwrap(),
        "http://127.0.0.1:18999/base"
    );
    assert_eq!(env.get("PROXY_PORT").unwrap(), "18998");
    assert_eq!(env.get("PROXY_HOST").unwrap(), "127.0.0.1");
    assert_eq!(env.get("PROXY_MAX_ATTEMPTS").unwrap(), "110");
    assert_eq!(env.len(), 18);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn 连接设置_拒绝非法地址端口和凭据且不覆盖上次有效设置() {
    let dir =
        std::env::temp_dir().join(format!("ag-relay-settings-invalid-{}", std::process::id()));
    let path = dir.join("connection.json");
    let store = ConnectionSettingsStore::new(path.clone());
    let valid = store.save("https://example.test", 19876).unwrap();
    for address in [
        "",
        "example.test",
        "file:///tmp/file",
        "https://",
        "https://test.invalid/\nsecret",
        "https://user:fake-token@test.invalid",
        "https://test.invalid?key=fake-token",
        "https://test.invalid/#fake-token",
        "https://test.invalid\\path",
    ] {
        assert!(store.save(address, 19876).is_err(), "应拒绝非法地址");
        assert_eq!(store.load().unwrap(), valid);
    }
    for port in [0, 65536, u32::MAX] {
        assert!(store.save("https://example.test", port).is_err());
        assert_eq!(store.load().unwrap(), valid);
    }
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"upstream": "https://example.test", "port": 19876})
    );
    std::fs::write(
        &path,
        br#"{"upstream":"https://example.test","port":19876,"key":"fake-token"}"#,
    )
    .unwrap();
    assert!(store.load().is_err(), "未知字段不能静默托管");
    std::fs::write(&path, br#"{"upstream":"file:///tmp/file","port":0}"#).unwrap();
    assert!(store.load().is_err(), "落盘文件也必须校验");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn 连接设置_临时路径保存后重新打开仍保留单上游和端口() {
    let dir = std::env::temp_dir().join(format!("ag-relay-settings-{}", std::process::id()));
    let path = dir.join("connection.json");
    let store = ConnectionSettingsStore::new(path.clone());
    let defaults = store.load().expect("出厂默认值");
    assert_eq!(defaults.upstream(), "https://agentrouter.org");
    assert_eq!(defaults.port(), 8787);
    assert_eq!(defaults.host(), "127.0.0.1");
    store
        .save("https://example.test/relay/", 18787)
        .expect("保存设置");
    let loaded = ConnectionSettingsStore::new(path)
        .load()
        .expect("重开后读取");
    assert_eq!(loaded.upstream(), "https://example.test/relay");
    assert_eq!(loaded.port(), 18787);
    assert_eq!(loaded.host(), "127.0.0.1");
    std::fs::remove_dir_all(dir).unwrap();
}
