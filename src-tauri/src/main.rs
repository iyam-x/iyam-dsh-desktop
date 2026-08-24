// GUI 应用：release 标记为 windows subsystem，不再分配控制台（避免启动时弹
// 出 Windows Terminal/cmd 窗口）。debug 保留 console，便于 `tauri dev` 看日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, RunEvent, WindowEvent};
use tauri::Emitter;

mod aumid;
mod downloader;
mod file_preview;
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
            // 主窗口改为 setup 中手动构建：这样才有机会挂 on_navigation / on_download
            // 两层原生拦截，堵住 macOS WKWebView 把「无法渲染的内容 / 导航 / 下载」
            // 交给 LaunchServices 弹系统 "Choose Application" 对话框的通道。
            // 窗口参数与原先 tauri.conf.json 配置完全一致（Windows 无边框、macOS 透明 Overlay）。
            {
                use tauri::{WebviewUrl, WebviewWindowBuilder};
                #[cfg(target_os = "macos")]
                use tauri::TitleBarStyle;

                let mut builder = WebviewWindowBuilder::new(
                    app,
                    "main",
                    WebviewUrl::App("index.html".into()),
                )
                .title("iyam-dsh")
                .inner_size(1280.0, 800.0)
                .resizable(true);

                #[cfg(target_os = "macos")]
                {
                    builder = builder
                        .decorations(true)
                        .transparent(true)
                        .title_bar_style(TitleBarStyle::Overlay)
                        .hidden_title(true);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    builder = builder.decorations(false);
                }

                builder
                    .on_navigation(|url| {
                        let scheme = url.scheme();
                        match scheme {
                            // 生产环境壳页面 tauri://localhost；Tauri IPC 通道
                            "tauri" | "ipc" => true,
                            // 开发环境壳页面 localhost:1420；DSH 服务 127.0.0.1
                            "http" | "https" => {
                                let host = url.host_str().unwrap_or("");
                                host == "localhost"
                                    || host == "127.0.0.1"
                                    || host == "::1"
                                    || host == "tauri.localhost"
                            }
                            "about" => url.path() == "blank",
                            // file://、自定义 scheme、外部站点一律拦截，
                            // 避免 WebKit 把内容交给 LaunchServices 弹系统对话框。
                            _ => false,
                        }
                    })
                    .on_download(|_webview, event| {
                        use tauri::webview::DownloadEvent;
                        match event {
                            DownloadEvent::Requested { url, destination } => {
                                // 有下载 handler 后，wry 对「无法渲染的 MIME 响应」会改走下载
                                // 而非 Allow → 不再弹 "Choose Application"。保存到系统下载目录。
                                log::info!(
                                    "[webview-download] {} → {}",
                                    url,
                                    destination.display()
                                );
                            }
                            DownloadEvent::Finished { url, path, success } => {
                                log::info!(
                                    "[webview-download] finished {} success={} path={:?}",
                                    url,
                                    success,
                                    path
                                );
                            }
                            // DownloadEvent 标记为非穷尽枚举，保留通配分支
                            _ => {}
                        }
                        true
                    })
                    .build()?;
            }

            // macOS：替换默认菜单栏（默认菜单含 Reload/DevTools/缩放/帮助等无用项），
            // 只保留应用/编辑/窗口三个子菜单。Windows/Linux 无系统菜单栏，不设置。
            #[cfg(target_os = "macos")]
            {
                let menu = build_app_menu(app.handle())?;
                app.set_menu(menu)?;
            }
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
            updater::trigger_dsh_update,
            window::show_system_menu,
            window::open_devtools,
            tray_commands::restart_dsh,
            notify::notify,
            file_preview::read_text_file,
            file_preview::read_file_data,
            file_preview::write_text_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // 应用真正退出前清理 DSH 子进程，防止 node 残留
    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { .. } = event {
            process_state::kill_dsh_on_exit();
            let home = installer::dsh_home();
            let _ = std::fs::remove_file(home.join(".iyam-dsh.pid"));
            let _ = std::fs::remove_file(home.join(".iyam-dsh.port"));
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
        std::fs::remove_file(home.join(".iyam-dsh.pid")).ok();
        std::fs::remove_file(home.join(".iyam-dsh.port")).ok();
        start_dsh(app).await?;
        Ok(())
    }
}

/// 精简 macOS 应用菜单：替换默认菜单（含 Reload / Force Reload / Toggle DevTools /
/// 缩放 / 全屏 / 帮助等对本应用无用的项）。只保留三个子菜单：
///  - 应用：关于 / 隐藏 / 隐藏其他 / 全部显示 / 退出
///  - 编辑：撤销 / 重做 / 剪切 / 复制 / 粘贴 / 全选（webview 输入框的 Cmd 快捷键依赖它）
///  - 窗口：最小化 / 缩放
#[cfg(target_os = "macos")]
fn build_app_menu(app: &tauri::AppHandle) -> Result<tauri::menu::Menu<tauri::Wry>, tauri::Error> {
    use tauri::menu::{Menu, PredefinedMenuItem, Submenu};

    let app_sub = Submenu::with_items(
        app,
        "iyam-dsh",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("关于 iyam-dsh"), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("隐藏 iyam-dsh"))?,
            &PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?,
            &PredefinedMenuItem::show_all(app, Some("全部显示"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("退出 iyam-dsh"))?,
        ],
    )?;
    let edit_sub = Submenu::with_items(
        app,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("撤销"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("剪切"))?,
            &PredefinedMenuItem::copy(app, Some("复制"))?,
            &PredefinedMenuItem::paste(app, Some("粘贴"))?,
            &PredefinedMenuItem::select_all(app, Some("全选"))?,
        ],
    )?;
    let window_sub = Submenu::with_items(
        app,
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("最小化"))?,
            &PredefinedMenuItem::maximize(app, Some("最大化"))?,
        ],
    )?;
    Menu::with_items(app, &[&app_sub, &edit_sub, &window_sub])
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
    let separator = PredefinedMenuItem::separator(app)
        .map_err(|e| e.to_string())?;
    let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)
        .map_err(|e| e.to_string())?;

    let menu = Menu::with_items(
        app,
        &[&open_item, &separator, &quit_item],
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
