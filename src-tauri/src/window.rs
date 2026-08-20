// ── 自定义标题栏辅助命令 ──

// Windows：右键标题栏弹出原生系统菜单（Restore / Move / Size / Minimize /
// Maximize / Close），行为与系统原生标题栏完全一致。
#[cfg(target_os = "windows")]
#[tauri::command]
pub fn show_system_menu(window: tauri::Window) -> Result<(), String> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetSystemMenu, PostMessageW, SendMessageW, SetForegroundWindow,
        TrackPopupMenuEx, WM_NULL, WM_SYSCOMMAND, TPM_LEFTALIGN, TPM_TOPALIGN, TPM_RETURNCMD,
    };

    let hwnd: windows_sys::Win32::Foundation::HWND =
        window.hwnd().map_err(|e| e.to_string())?.0 as _;

    unsafe {
        let hmenu = GetSystemMenu(hwnd, 0);
        if hmenu == 0 {
            return Err("GetSystemMenu returned null".into());
        }
        // Bring the window to the foreground so the system menu can receive input.
        SetForegroundWindow(hwnd);
        let mut pt: POINT = std::mem::zeroed();
        GetCursorPos(&mut pt);
        let cmd = TrackPopupMenuEx(
            hmenu,
            TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD,
            pt.x,
            pt.y,
            hwnd,
            std::ptr::null(),
        );
        // Release the mouse capture so the app keeps receiving input afterwards.
        PostMessageW(hwnd, WM_NULL, 0, 0);
        if cmd != 0 {
            SendMessageW(hwnd, WM_SYSCOMMAND, cmd as usize, 0);
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn show_system_menu(_window: tauri::Window) -> Result<(), String> {
    Err("Native system menu is only available on Windows".into())
}

/// 打开主窗口的开发者工具（调试用）。release 构建需启用 tauri `devtools` 特性。
#[tauri::command]
pub fn open_devtools(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if let Some(win) = app.get_webview_window("main") {
        win.open_devtools();
    }
    Ok(())
}
