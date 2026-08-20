// GUI 应用：release 标记为 windows subsystem，不再分配控制台（避免启动时弹
// 出 Windows Terminal/cmd 窗口）。debug 保留 console，便于 `tauri dev` 看日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, RunEvent, WindowEvent};
use tauri::Emitter;

mod aumid;
mod installer;
mod notify;
mod process;
mod process_state;
mod updater;
mod window;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Windows：注册 AUMID（注册表登记 + 进程级 AUMID），toast 归属与任务栏分组必须。
            aumid::register(app.handle());
            create_tray(&app.app_handle())?;
            Ok(())
        })
        // 关闭主窗口时最小化到托盘（不退出，DSH 后台继续跑；托盘「退出」才是真退出）
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            installer::get_install_status,
            installer::check_and_install,
            process::start_dsh,
            process::stop_dsh,
            updater::check_for_update,
            window::show_system_menu,
            tray_commands::restart_dsh,
            notify::notify,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // 应用真正退出前清理 DSH 子进程，防止 node 残留
    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { .. } = event {
            process_state::kill_dsh_on_exit();
            let home = installer::dsh_home();
            let _ = std::fs::remove_file(home.join("dsh.pid"));
            let _ = std::fs::remove_file(home.join("dsh.port"));
            // 通知前端应用正在退出（可选）
            let _ = app_handle.emit("dsh-app-exiting", ());
        }
    });
}

/// 托盘相关命令实现
mod tray_commands {
    use super::process_state::kill_dsh_on_exit;
    use crate::process::start_dsh;

    /// 托盘「重启 DSH」：先停后启
    #[tauri::command]
    pub async fn restart_dsh(app: tauri::AppHandle) -> Result<(), String> {
        kill_dsh_on_exit();
        let home = crate::installer::dsh_home();
        std::fs::remove_file(home.join("dsh.pid")).ok();
        std::fs::remove_file(home.join("dsh.port")).ok();
        start_dsh(app).await?;
        Ok(())
    }
}

/// 创建系统托盘
fn create_tray(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::{
        image::Image,
        menu::{Menu, MenuItem, PredefinedMenuItem},
        tray::TrayIconBuilder,
    };

    // 关键：必须用 with_id 设置菜单项 id（MenuItem::new 的第 4 参数是快捷键而非 id，
    // 会导致 id 被自动编号，on_menu_event 里按字符串 id match 永远命中不了）。
    let open_item = MenuItem::with_id(app, "open_dsh", "打开 DSH", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let restart_item =
        MenuItem::with_id(app, "restart_dsh", "重启 DSH", true, None::<&str>)
            .map_err(|e| e.to_string())?;
    let separator = PredefinedMenuItem::separator(app)
        .map_err(|e| e.to_string())?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)
        .map_err(|e| e.to_string())?;

    let menu = Menu::with_items(
        app,
        &[&open_item, &restart_item, &separator, &quit_item],
    )
    .map_err(|e| e.to_string())?;

    // 使用 bundled icon.png 作为托盘图标（与 tauri.conf.json bundle.resources 声明一致）
    let icon_path = app
        .path()
        .resource_dir()
        .map(|r| r.join("icons/icon.png"))
        .ok();

    let tray_builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("iyam-dsh")
        // 菜单事件分发：菜单项 id 在 build 时绑定，点击后按 id 路由到对应动作。
        // 此前缺失此回调 + 菜单项 id 为 None，导致「重启 DSH」「退出」点击无任何反应。
        .on_menu_event(|app, event| {
            let id: &str = event.id().as_ref();
            match id {
                "open_dsh" => {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.unminimize();
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
                "restart_dsh" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = tray_commands::restart_dsh(app).await;
                    });
                }
                "quit" => {
                    // 必须在主线程调用 app.exit() 才能可靠终止事件循环。
                    // 此前放在 async_runtime 工作线程里调用，exit 信号未被主线程
                    // 事件循环处理，表现为点击「退出」无反应（而「打开 DSH」在主线程
                    // 同步执行，正常）。DSH 清理由 app.run 的 ExitRequested 处理器完成。
                    // 这里先显式杀 DSH（守护线程独占 child 句柄后，exit 回调里的清理
                    // 拿不到句柄，必须靠 dsh.pid 兜底杀 node），再退出。
                    process_state::kill_dsh_on_exit();
                    app.clone().exit(0);
                }
                _ => {}
            }
        });

    let _tray = if let Some(path) = icon_path {
        if path.exists() {
            let img = Image::from_path(&path).map_err(|e| e.to_string())?;
            tray_builder.icon(img).build(app).map_err(|e| e.to_string())?
        } else {
            // 兜底：用默认窗口图标
            let icon = app.default_window_icon().cloned().ok_or("no default icon")?;
            tray_builder.icon(icon).build(app).map_err(|e| e.to_string())?
        }
    } else {
        let icon = app.default_window_icon().cloned().ok_or("no default icon")?;
        tray_builder.icon(icon).build(app).map_err(|e| e.to_string())?
    };

    Ok(())
}
