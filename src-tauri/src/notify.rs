//! 系统通知命令。
//!
//! 策略（与 dsh-rtui 验证过的 notify_when_obscured 一致）：仅当主窗口被"遮蔽"
//! （不可见 / 最小化 / 失焦）时才弹系统通知；窗口可见且聚焦时界面本身在展示，
//! 不弹，避免打扰正在看界面的用户。
//!
//! 点击处理：`tauri-plugin-notification` 桌面端调用 `let _ = notification.show()`，
//! 把激活句柄直接丢弃，点击 toast 的事件收不到，窗口不会带到前台。这里直接用
//! `notify-rust` 弹通知并保留句柄，等待用户"默认激活"（点击 toast 主体）后把主窗口
//! 显示/还原/聚焦。

use tauri::Manager;

/// 窗口"遮蔽"判断：主窗口不可见、最小化，或可见但失焦（用户在别的窗口干活）。
/// 仅当窗口可见且聚焦时不通知。
#[cfg(windows)]
fn window_obscured(app: &tauri::AppHandle) -> bool {
    match app.get_webview_window("main") {
        Some(w) => match w.is_visible() {
            // Windows 下最小化窗口 IsWindowVisible 仍为 true；最小化/失焦都算"不在看"。
            Ok(true) => w.is_minimized().unwrap_or(false) || !w.is_focused().unwrap_or(false),
            _ => true, // 隐藏或无法判断 → 通知
        },
        None => true,
    }
}

#[cfg(not(windows))]
fn window_obscured(_app: &tauri::AppHandle) -> bool {
    true
}

/// 把主窗口带到前台：还原最小化 → 显示（隐藏到托盘时）→ 聚焦。
fn bring_main_to_front(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[tauri::command]
pub fn notify(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    if !window_obscured(&app) {
        return Ok(());
    }

    let mut notification = notify_rust::Notification::new();
    notification
        .summary(&title)
        .body(&body)
        .auto_icon();
    // toast 归属到与 aumid::register 一致的 AUMID（ai.iyam.dsh），
    // 保证点击激活回调正确关联到本应用。仅 Windows 的 toast 需要 AUMID。
    #[cfg(windows)]
    notification.app_id(&app.config().identifier);

    match notification.show() {
        Ok(handle) => {
            let app = app.clone();
            std::thread::spawn(move || {
                // 点击 toast 主体触发 Default 激活（本应用无按钮，Default 是唯一激活入口）。
                // 收到激活后把主窗口带到前台；关闭/超时则什么都不做。
                let _ =
                    handle.wait_for_response(move |resp: &notify_rust::NotificationResponse| {
                        match resp {
                            notify_rust::NotificationResponse::Default
                            | notify_rust::NotificationResponse::Action(_) => {
                                bring_main_to_front(&app);
                            }
                            _ => {}
                        }
                    });
            });
        }
        Err(e) => {
            log::warn!("notification show failed: {}", e);
        }
    }
    Ok(())
}
