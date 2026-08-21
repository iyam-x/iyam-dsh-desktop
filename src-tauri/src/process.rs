use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use regex::Regex;
use tauri::Emitter;
use tauri::Manager;

use crate::installer::{bundled_node, dsh_home, get_install_status, refresh_dsh_core, InstallStatus};
use crate::process_state::DSH_CHILD;

/// 探测当前 bundle 的 DSH 是否支持 `--no-open`。
///
/// 背景：rc.7 及更早版本 `web` 命令不认识 `--no-open`，commander 会立即报
/// "unknown option '--no-open'" 并退出，导致 DSH 后端起不来、界面操作全废。
/// 但"哪个版本开始支持"不可靠（曾误以为 rc.8 就支持，实际并非如此），故不靠
/// 版本号硬猜，而是**实际拉起一次**观察：若进程因 unknown option 立即退出 → 不支持；
/// 若 3s 内仍在运行（说明参数被接受、server 已起）→ 支持。探测进程会被杀掉，不影响正式启动。
fn dsh_supports_no_open(node: &PathBuf, bin_js: &PathBuf, home: &PathBuf) -> bool {
    let mut cmd = Command::new(node);
    cmd.env("DSH_HOME", home.to_string_lossy().to_string())
        .arg(bin_js)
        .arg("web")
        .arg("--no-open")
        .arg("--port")
        .arg("0")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                // 进程已退出：读取 stderr 判断是否因 unknown option 退出
                if let Some(mut s) = child.stderr.take() {
                    let mut content = String::new();
                    let _ = s.read_to_string(&mut content);
                    if content.contains("unknown option") && content.contains("no-open") {
                        return false;
                    }
                }
                return true;
            }
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    return true; // 3s 仍在运行 → 参数被接受
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return false,
        }
    }
}

