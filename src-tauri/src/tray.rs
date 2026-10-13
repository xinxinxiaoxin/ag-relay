//! 系统托盘（T5）：常驻图标 + 左键单击显示/隐藏面板 + 右键菜单「打开面板 / 安全重启 / 退出」。
//! 菜单 id 与动作的映射是纯函数（单测锁定，构建菜单与分发共用同一真值）；
//! 「安全重启」直接调用 T4 命令层 restart_gateway 同一函数（与面板按钮完全同一代码路径）；
//! 「退出」走统一等待型安全退出（TrayMenu），失败显示面板并通知用户。
//! 关窗选择「后台运行」只隐藏面板，托盘与受管网关继续保留。

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use crate::app_exit::{self, ExitSource};

/// 托盘图标 id（当前仅一个托盘；显式命名便于诊断）。
pub const TRAY_ID: &str = "main-tray";

/// 菜单项 id：on_menu_event 按 id 分发；与 tray_menu_items() 同源，单测锁定不漂移。
pub const MENU_OPEN_PANEL: &str = "open-panel";
pub const MENU_RESTART_GATEWAY: &str = "restart-gateway";
pub const MENU_QUIT: &str = "quit";

/// 托盘菜单项定义（id + 中文文案），构建菜单与单测共用同一真值。
pub fn tray_menu_items() -> &'static [(&'static str, &'static str)] {
    &[
        (MENU_OPEN_PANEL, "打开面板"),
        (MENU_RESTART_GATEWAY, "安全重启"),
        (MENU_QUIT, "退出"),
    ]
}

/// 菜单 id → 托盘动作（纯函数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenPanel,
    RestartGateway,
    Quit,
}

pub fn resolve_tray_action(id: &str) -> Option<TrayAction> {
    match id {
        MENU_OPEN_PANEL => Some(TrayAction::OpenPanel),
        MENU_RESTART_GATEWAY => Some(TrayAction::RestartGateway),
        MENU_QUIT => Some(TrayAction::Quit),
        _ => None,
    }
}

/// 主窗口显示并置前（托盘菜单「打开面板」与左键唤起共用）。
pub fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// 左键单击：面板可见（且未最小化）→ 隐藏；否则唤起显示。
/// 最小化也算「待唤起」：点托盘应恢复窗口而不是把最小化窗口藏起来。
pub fn toggle_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let visible = win.is_visible().unwrap_or(false) && !win.is_minimized().unwrap_or(false);
        if visible {
            let _ = win.hide();
        } else {
            show_main_window(app);
        }
    }
}

/// 处理托盘动作（on_menu_event 的分发目标；左键切换与右键菜单共用本模块）。
pub fn handle_tray_action(app: &AppHandle, action: TrayAction) {
    match action {
        TrayAction::OpenPanel => show_main_window(app),
        TrayAction::RestartGateway => {
            // 与面板「安全重启」按钮同一命令函数：托盘不另写路径
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _result = crate::gateway_commands::restart_gateway(app).await;
            });
        }
        TrayAction::Quit => app_exit::request_exit_from_event(app, Some(ExitSource::TrayMenu)),
    }
}

/// 构建托盘（setup 内调用一次）：AG 图标 + tooltip + 右键菜单 + 事件接线。
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::new(app)?;
    for (id, label) in tray_menu_items() {
        menu.append(&MenuItem::with_id(app, *id, *label, true, None::<&str>)?)?;
    }

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tauri::include_image!("icons/tray.png"))
        .tooltip("AG Relay · 网关")
        .menu(&menu)
        // 左键不弹菜单：单击留给「显示/隐藏面板」切换，菜单只走右键
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| {
            if let Some(action) = resolve_tray_action(event.id.as_ref()) {
                handle_tray_action(app, action);
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 托盘菜单三项_中文文案与顺序固定() {
        let items = tray_menu_items();
        assert_eq!(items.len(), 3, "菜单必须是固定的三项");
        assert_eq!(items[0], (MENU_OPEN_PANEL, "打开面板"));
        assert_eq!(items[1], (MENU_RESTART_GATEWAY, "安全重启"));
        assert_eq!(items[2], (MENU_QUIT, "退出"));
    }

    #[test]
    fn 菜单id非空且互不重复() {
        let ids: Vec<_> = tray_menu_items().iter().map(|(id, _)| *id).collect();
        assert!(ids.iter().all(|id| !id.is_empty()), "菜单 id 不得为空");
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "菜单 id 不得重复：{a} vs {b}");
            }
        }
    }

    #[test]
    fn 每个菜单id都有动作映射_分发不丢项() {
        assert_eq!(
            resolve_tray_action(MENU_OPEN_PANEL),
            Some(TrayAction::OpenPanel)
        );
        assert_eq!(
            resolve_tray_action(MENU_RESTART_GATEWAY),
            Some(TrayAction::RestartGateway)
        );
        assert_eq!(resolve_tray_action(MENU_QUIT), Some(TrayAction::Quit));
        for (id, _) in tray_menu_items() {
            assert!(
                resolve_tray_action(id).is_some(),
                "菜单项 {id} 缺少动作映射"
            );
        }
    }

    #[test]
    fn 未知菜单id不触发任何动作() {
        assert_eq!(resolve_tray_action(""), None);
        assert_eq!(resolve_tray_action("unknown"), None);
        assert_eq!(resolve_tray_action("quit "), None, "带空白不算有效 id");
        assert_eq!(resolve_tray_action("QUIT"), None, "大小写敏感，不算有效 id");
    }
}
