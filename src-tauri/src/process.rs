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
#[cfg(windows)]
use tauri::Manager;

use crate::installer::{detect_dsh_cli, dsh_home, get_install_status, refresh_dsh_core, InstallStatus};
use crate::process_state::DSH_CHILD;

/// 探测 DSH 是否支持 `--no-open`。
///
/// 背景：rc.7 及更早版本 `web` 命令不认识 `--no-open`，commander 会立即报
/// "unknown option '--no-open'" 并退出，导致 DSH 后端起不来、界面操作全废。
/// 但"哪个版本开始支持"不可靠，故不靠版本号硬猜，而是**实际拉起一次**观察：
/// 若进程因 unknown option 立即退出 → 不支持；若 3s 内仍在运行（说明参数被接受、
/// server 已起）→ 支持。探测进程会被杀掉，不影响正式启动。
///
/// 探测结果缓存到 `<home>/.no-open-supported`（"1"/"0"），后续启动直接读取，
/// 跳过这次会 spawn 一个 node 进程、耗时约 3s 的探测，加快启动、减少闪窗。
fn dsh_supports_no_open(cli: &PathBuf, home: &PathBuf) -> bool {
    let cache = home.join(".no-open-supported");
    if let Ok(s) = fs::read_to_string(&cache) {
        if s.trim() == "1" {
            return true;
        }
        if s.trim() == "0" {
            return false;
        }
    }

    let supported = probe_no_open(cli, home);
    let _ = fs::write(&cache, if supported { "1" } else { "0" });
    supported
}

