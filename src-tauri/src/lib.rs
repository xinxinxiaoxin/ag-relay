pub mod autostart;
pub mod autostart_commands;
pub mod gateway_commands;
pub mod gateway_env;
pub mod gateway_process;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(gateway_commands::GatewayState::default())
        .manage(autostart_commands::AutostartUiState::default())
        .setup(|app| {
            // T6：应用启动即幂等编排开机自启（建立/改写自启项 + 清理确认属于本网关的旧快捷方式）
            autostart_commands::sync_on_launch(app.handle());
            // 规范生命周期「软件启动→网关未运行则拉起」：自启软件本身，它再拉起网关；
            // 8787 已有网关则接管，不重复拉起
            gateway_commands::auto_start_on_launch(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            gateway_commands::start_gateway,
            gateway_commands::stop_gateway,
            autostart_commands::get_autostart,
            autostart_commands::set_autostart_enabled
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // 关窗即退出的默认链；退出前同步优雅停网关（≤10 秒），幂等
                gateway_commands::shutdown_blocking_on_exit(app);
            }
        });
}