/// Start the DSH web server process and return the port.
/// 直接 spawn bundle 内的 node 运行 lib/bin.js，不依赖系统 node / 系统 dsh。
#[tauri::command]
pub async fn start_dsh(app: tauri::AppHandle) -> Result<u16, String> {
    log::info!("start_dsh called");
    let home = dsh_home();
    log::info!("DSH_HOME: {:?}", home);

    let node = bundled_node(&app)
        .ok_or("内置 Node 运行时未找到，请重新安装应用。")?;
    log::info!("bundled node: {:?}", node);

    // 确保已安装（未安装时才拷贝，避免每次启动重复拷贝）
    if get_install_status(app.clone()).await != InstallStatus::Installed {
        log::info!("DSH not installed yet, installing...");
        crate::installer::check_and_install(app.clone()).await?;
    }

    // 内置 DSH 核心版本与 DSH_HOME 部署版本不一致时重新部署（bundle 升级后旧核心
    // 会残留，导致新资源不生效）。必须在入口校验、已运行早退、插件刷新之前执行。
    if let Err(e) = refresh_dsh_core(&app) {
        log::warn!("refresh dsh core failed: {}", e);
    }

    let bin_js = home.join("lib").join("bin.js");
    if !bin_js.exists() {
        return Err(format!("DSH 入口文件不存在: {:?}", bin_js));
    }

    // 内置插件刷新（幂等）必须在"已运行早退"之前执行，否则 DSH 已在运行时新插件永远装不上。
    // needs_dsh_restart：文件查看插件是最近新增的内置插件；刷新前它不在 DSH_HOME，
    // 说明运行中的 DSH 早于当前构建（未加载我们的插件集）→ 下方检测到已运行时会杀掉重启。
    let needs_dsh_restart = !home
        .join("node_modules")
        .join("@iyam")
        .join("dsh-file-handler")
        .join("client.js")
        .exists();
    if let Err(e) = crate::installer::refresh_shell_plugin(&app) {
        log::warn!("refresh shell plugin failed: {}", e);
    }
    if let Err(e) = crate::installer::refresh_rtui_ui_plugin(&app) {
        log::warn!("refresh rtui-ui plugin failed: {}", e);
    }
    if let Err(e) = crate::installer::refresh_file_handler_plugin(&app) {
        log::warn!("refresh file-handler plugin failed: {}", e);
    }

    // Check if already running via PID file
    let pid_file = home.join("dsh.pid");
    if pid_file.exists() {
        let pid_str = fs::read_to_string(&pid_file).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            if is_process_alive(pid) {
                let port_file = home.join("dsh.port");
                if !needs_dsh_restart && port_file.exists() {
                    if let Ok(port_str) = fs::read_to_string(&port_file) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            let _ = app.emit("dsh-port-ready", port);
                            return Ok(port);
                        }
                    }
                }
                // 端口不可用，或插件集过期（needs_dsh_restart）→ 杀掉旧进程，走下方全新 spawn
                kill_process(pid);
            }
        }
    }

    // Spawn DSH with DSH_HOME pointing to our home
    // 写入任务栏 AUMID 预加载脚本（幂等），使 node 子进程（目录选择对话框等）
    // 与主应用共享 AppUserModelID，任务栏按钮并入主应用，不单独显示图标。
    crate::installer::ensure_taskbar_preload(&home)?;
    // 为目录选择器 worker 打 owner 补丁（幂等），使对话框归入主窗口任务栏按钮
    crate::installer::ensure_picker_owner_patch(&home);

    // 运行时探测 DSH 是否支持 `--no-open`（旧版本不支持会直接退出、后端起不来）。
    // 支持才传，避免弹系统浏览器；不支持则不传，保证 DSH 一定能启动。
    let no_open = dsh_supports_no_open(&node, &bin_js, &home);
    log::info!("DSH --no-open supported: {}", no_open);

    let mut cmd = Command::new(&node);
    cmd.env("DSH_HOME", home.to_string_lossy().to_string())
       .arg(&bin_js).arg("web");
    if no_open {
        cmd.arg("--no-open");
    }
    cmd.arg("--port").arg("0")
       .stdout(Stdio::piped())
       .stderr(Stdio::piped())
       .stdin(Stdio::null());
    // 注入 AUMID 预加载脚本（仅 Windows 有效；文件缺失时跳过，避免启动失败）
    #[cfg(windows)]
    {
        let preload = home.join("set-taskbar-aumid.cjs");
        if preload.exists() {
            let path = preload.to_string_lossy().replace('\\', "/");
            cmd.env("NODE_OPTIONS", format!("--require=\"{}\"", path));
        }
    }
    // 注入对话框 owner HWND：native 目录选择器 worker 以此为 IFileOpenDialog 的 owner，
    // 对话框成为主窗口的 owned window → 不占独立任务栏按钮、图标继承应用。
    // 仅注入有效的顶层窗口句柄；无效则不注入（worker 回退 Show(null)，目录选择器照常可用）。
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
        if let Some(win) = app.get_webview_window("main") {
            if let Ok(hwnd) = win.hwnd() {
                // hwnd.0: *mut c_void；windows-sys 的 HWND = isize
                let val = hwnd.0 as isize;
                let valid = unsafe { IsWindow(val as _) } != 0;
                log::info!("owner HWND: {val} (is_window={valid})");
                if valid {
                    cmd.env("DSH_DIALOG_OWNER_HWND", val.to_string());
                }
            }
        }
    }
    // Windows：GUI 应用 spawn 控制台程序（node.exe）时，默认会新建一个可见的
    // cmd 窗口。加 CREATE_NO_WINDOW 让子进程无控制台后台运行。
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("无法启动 DSH: {}", e))?;
    let pid = child.id();

    fs::write(&pid_file, pid.to_string()).ok();

    // 先取出 stdout/stderr pipe
    let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
    let stderr = child.stderr.take().ok_or("无法获取 stderr")?;

    let port_regex = Regex::new(r"dsh\s+web:\s+http://127\.0\.0\.1:(\d+)").unwrap();

    // Thread to read port from stdout（只传 stdout pipe，不传 child）
    let port_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if let Some(cap) = port_regex.captures(&l) {
                        if let Some(port_match) = cap.get(1) {
                            if let Ok(port) = port_match.as_str().parse::<u16>() {
                                return Some(port);
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
        None
    });

    // Drain stderr（只传 stderr pipe），并落盘便于排查 DSH 行为
    let stderr_log = home.join("dsh-stderr.log");
    std::thread::spawn(move || {
        use std::io::Write;
        let reader = BufReader::new(stderr);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_log)
            .ok();
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    log::info!("[dsh] {}", l);
                    if let Some(f) = file.as_mut() {
                        let _ = writeln!(f, "{}", l);
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Wait for port with 30s timeout
    match port_handle.join() {
        Ok(Some(port)) => {
            fs::write(home.join("dsh.port"), port.to_string()).ok();
            let _ = app.emit("dsh-port-ready", port);

            // 成功后才将 child 存入全局静态，并启动守护线程监听进程退出
            {
                let mut global = DSH_CHILD.lock().unwrap();
                if let Some(old) = global.as_mut() {
                    let _ = old.kill();
                }
                *global = Some(child);
            }
            let exit_app = app.clone();
            std::thread::spawn(move || {
                // 先取出 child 并立即释放锁，再 wait，避免持有锁期间阻塞导致死锁
                let taken = if let Ok(mut locked) = DSH_CHILD.lock() {
                    locked.take()
                } else {
                    None
                };
                if let Some(mut c) = taken {
                    let _ = c.wait();
                    let _ = exit_app.emit("dsh-process-exit", ());
                }
            });

            Ok(port)
        }
        _ => {
            // 超时或端口读取失败：child 仍在作用域内，直接 kill
            child.kill().ok();
            fs::remove_file(&pid_file).ok();
            Err("DSH 启动超时（30s），请查看日志".to_string())
        }
    }
}

/// Stop the running DSH process
#[tauri::command]
pub async fn stop_dsh() -> Result<(), String> {
    crate::process_state::kill_dsh_on_exit();
    let home = dsh_home();
    fs::remove_file(home.join("dsh.pid")).ok();
    fs::remove_file(home.join("dsh.port")).ok();
    Ok(())
}

fn kill_process(pid: u32) {
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
            std::thread::sleep(std::time::Duration::from_secs(3));
            if is_process_alive(pid) {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        let mut k = Command::new("taskkill");
        k.args(["/F", "/PID", &pid.to_string()]);
        k.creation_flags(CREATE_NO_WINDOW);
        k.output().ok();
    }
}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        let mut tl = Command::new("tasklist");
        tl.args(["/FI", &format!("PID eq {}", pid), "/NH"]);
        tl.creation_flags(CREATE_NO_WINDOW);
        tl.output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}
