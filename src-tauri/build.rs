fn main() {
    let manifest_dir = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.parent().unwrap();
    for input in [
        "scripts/prepare-distribution.mjs",
        "scripts/check-licenses.mjs",
        "licenses",
        "LICENSE",
        "THIRD_PARTY_NOTICES.md",
        "package.json",
        "package-lock.json",
        "src-tauri/Cargo.toml",
        "src-tauri/Cargo.lock",
        "src-tauri/binaries/node-x86_64-pc-windows-msvc.exe",
        "src-tauri/generated/license-distribution",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    println!("cargo:rerun-if-env-changed=AG_RELAY_TEST_DISTRIBUTION");
    let prepared = std::process::Command::new("node")
        .arg(root.join("scripts/prepare-distribution.mjs"))
        .current_dir(root)
        .status()
        .expect("无法运行 Node 许可分发准备器；拒绝使用旧资源构建");
    assert!(prepared.success(), "许可分发准备失败；拒绝构建");
    tauri_build::build();
    if cfg!(windows) {
        // examples 不会像成品 EXE 一样自动嵌入 v6 控件清单。
        println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-examples=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
    // MockRuntime 会链接 TaskDialogIndirect，lib 单测不像成品 EXE 自动嵌入 v6 控件清单。
    // 仅显式启用下载接缝测试时补测试产物清单，不安装或替换任何系统组件。
    if cfg!(windows) && std::env::var_os("CARGO_FEATURE_UPDATER_DOWNLOAD_TESTS").is_some() {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
}