/// 真正执行一次 `--no-open` 能力探测。
/// `cli` 为真实 `bin.js` 路径，直接用托管 node 跑（跨平台一致，不依赖软链/shebang）。
fn probe_no_open(cli: &PathBuf, home: &PathBuf) -> bool {
    let node = crate::installer::managed_node(home);
    let mut cmd = Command::new(&node);
    cmd.arg(cli);
    cmd.env("DSH_HOME", home.to_string_lossy().to_string())
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

    // 确保已安装：系统已有 dsh 则仅注入插件；否则运行时下载 node+dsh 装到 ~/.dsh。
    if get_install_status(app.clone()) != InstallStatus::Installed {
        log::info!("DSH not installed yet, installing...");
        crate::installer::check_and_install(app.clone()).await?;
    }

    // 升级生效检查：若 ~/.dsh/.update.json 标记有已备货的新版本，提升到正式目录。
    // 必须在入口校验、已运行早退、插件刷新之前执行。
    if let Err(e) = refresh_dsh_core(&app) {
        log::warn!("refresh dsh core failed: {}", e);
    }

    // 启动 dsh CLI：`cli` 已是真实 `bin.js` 路径（托管态用托管 node 跑，系统态用 OS node 跑）。
    let cli = detect_dsh_cli().ok_or("未找到 dsh 命令，请检查安装或网络后重试。")?;

    // 内置插件刷新（幂等）必须在"已运行早退"之前执行，否则 DSH 已在运行时新插件永远装不上。
    // needs_dsh_restart：文件查看插件是最近新增的内置插件；刷新前它不在 DSH_HOME，
    // 说明运行中的 DSH 早于当前构建（未加载我们的插件集）→ 下方检测到已运行时会杀掉重启。
    let needs_dsh_restart = !crate::installer::dsh_node_modules(&home)
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
    // 每次启动都校准顶层 `@deepseek-ai/*` 与 core 内嵌版本一致：升级 core 后若顶层残留旧版
    // 会与新的 client-modules 等错配（如 boot manifest 缺 batches 字段）。版本一致则跳过，
    // 仅版本变更才重拷，正常启动几乎零开销。
    crate::downloader::hoist_nested_dsh_deps(&home);
    // 不在此自动预装 dshmarket，改为启动完成后弹窗让用户选择（见下方 dsh-port-ready 后）。
    // 这样无网络/不需要市场时不会拖慢首启，也不强制安装。

    // Check if already running via PID file
    let pid_file = home.join(".iyam-dsh.pid");
    if pid_file.exists() {
        let pid_str = fs::read_to_string(&pid_file).unwrap_or_default();
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            if is_process_alive(pid) {
                let port_file = home.join(".iyam-dsh.port");
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
    let no_open = dsh_supports_no_open(&cli, &home);
    log::info!("DSH --no-open supported: {}", no_open);

    // 跨平台统一：用 node 直接跑 `bin.js`，不依赖 `bin/dsh` 软链 + shebang
    // （部分镜像 tarball 的 bin.js 是坏壳子，靠软链必然崩；且托管 node 不在系统 PATH）。
    // 托管态 clI 在 ~/.dsh 下 → 用托管 node；系统态 → 用 OS `node`。
    let managed = cli.starts_with(&home);
    let mut cmd = if managed {
        let node = crate::installer::managed_node(&home);
        Command::new(&node)
    } else {
        Command::new("node")
    };
    cmd.arg(&cli);
    if let Some(node_path) = crate::installer::prepend_managed_node_path(&home) {
        cmd.env("PATH", node_path);
    }
    cmd.env("DSH_HOME", home.to_string_lossy().to_string())
       .arg("web");
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

    // 启动并等待端口；失败则自愈（自动剥离加载失败的插件）后重试，最多 3 轮。
    let max_retries = 3;
    let mut all_removed: Vec<String> = Vec::new();
    let mut attempt = 0;
    loop {
        attempt += 1;
        // 记录本次启动前的 stderr 长度，便于只解析本轮新增的错误
        let stderr_len = fs::metadata(home.join(".iyam-dsh-stderr.log"))
            .map(|m| m.len())
            .unwrap_or(0);
        match spawn_and_wait_port(&app, &mut cmd, &home, &pid_file) {
            Ok(port) => {
                if !all_removed.is_empty() {
                    let _ = app.emit("dsh-plugins-auto-disabled", all_removed.clone());
                }
                return Ok(port);
            }
            Err(_) => {
                let tail = read_stderr_tail(&home.join(".iyam-dsh-stderr.log"), stderr_len);
                let missing = parse_missing_packages(&tail);
                // 无插件可剥离，或已达重试上限：回滚升级（若适用）并报真实错误
                if missing.is_empty() || attempt >= max_retries {
                    if crate::downloader::rollback_after_failure(&home) {
                        let _ = app.emit("dsh-update-failed", ());
                    }
                    return Err(real_start_error(&tail, &all_removed));
                }
                let removed = quarantine_broken_plugins(&home, &missing);
                if removed.is_empty() {
                    if crate::downloader::rollback_after_failure(&home) {
                        let _ = app.emit("dsh-update-failed", ());
                    }
                    return Err(real_start_error(&tail, &all_removed));
                }
                log::warn!(
                    "DSH 启动失败，自动禁用 {} 个无法加载的插件: {}",
                    removed.len(),
                    removed.join(", ")
                );
                all_removed.extend(removed);
                // 继续循环重试（已写回 profiles/web/package.json）
            }
        }
    }
}

/// 启动 DSH 子进程并等待其打出端口行。
/// 成功：完成端口落盘、子进程守护、市场弹窗等全部收尾，返回端口。
/// 失败（超时/早退）：杀掉子进程并等 stderr 落盘，返回 `Err(())`。
fn spawn_and_wait_port(
    app: &tauri::AppHandle,
    cmd: &mut Command,
    home: &PathBuf,
    pid_file: &PathBuf,
) -> Result<u16, ()> {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log::error!("无法启动 DSH: {}", e);
            return Err(());
        }
    };
    fs::write(pid_file, child.id().to_string()).ok();

    // 落盘 stderr 便于排查；保留 JoinHandle 以便失败时等其刷完
    let stderr_log = home.join(".iyam-dsh-stderr.log");
    let drain = child.stderr.take().map(|stderr| {
        let log_path = stderr_log.clone();
        std::thread::spawn(move || {
            use std::io::Write;
            let reader = BufReader::new(stderr);
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
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
        })
    });

    let stdout = match child.stdout.take() {
        Some(o) => o,
        None => return Err(()),
    };
    let port_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let re = Regex::new(r"dsh\s+web:\s+http://127\.0\.0\.1:(\d+)").unwrap();
        for line in reader.lines() {
            if let Ok(l) = line {
                if let Some(cap) = re.captures(&l) {
                    if let Some(m) = cap.get(1) {
                        if let Ok(p) = m.as_str().parse::<u16>() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        None
    });

    match port_handle.join() {
        Ok(Some(port)) => {
            crate::downloader::clear_applying(home);
            fs::write(home.join(".iyam-dsh.port"), port.to_string()).ok();
            let _ = app.emit("dsh-port-ready", port);
            {
                let mut global = DSH_CHILD.lock().unwrap();
                if let Some(old) = global.as_mut() {
                    let _ = old.kill();
                }
                *global = Some(child);
            }
            if !crate::installer::dshmarket_installed(home) {
                let _ = app.emit("dshmarket-offer-install", ());
            }
            let exit_app = app.clone();
            std::thread::spawn(move || {
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
            let _ = child.kill();
            fs::remove_file(pid_file).ok();
            if let Some(h) = drain {
                let _ = h.join();
            }
            Err(())
        }
    }
}

/// 从一段 stderr 文本里解析出所有 `Cannot find package '<pkg>'` 的包名（去重、保序）。
fn parse_missing_packages(text: &str) -> Vec<String> {
    let re = Regex::new(r"Cannot find package '([^']+)'").unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cap in re.captures_iter(text) {
        let pkg = cap[1].to_string();
        if seen.insert(pkg.clone()) {
            out.push(pkg);
        }
    }
    out
}

/// 读取 stderr 日志从指定字节偏移到末尾（只取本次启动新增部分）。
fn read_stderr_tail(path: &PathBuf, from: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    match std::fs::File::open(path) {
        Ok(mut f) => {
            if f.seek(SeekFrom::Start(from)).is_err() {
                return String::new();
            }
            let mut s = String::new();
            let _ = f.read_to_string(&mut s);
            s
        }
        Err(_) => String::new(),
    }
}

/// 核心 bundle 永不自动剥离，避免把 DSH / 内置插件搞挂。
fn is_core_bundle(name: &str) -> bool {
    name.starts_with("@iyam/") || name.starts_with("@deepseek-ai/") || name == "dshmarket"
}

/// 判断某个已安装 bundle 的成员（dsh.bundles / dependencies）是否包含缺失包。
fn bundle_references_missing(
    home: &PathBuf,
    bundle: &str,
    missing: &std::collections::HashSet<&str>,
) -> bool {
    let content = match fs::read_to_string(home.join("node_modules").join(bundle).join("package.json")) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let val: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut names: Vec<String> = Vec::new();
    if let Some(arr) = val
        .get("dsh")
        .and_then(|d| d.get("bundles"))
        .and_then(|a| a.as_array())
    {
        for it in arr {
            if let Some(s) = it.as_str() {
                names.push(s.to_string());
            }
        }
    }
    if let Some(obj) = val.get("dependencies").and_then(|d| d.as_object()) {
        for k in obj.keys() {
            names.push(k.clone());
        }
    }
    if let Some(obj) = val
        .get("dsh")
        .and_then(|d| d.get("dependencies"))
        .and_then(|d| d.as_object())
    {
        for k in obj.keys() {
            names.push(k.clone());
        }
    }
    names.iter().any(|n| missing.contains(n.as_str()))
}

/// 把 `profiles/web/package.json` 中「声明了却没装进 node_modules」或「成员包缺失」的非核心
/// bundle / 依赖自动剥离（核心包除外）。返回被移除的条目名，供通知用户。
fn quarantine_broken_plugins(home: &PathBuf, missing: &[String]) -> Vec<String> {
    let profile = home.join("profiles").join("web").join("package.json");
    let content = match fs::read_to_string(&profile) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut doc: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let missing_set: std::collections::HashSet<&str> = missing.iter().map(|s| s.as_str()).collect();
    let mut removed: Vec<String> = Vec::new();
    for field in ["bundles", "dependencies"] {
        let arr = match doc
            .get_mut("dsh")
            .and_then(|d| d.get_mut("profile"))
            .and_then(|p| p.get_mut(field))
            .and_then(|a| a.as_array_mut())
        {
            Some(a) => a,
            None => continue,
        };
        let mut to_remove = Vec::new();
        for (i, item) in arr.iter().enumerate() {
            if let Some(name) = item.as_str() {
                if is_core_bundle(name) {
                    continue;
                }
                let installed = home.join("node_modules").join(name).exists();
                let strip = !installed
                    || missing_set.contains(name)
                    || bundle_references_missing(home, name, &missing_set);
                if strip {
                    to_remove.push(i);
                }
            }
        }
        for i in to_remove.into_iter().rev() {
            let v = arr.remove(i);
            if let Some(s) = v.as_str() {
                removed.push(s.to_string());
            }
        }
    }
    if !removed.is_empty() {
        if let Ok(s) = serde_json::to_string_pretty(&doc) {
            let _ = fs::write(&profile, s);
        }
    }
    removed
}

/// 启动失败时构造给用户看的真实错误（尽量透出缺失的插件，而非笼统超时）。
fn real_start_error(tail: &str, all_removed: &[String]) -> String {
    if let Some(cap) = Regex::new(r"Cannot find package '([^']+)'")
        .unwrap()
        .captures(tail)
    {
        let pkg = &cap[1];
        if all_removed.is_empty() {
            return format!(
                "DSH 启动失败：插件 {} 未能加载（ERR_MODULE_NOT_FOUND），请检查插件市场安装后重试，或查看日志",
                pkg
            );
        }
        return format!(
            "DSH 启动失败：已自动禁用 {} 个损坏插件（{}），但 {} 仍无法加载，请查看日志",
            all_removed.len(),
            all_removed.join("、"),
            pkg
        );
    }
    if !all_removed.is_empty() {
        return format!(
            "DSH 启动失败：已自动禁用 {} 个损坏插件（{}），但 DSH 仍无法启动，请查看日志",
            all_removed.len(),
            all_removed.join("、")
        );
    }
    "DSH 启动超时（30s），请查看日志".to_string()
}

/// Stop the running DSH process
#[tauri::command]
pub async fn stop_dsh() -> Result<(), String> {
    crate::process_state::kill_dsh_on_exit();
    let home = dsh_home();
    fs::remove_file(home.join(".iyam-dsh.pid")).ok();
    fs::remove_file(home.join(".iyam-dsh.port")).ok();
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
