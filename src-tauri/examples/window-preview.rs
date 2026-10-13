use tauri::WebviewWindowBuilder;

fn main() {
    let mut context = tauri::generate_context!();
    context.set_default_window_icon(Some(tauri::include_image!("icons/icon.png")));
    let mut window = context.config_mut().app.windows.remove(0);
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = "ag-relay-window-preview".into();
    window.title = "AG Relay · 窗口验证（不连接网关）".into();
    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::from_config(app, &window)?
                .initialization_script("delete window.__TAURI__.core;")
                .build()?;
            tauri::tray::TrayIconBuilder::with_id("preview-tray")
                .icon(tauri::include_image!("icons/tray.png"))
                .tooltip("AG Relay · 图标验证（不连接网关）")
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                println!("WINDOW_CLOSE_REQUESTED: {}", window.label());
            }
        })
        .run(context)
        .expect("window preview failed");
}
