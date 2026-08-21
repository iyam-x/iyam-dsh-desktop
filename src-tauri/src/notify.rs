//! 系统通知命令。
//!
//! 策略（与 dsh-rtui 验证过的 notify_when_obscured 一致）：仅当主窗口被"遮蔽"
//! （不可见 / 最小化 / 失焦）时才弹系统通知；窗口可见且聚焦时界面本身在展示，
//! 不弹，避免打扰正在看界面的用户。
//!
//! 点击处理：点击系统通知要把主窗口带到前台（显示/还原/聚焦）。
//!
//! macOS 直接用 `mac_notification_sys` 弹通知并 `wait_for_click(true)`——整条 toast
//! 可点击，且 `send()` 会真正阻塞等待用户交互，点击后主线程的 `didActivateNotification`
//! 回调唤醒后台线程并拿到 `NotificationResponse::Click`。注意：`notify-rust` 的
//! `wait_for_response` 对"无按钮的纯点击"不会真正阻塞等待（其 `needs_response()` 为
//! false，底层 `should_wait=false`），点击激活收不到，故 macOS 不走 notify-rust。

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

    // macOS：直接走 mac_notification_sys，整条 toast 可点击（wait_for_click）。
    // 不走 notify-rust 的 wait_for_response——它对"无按钮的纯点击"不会真正阻塞等待，
    // 导致点击激活永远收不到（窗口无法被唤起）。
    #[cfg(target_os = "macos")]
    {
        // 显式指定本应用 bundle id：mac-notification-sys 首次发通知若未指定，会用
        // AppleScript 查询名为 "use_default" 的应用（不存在）→ 弹系统「Choose Application」
        // 对话框。这里一次性指定；重复调用返回 AlreadySet 错误，静默忽略。
        // set_application 含 NSBundle swizzle，须在调用线程（主线程）执行。
        let _ = mac_notification_sys::set_application(&app.config().identifier);

        // mac_notification_sys::Notification<'a> 以借用方式持有字符串，必须把 title/body
        // 一并移入后台线程，让被借用的 String 与通知同生命周期，否则 move 到线程会悬垂。
        let app2 = app.clone();
        std::thread::spawn(move || {
            let mut n = mac_notification_sys::Notification::default();
            n.title(&title).message(&body).wait_for_click(true);
            // send() 在后台线程阻塞，直到用户点击（主线程 didActivateNotification 回调唤醒）
            // 或通知自动消失。unminimize/show/set_focus 是 Cocoa 调用，必须回到主线程执行。
            match n.send() {
                Ok(mac_notification_sys::NotificationResponse::Click)
                | Ok(mac_notification_sys::NotificationResponse::ActionButton(_))
                | Ok(mac_notification_sys::NotificationResponse::Reply(_)) => {
                    let app_for_main = app2.clone();
                    let _ = app2.run_on_main_thread(move || {
                        bring_main_to_front(&app_for_main);
                    });
                }
                _ => {}
            }
        });
        return Ok(());
    }

    // 非 macOS：沿用 notify-rust（Windows 的 toast 走 AUMID 激活）。
    #[cfg(not(target_os = "macos"))]
    {
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
                    let _ = handle
                        .wait_for_response(move |resp: &notify_rust::NotificationResponse| {
                            match resp {
                                notify_rust::NotificationResponse::Default
                                | notify_rust::NotificationResponse::Action(_) => {
                                    let app2 = app.clone();
                                    let _ = app.run_on_main_thread(move || {
                                        bring_main_to_front(&app2);
                                    });
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
}
