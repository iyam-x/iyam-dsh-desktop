use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use regex::Regex;
use tauri::Emitter;

use crate::installer::{bundled_node, dsh_home, get_install_status, InstallStatus};

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

    let bin_js = home.join("lib").join("bin.js");
    if !bin_js.exists() {
        return Err(format!("DSH 入口文件不存在: {:?}", bin_js));
    }

    // Check if already running via PID file
    let pid_file = home.join("dsh.pid");
    if pid_file.exists() {
        let pid_str = fs::read_to_string(&pid_file).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            if is_process_alive(pid) {
                let port_file = home.join("dsh.port");
                if port_file.exists() {
                    if let Ok(port_str) = fs::read_to_string(&port_file) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            let _ = app.emit("dsh-port-ready", port);
                            return Ok(port);
                        }
                        kill_process(pid);
                    }
                }
            }
        }
    }

    // Spawn DSH with DSH_HOME pointing to our home
    // 写入任务栏 AUMID 预加载脚本（幂等），使 node 子进程（目录选择对话框等）
    // 与主应用共享 AppUserModelID，任务栏按钮并入主应用，不单独显示图标。
    crate::installer::ensure_taskbar_preload(&home)?;
    // 每次启动刷新桌面壳插件（幂等），旧安装也能获得布局/通知桥更新
    if let Err(e) = crate::installer::refresh_shell_plugin(&app) {
        log::warn!("refresh shell plugin failed: {}", e);
    }

    let mut cmd = Command::new(&node);
    cmd.env("DSH_HOME", home.to_string_lossy().to_string())
       .arg(&bin_js).arg("web").arg("--port").arg("0")
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
    // Windows：GUI 应用 spawn 控制台程序（node.exe）时，默认会新建一个可见的
    // cmd 窗口。加 CREATE_NO_WINDOW 让子进程无控制台后台运行。
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("无法启动 DSH: {}", e))?;
    let pid = child.id();

    fs::write(&pid_file, pid.to_string()).ok();

    let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
    let stderr = child.stderr.take().ok_or("无法获取 stderr")?;

    let port_regex = Regex::new(r"dsh\s+web:\s+http://127\.0\.0\.1:(\d+)").unwrap();

    // Thread to read port from stdout
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

    // Drain stderr
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(l) => log::info!("[dsh] {}", l),
                Err(_) => break,
            }
        }
    });

    // Wait for port with 30s timeout
    match port_handle.join() {
        Ok(Some(port)) => {
            fs::write(home.join("dsh.port"), port.to_string()).ok();
            let _ = app.emit("dsh-port-ready", port);
            Ok(port)
        }
        _ => {
            child.kill().ok();
            fs::remove_file(&pid_file).ok();
            Err("DSH 启动超时（30s），请查看日志".to_string())
        }
    }
}

/// Stop the running DSH process
#[tauri::command]
pub async fn stop_dsh() -> Result<(), String> {
    let home = dsh_home();
    let pid_file = home.join("dsh.pid");

    if pid_file.exists() {
        let pid_str = fs::read_to_string(&pid_file).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            kill_process(pid);
        }
        fs::remove_file(pid_file).ok();
        fs::remove_file(home.join("dsh.port")).ok();
    }

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
