pub mod gateway_commands;
pub mod gateway_env;
pub mod gateway_process;
pub mod gateway_status;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(gateway_commands::GatewayState::default())
        .invoke_handler(tauri::generate_handler![
            gateway_commands::start_gateway,
            gateway_commands::stop_gateway,
            gateway_status::get_status
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
