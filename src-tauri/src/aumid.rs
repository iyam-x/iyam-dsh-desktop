//! Windows toast 归属(AUMID)与注册表登记。
//!
//! 背景: `tauri-plugin-notification` 用 `config.identifier` 作为 toast 的 AUMID
//! 调 `ToastNotificationManager.CreateToastNotifierWithId`。非打包桌面 app 必须先把这个 AUMID
//! 登记到注册表，否则 Windows 把 toast 归属到父进程（从 powershell 启动就显示 "powershell"），
//! 且点击无响应。
//!
//! 本模块在应用启动时:
//! 1. `SetCurrentProcessExplicitAppUserModelID` 设置进程级 AUMID —— 用于任务栏分组;
//! 2. 在 `HKCU\Software\Classes\AppUserModelId\<identifier>` 登记 AUMID（必需）;
//! 3. 同时开启 `Notifications\Settings` 下的 `ToastEnabled`，避免被通知设置静默拦截。

use tauri::AppHandle;

#[cfg(windows)]
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RegCloseKey, RegCreateKeyW, RegSetValueExW,
};
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

/// 应用启动时调用（幂等；Windows 之外为 no-op）。
pub fn register(app: &AppHandle) {
    #[cfg(windows)]
    {
        let identifier = app.config().identifier.clone();
        set_process_aumid(&identifier);
        register_aumid_registry(&identifier);
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(windows)]
fn set_process_aumid(aumid: &str) {
    let wide = str_to_wide(aumid);
    unsafe { SetCurrentProcessExplicitAppUserModelID(wide.as_ptr()) };
    // wide 存活到函数结束，drop 时自动释放
    drop(wide);
}

/// 在 HKCU 注册 AUMID，使 Windows(`ToastNotificationManager.CreateToastNotifierWithId`)能正确
/// 归属 toast 到本 APP，并开启 ToastEnabled 避免被通知设置静默拦截。
/// 幂等：键已存在时直接覆盖。
#[cfg(windows)]
fn register_aumid_registry(aumid: &str) {
    // 1) AppUserModelId 键（必需：CreateToastNotifierWithId 要求 AUMID 已注册）
    reg_set_str(
        &format!("Software\\Classes\\AppUserModelId\\{aumid}"),
        None,
        aumid,
    );
    // 2) 允许该 AUMID 弹出 toast
    reg_set_dword(
        &format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings\\{aumid}"
        ),
        "ToastEnabled",
        1,
    );
}

/// 写入 REG_SZ 字符串值（默认值，无键名）
#[cfg(windows)]
fn reg_set_str(path: &str, _name: Option<&str>, value: &str) {
    let wide_path = str_to_wide(path);
    let wide_val = str_to_wide(value);
    let mut hkey: HKEY = 0;
    unsafe {
        if RegCreateKeyW(HKEY_CURRENT_USER, wide_path.as_ptr(), &mut hkey) == 0 {
            RegSetValueExW(
                hkey,
                std::ptr::null(),
                0,
                REG_SZ,
                wide_val.as_ptr() as *const u8,
                (wide_val.len() * 2) as u32,
            );
            RegCloseKey(hkey);
        }
    }
}

/// 写入 REG_DWORD 值
#[cfg(windows)]
fn reg_set_dword(path: &str, name: &str, value: u32) {
    let wide_path = str_to_wide(path);
    let wide_name = str_to_wide(name);
    let mut hkey: HKEY = 0;
    unsafe {
        if RegCreateKeyW(HKEY_CURRENT_USER, wide_path.as_ptr(), &mut hkey) == 0 {
            RegSetValueExW(
                hkey,
                wide_name.as_ptr(),
                0,
                REG_DWORD,
                &value as *const u32 as *const u8,
                std::mem::size_of::<u32>() as u32,
            );
            RegCloseKey(hkey);
        }
    }
}

/// 把 &str 转成以 NUL 结尾的 UTF-16 向量（堆分配）。
#[cfg(windows)]
fn str_to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
