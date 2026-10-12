use tauri::Manager;

pub mod app_exit;
pub mod bounded_log;
pub mod autostart;
pub mod autostart_commands;
pub mod connection_settings;
pub mod gateway_commands;
pub mod gateway_connection;
pub mod gateway_safe_shutdown;
pub mod gateway_control;
pub mod gateway_diagnostics;
pub mod gateway_diagnostics_reader;
pub mod gateway_env;
pub mod gateway_process;
pub mod gateway_status;
pub mod timeutil;
pub mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 只由 Rust 安全更新服务调用；不给前端开放可绕过排空的 updater 安装权限。
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(gateway_commands::GatewayState::default())
        .manage(autostart_commands::AutostartUiState::default())
        .manage(gateway_diagnostics_reader::DiagnosticsState::default())
        .setup(|app| {
            // T6：应用启动即幂等编排开机自启（建立/改写自启项 + 清理确认属于本网关的旧快捷方式）
            autostart_commands::sync_on_launch(app.handle());
            // 规范生命周期「软件启动→网关未运行则拉起」：自启软件本身，它再拉起网关；
            // 8787 已有网关则接管，不重复拉起
            gateway_commands::auto_start_on_launch(app.handle());
            // T5：系统托盘常驻（左键切换面板 / 右键菜单）。失败不阻塞壳（面板仍可用），
            // 仅记录到 stderr（release 无控制台时不可见，托盘缺失属可见问题，不影响网关安全）
            if let Err(e) = tray::build_tray(app.handle()) {
                eprintln!("托盘初始化失败（面板不受影响）：{e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // T5：点 ×（含等价系统关窗）保持默认链放行 = 退出软件，不做最小化到托盘；
            // 只把退出来源记入壳侧日志。托盘「退出」/面板「退出」经 app.exit(0) 不经过本事件，
            // 三种来源在日志里可区分，最终都汇入下方 RunEvent::Exit 的优雅停网关钩子。
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if window.label() == "main" {
                    app_exit::log_exit_source(
                        window.app_handle(),
                        app_exit::ExitSource::WindowClose,
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            gateway_commands::get_connection_settings,
            gateway_commands::save_connection_settings,
            gateway_commands::start_gateway,
            gateway_commands::stop_gateway,
            gateway_status::get_status,
            gateway_commands::restart_gateway,
            gateway_commands::open_logs_dir,
            autostart_commands::get_autostart,
            autostart_commands::set_autostart_enabled,
            app_exit::quit_app,
            // T5：诊断读取（只读消费者）——取最近活动事件 + 分页查询（级别筛选 + 搜索）
            gateway_diagnostics_reader::get_recent_activity,
            gateway_diagnostics_reader::query_activity_events
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // 关窗即退出的默认链；托盘「退出」/面板「退出」经 app.exit(0) 同样汇入此钩子；
                // 退出前同步优雅停网关（≤10 秒），幂等
                gateway_commands::shutdown_blocking_on_exit(app);
            }
        });
}
