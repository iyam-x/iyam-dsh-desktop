// GUI 应用：release 标记为 windows subsystem，不再分配控制台（避免启动时弹
// 出 Windows Terminal/cmd 窗口）。debug 保留 console，便于 `tauri dev` 看日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::generate_handler;
mod installer;
mod process;
mod updater;
mod window;

fn main() {
    // Windows：与 DSH 子进程（目录/文件选择对话框）共享 AppUserModelID，
    // 任务栏按钮归并为同一个，避免单独弹出 node 图标。
    // 注意：tauri/tao/wry 都不会自动设置 AUMID，默认按 exe 路径分组。
    #[cfg(target_os = "windows")]
    unsafe {
        windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
            windows_sys::core::w!("ai.iyam.dsh"),
        );
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(generate_handler![
            installer::get_install_status,
            installer::check_and_install,
            process::start_dsh,
            process::stop_dsh,
            updater::check_for_update,
            window::show_system_menu,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
