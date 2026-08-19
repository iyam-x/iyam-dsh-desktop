//! 系统通知命令。
//!
//! 策略（与 dsh-rtui 验证过的 notify_when_obscured 一致）：仅当主窗口被"遮蔽"
//! （不可见 / 最小化 / 失焦）时才弹系统通知；窗口可见且聚焦时界面本身在展示，
//! 不弹，避免打扰正在看界面的用户。

use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

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

#[tauri::command]
pub fn notify(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    if !window_obscured(&app) {
        return Ok(());
    }
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
    Ok(())
}
