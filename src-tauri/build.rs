fn main() {
    tauri_build::build();
    // MockRuntime 会链接 TaskDialogIndirect，lib 单测不像成品 EXE 自动嵌入 v6 控件清单。
    // 仅显式启用下载接缝测试时补测试产物清单，不安装或替换任何系统组件。
    if cfg!(windows) && std::env::var_os("CARGO_FEATURE_UPDATER_DOWNLOAD_TESTS").is_some() {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
}
