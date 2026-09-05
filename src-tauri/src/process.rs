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

/// Start the DSH web server process and return the authenticated web URL
/// （含 launch token；旧版 dsh 为裸 URL）。iframe 必须加载该 URL 才能通过认证。
/// 直接 spawn bundle 内的 node 运行 lib/bin.js，不依赖系统 node / 系统 dsh。
#[tauri::command]
pub async fn start_dsh(app: tauri::AppHandle) -> Result<String, String> {
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

    // dsh 版本已变化（刚升级）时，把上次因不兼容被禁用的第三方插件加回来重试：
    // 新版 dsh 下插件可能已适配；若仍不兼容，下方启动重试会再次隔离它。
    reinstate_quarantined(&home, false);

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
                if !needs_dsh_restart && !core_version_mismatch(&home) && port_file.exists() {
                    if let Ok(port_str) = fs::read_to_string(&port_file) {
                        if let Ok(port) = port_str.trim().parse::<u16>() {
                            let url = stored_web_url(&home, port);
                            let _ = app.emit("dsh-port-ready", url.clone());
                            return Ok(url);
                        }
                    }
                }
                // 端口不可用、插件集过期，或磁盘核心已升级而运行中的还是旧进程
                //（promote 后的内存/磁盘错配）→ 杀掉旧进程，走下方全新 spawn
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

    // 启动并等待端口；失败则自愈（自动剥离不兼容/损坏的插件）后重试，最多 3 轮。
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
            Ok(url) => {
                if !all_removed.is_empty() {
                    let _ = app.emit("dsh-plugins-auto-disabled", all_removed.clone());
                }
                return Ok(url);
            }
            Err(_) => {
                let tail = read_stderr_tail(&home.join(".iyam-dsh-stderr.log"), stderr_len);
                let missing = parse_missing_packages(&tail);
                let named = parse_broken_plugins(&tail);
                // stderr 点得出具体插件（链接错误 / 缺包）→ 只剥肇事者，重试。
                if attempt < max_retries && (!missing.is_empty() || !named.is_empty()) {
                    let removed = quarantine_broken_plugins(&home, &missing, &named);
                    if !removed.is_empty() {
                        log::warn!(
                            "DSH 启动失败，自动禁用 {} 个不兼容插件: {}",
                            removed.len(),
                            removed.join(", ")
                        );
                        all_removed.extend(removed);
                        continue;
                    }
                }
                // 点不出具体插件（导出移除等疑难报错）→ 最后一搏：剥离全部第三方插件，
                // 保证 DSH 一定能起来（app 可用优先；被禁插件记录在案、待适配后自动恢复）。
                if attempt < max_retries && missing.is_empty() && named.is_empty() {
                    let removed = quarantine_all_third_party(&home);
                    if !removed.is_empty() {
                        log::warn!(
                            "无法定位故障插件，已禁用全部第三方插件重试: {}",
                            removed.join(", ")
                        );
                        all_removed.extend(removed);
                        continue;
                    }
                }
                // 无计可施：回滚本次升级（若适用），并自动用回滚后的版本重启一次，
                // 保证 app 立刻恢复可用（回滚会把状态置为 failed，递归最多一层、不会死循环）。
                if crate::downloader::rollback_after_failure(&home) {
                    let _ = app.emit("dsh-update-failed", ());
                    log::warn!("已回滚升级，自动以回滚后的版本重启 DSH");
                    // 递归 async fn 需 Box::pin 引入间接层（回滚状态机保证只递归一层）。
                    return Box::pin(start_dsh(app)).await;
                }
                return Err(real_start_error(&tail, &all_removed));
            }
        }
    }
}

/// 从 dsh stdout 行解析 web 访问 URL（**含 launch token**）。
///
/// dsh 0.1.2-rc.1 起 web 服务带认证：stdout 打印
/// `dsh web: http://127.0.0.1:<port>/?token=<t>`，首次带 token 访问 `/` 会换取签名
/// cookie，之后凭 cookie 加载界面；裸 URL 一律 401（"authentication required"）。
/// 旧版本打印的是无 token 裸 URL。统一取 `dsh web:` 后第一个空白分隔的 127.0.0.1
/// URL（LAN URL 在括号内，不会被匹配）。
fn parse_web_url(line: &str) -> Option<String> {
    let re = Regex::new(r"dsh\s+web:\s+(http://127\.0\.0\.1:\d+\S*)").unwrap();
    re.captures(line).map(|c| c[1].to_string())
}

/// 从 web URL 提取端口（写 `.iyam-dsh.port` 兼容既有流程）。
fn port_of_url(url: &str) -> Option<u16> {
    let re = Regex::new(r"http://127\.0\.0\.1:(\d+)").unwrap();
    re.captures(url)?.get(1)?.as_str().parse::<u16>().ok()
}

/// 运行中 dsh（spawn 时记录在 `.iyam-dsh.version`）与当前磁盘核心版本是否不一致。
///
/// 升级提升发生在启动早期：若旧进程还活着（窗口关闭进托盘时 DSH 常驻后台），
/// promote 后磁盘已是新版、旧进程仍以旧代码服务界面——旧格式 boot manifest
/// （无 `batches`）与新版前端校验混搭，正是 "client-modules: boot manifest batches
/// must be an array" 的来源。此时必须杀掉旧进程走全新 spawn，让内存与磁盘对齐。
/// 版本记录缺失（升级前遗留的旧进程 / 旧版安装）同样强制重启一次。
fn core_version_mismatch(home: &PathBuf) -> bool {
    let recorded = fs::read_to_string(home.join(".iyam-dsh.version"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match dsh_core_version(home) {
        Some(current) => recorded.as_deref() != Some(current.as_str()),
        // 磁盘上读不到核心版本（残损安装），不在此处理，走后续正常启动流程。
        None => false,
    }
}

/// 读取已运行 dsh 的访问 URL（spawn 时落盘的带 token 地址）；缺失则退回裸 URL。
fn stored_web_url(home: &PathBuf, port: u16) -> String {
    fs::read_to_string(home.join(".iyam-dsh.url"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", port))
}

/// 启动 DSH 子进程并等待其打出端口行。
/// 成功：完成 URL/port/版本落盘、子进程守护、市场弹窗等全部收尾，返回带 token 的访问 URL。
/// 失败（超时/早退）：杀掉子进程并等 stderr 落盘，返回 `Err(())`。
fn spawn_and_wait_port(
    app: &tauri::AppHandle,
    cmd: &mut Command,
    home: &PathBuf,
    pid_file: &PathBuf,
) -> Result<String, ()> {
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
    let url_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(l) = line {
                if let Some(url) = parse_web_url(&l) {
                    return Some(url);
                }
            }
        }
        None
    });

    match url_handle.join() {
        Ok(Some(url)) => {
            let port = port_of_url(&url);
            crate::downloader::clear_applying(home);
            if let Some(p) = port {
                fs::write(home.join(".iyam-dsh.port"), p.to_string()).ok();
            }
            fs::write(home.join(".iyam-dsh.url"), &url).ok();
            // 记录本次 spawn 的核心版本：下次启动用于检测「磁盘已升级而进程还是旧版」
            // 的错配，决定是否强制重启（见 core_version_mismatch）。
            let core_ver = dsh_core_version(home).unwrap_or_default();
            fs::write(home.join(".iyam-dsh.version"), core_ver).ok();
            let _ = app.emit("dsh-port-ready", url.clone());
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
            Ok(url)
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

/// 从一段 stderr 文本里解析出「插件树加载失败」的肇事第三方插件（去重、保序）。
///
/// 背景：dsh 升级常移除内部导出（如 0.1.2-rc.1 移除 `installSettingsSection`），
/// 第三方插件的静态 import 在 ESM 链接期即失败，整棵 dsh 启动崩溃。这类错误
/// 不是 `Cannot find package`，需要单独识别。两种线索：
/// 1. dsh-app-boot 的汇总行：`failed to import loader entry <id> (<pkg>): ...`
///    （如 `failed to import loader entry dsh-market (dshmarket): ...`）；
/// 2. 栈里的文件路径（ESM 一律 file:// URL）：`.../node_modules/[.pnpm/<x>/node_modules/]<pkg>/...`，
///    过滤 `@deepseek-ai/*`、`@iyam/*`、`node:` 等核心/内部模块后即为第三方插件。
fn parse_broken_plugins(text: &str) -> Vec<String> {
    let entry_re = Regex::new(r"failed to import loader entry \S+ \(([^)]+)\)").unwrap();
    // node_modules 可能有一层 .pnpm/<pkg>@<ver>_.../node_modules/ 中间段，取最后一段的包名。
    let path_re = Regex::new(
        r"node_modules/(?:\.pnpm/[^/\s:)]+/node_modules/)?((?:@[^/:\s)]+/)?[^/:\s)]+)",
    )
    .unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cap in entry_re.captures_iter(text) {
        let pkg = cap[1].to_string();
        if pkg.starts_with("node:") || is_core_bundle(&pkg) {
            continue;
        }
        if seen.insert(pkg.clone()) {
            out.push(pkg);
        }
    }
    for cap in path_re.captures_iter(text) {
        let pkg = cap[1].to_string();
        if pkg.starts_with("node:") || is_core_bundle(&pkg) {
            continue;
        }
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
/// 注意：dshmarket 等市场安装的第三方插件**不在保护之列**——它们恰是 dsh 升级后
/// 最常见的启动崩溃源（如 0.1.2-rc.1 移除 `installSettingsSection` 后 dshmarket 1.20.0
/// 链接失败拖垮整棵 DSH），必须允许被隔离。
fn is_core_bundle(name: &str) -> bool {
    name.starts_with("@iyam/") || name.starts_with("@deepseek-ai/")
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

/// 把 `profiles/web/package.json` 中确认损坏的非核心 bundle / 依赖自动剥离（核心包除外）。
/// 判定来源（满足其一即剥离）：
/// - `named`：stderr 里被点名的肇事插件（链接错误 / 栈路径解析，见 `parse_broken_plugins`）；
/// - 声明了却没装进 node_modules、或其成员包缺失（`missing` 推断，ERR_MODULE_NOT_FOUND 类）。
/// 返回被移除的条目名，供通知用户。
fn quarantine_broken_plugins(home: &PathBuf, missing: &[String], named: &[String]) -> Vec<String> {
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
    let named_set: std::collections::HashSet<&str> = named.iter().map(|s| s.as_str()).collect();
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
                // 安装位置有两处：顶层依赖树（unix 为 lib/node_modules）与 profile 目录
                // （`dsh plugin add` 装到 profiles/web/node_modules）。任一存在即算已装。
                let installed = crate::installer::dsh_node_modules(home).join(name).exists()
                    || home
                        .join("profiles")
                        .join("web")
                        .join("node_modules")
                        .join(name)
                        .exists();
                let strip = named_set.contains(name)
                    || !installed
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
        record_quarantine(home, &removed);
    }
    removed
}

/// 兜底隔离：剥离 profile 里**全部**非核心第三方 bundle / 依赖（保留 `@deepseek-ai/*`
/// 与 `@iyam/*`）。用于启动失败但 stderr 点不出具体插件的场景——此时无法定位肇事者，
/// 优先保证 DSH 能起来（app 可用），被禁插件经通知告知用户、待 dsh 版本变化自动恢复。
/// 返回被移除的条目名。
fn quarantine_all_third_party(home: &PathBuf) -> Vec<String> {
    let profile = home.join("profiles").join("web").join("package.json");
    let content = match fs::read_to_string(&profile) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut doc: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
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
                if !is_core_bundle(name) {
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
        record_quarantine(home, &removed);
    }
    removed
}

/// 把本次被禁用的插件合并记录到 `<home>/.quarantine.json`，并附上当前 dsh 版本。
/// 供「dsh 版本变化后自动恢复重试」（`reinstate_quarantined`）与回滚时还原。
fn record_quarantine(home: &PathBuf, removed: &[String]) {
    let path = home.join(".quarantine.json");
    let mut doc: serde_json::Value = fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| serde_json::json!({ "disabled": [] }));
    let disabled = doc["disabled"].as_array_mut();
    let Some(disabled) = disabled else { return };
    for name in removed {
        if !disabled.iter().any(|v| v.as_str() == Some(name)) {
            disabled.push(serde_json::Value::String(name.to_string()));
        }
    }
    doc["dsh_version"] = match dsh_core_version(home) {
        Some(v) => serde_json::Value::String(v),
        None => serde_json::Value::Null,
    };
    if let Ok(s) = serde_json::to_string_pretty(&doc) {
        let _ = fs::write(&path, s);
    }
}

/// 当前 dsh 核心包版本（读 `<core>/package.json`）；不可解析返回 None。
fn dsh_core_version(home: &PathBuf) -> Option<String> {
    crate::downloader::package_version(&crate::installer::dsh_core_dir(home))
}

/// 恢复此前因不兼容被禁用的第三方插件（重新加回 profile bundles，幂等）。
/// `downloader::rollback_after_failure` 回滚时会以 force 调用，故为 pub(crate)。
///
/// 两种触发：
/// - `force=true`：升级失败回滚后调用——回滚即恢复到升级前的兼容组合，全部还原；
/// - `force=false`：每次启动调用——记录时的 dsh 版本与当前不同（已升级）才恢复，
///   新版 dsh 下插件可能已适配；若仍不兼容，会再次启动失败并被重新隔离。
pub(crate) fn reinstate_quarantined(home: &PathBuf, force: bool) {
    let path = home.join(".quarantine.json");
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let doc: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };
    let disabled: Vec<String> = doc["disabled"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if disabled.is_empty() {
        let _ = fs::remove_file(&path);
        return;
    }
    if !force {
        let recorded = doc["dsh_version"].as_str();
        let current = dsh_core_version(home);
        // 版本未知（core 尚未就绪 / 记录时无版本）不轻举妄动；同版本也不必重试。
        match (recorded, current) {
            (Some(a), Some(b)) if a != b => {}
            _ => return,
        }
    }
    let profile = home.join("profiles").join("web").join("package.json");
    let Ok(content) = fs::read_to_string(&profile) else {
        return;
    };
    let mut doc: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };
    let Some(bundles) = doc
        .get_mut("dsh")
        .and_then(|d| d.get_mut("profile"))
        .and_then(|p| p.get_mut("bundles"))
        .and_then(|b| b.as_array_mut())
    else {
        return;
    };
    let mut reinstated: Vec<String> = Vec::new();
    for name in &disabled {
        // 包已不存在（被卸载/清理）的不恢复，且视为已处理。
        let pkg_exists = home
            .join("profiles")
            .join("web")
            .join("node_modules")
            .join(name)
            .exists()
            || crate::installer::dsh_node_modules(home).join(name).exists();
        if !pkg_exists {
            continue;
        }
        if !bundles.iter().any(|b| b.as_str() == Some(name)) {
            bundles.push(serde_json::Value::String(name.clone()));
            reinstated.push(name.clone());
        }
    }
    if reinstated.is_empty() {
        let _ = fs::remove_file(&path);
        return;
    }
    if let Ok(s) = serde_json::to_string_pretty(&doc) {
        let _ = fs::write(&profile, s);
    }
    let _ = fs::remove_file(&path);
    log::info!(
        "dsh 版本已变化，恢复上次被禁用的插件重试: {}",
        reinstated.join(", ")
    );
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
    fs::remove_file(home.join(".iyam-dsh.url")).ok();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实故障样本：dsh 0.1.2-rc.1 移除 `installSettingsSection` 后，dshmarket 1.20.0
    /// 链接失败拖垮整棵 DSH 的 stderr（节选自用户机器 `~/.dsh/.iyam-dsh-stderr.log`）。
    const STDERR_DSHMARKET: &str = r#"
Error: dsh: plugin tree failed to load: failed to apply loader entry include (cordis:include): failed to import loader entry dsh-market (dshmarket): The requested module '@deepseek-ai/dsh-settings' does not provide an export named 'installSettingsSection'
file:///Users/u/.dsh/profiles/web/node_modules/.pnpm/dshmarket@1.20.0_@deepseek-ai+cordis@4.0.1/node_modules/dshmarket/lib/settings.js:35
import { installSettingsSection, settingsNamespace } from '@deepseek-ai/dsh-settings';
SyntaxError: The requested module '@deepseek-ai/dsh-settings' does not provide an export named 'installSettingsSection'
    at #asyncInstantiate (node:internal/modules/esm/module_job:327:21)
    at async file:///Users/u/.dsh/lib/node_modules/@deepseek-ai/cordis-plugin-loader/lib/index.js:274:41
    at async Entry._init (file:///Users/u/.dsh/lib/node_modules/@deepseek-ai/dsh/node_modules/@deepseek-ai/dsh-app-boot/lib/index.js:522:39)
    at boot (file:///Users/u/.dsh/lib/node_modules/@deepseek-ai/dsh/node_modules/@deepseek-ai/dsh-app-boot/lib/index.js:1511:9)
"#;

    #[test]
    fn parse_broken_plugins_names_third_party_offender() {
        let parsed = parse_broken_plugins(STDERR_DSHMARKET);
        assert_eq!(parsed, vec!["dshmarket".to_string()]);
    }

    #[test]
    fn parse_broken_plugins_ignores_core_and_internal() {
        let text = r#"
    at async file:///h/.dsh/lib/node_modules/@deepseek-ai/cordis-plugin-loader/lib/index.js:1:1
    at #asyncInstantiate (node:internal/modules/esm/module_job:327:21)
Error: failed to import loader entry foo (@iyam/dsh-rtui-ui): boom
"#;
        assert!(parse_broken_plugins(text).is_empty());
    }

    #[test]
    fn parse_missing_packages_dedupes() {
        let text = "Cannot find package 'foo' \n x \n Cannot find package 'foo'";
        assert_eq!(parse_missing_packages(text), vec!["foo".to_string()]);
    }

    #[test]
    fn parse_web_url_captures_token_and_ignores_lan() {
        // 0.1.2-rc.1：带 token + LAN 尾注
        let line = r#"dsh web: http://127.0.0.1:60830/?token=Ab12-_.x (LAN: http://192.168.1.5:60830/?token=Ab12-_.x)"#;
        let url = parse_web_url(line).unwrap();
        assert_eq!(url, "http://127.0.0.1:60830/?token=Ab12-_.x");
        assert_eq!(port_of_url(&url), Some(60830));
        // 旧版本：无 token 裸 URL
        let old = parse_web_url("dsh web: http://127.0.0.1:5173").unwrap();
        assert_eq!(old, "http://127.0.0.1:5173");
        assert_eq!(port_of_url(&old), Some(5173));
        // 无关行不匹配
        assert!(parse_web_url("listening on http://127.0.0.1:1").is_none());
    }

    /// 临时目录里的迷你 DSH_HOME：profile 含第三方 dshmarket + 核心包，核心版本可写。
    fn fixture_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("iyam-quarantine-test-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("profiles").join("web").join("node_modules").join("dshmarket")).unwrap();
        let core = crate::installer::dsh_node_modules(&home)
            .join("@deepseek-ai")
            .join("dsh");
        fs::create_dir_all(&core).unwrap();
        fs::write(home.join("profiles").join("web").join("package.json"), serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dsh": { "profile": { "bundles": [
                "@deepseek-ai/dsh-base",
                "@iyam/dsh-desktop-shell",
                "dshmarket"
            ] } }
        }).to_string())
        .unwrap();
        set_core_version(&home, "0.1.1-rc.2");
        home
    }

    fn set_core_version(home: &PathBuf, version: &str) {
        let core = crate::installer::dsh_node_modules(home)
            .join("@deepseek-ai")
            .join("dsh");
        fs::write(
            core.join("package.json"),
            serde_json::json!({ "name": "@deepseek-ai/dsh", "version": version }).to_string(),
        )
        .unwrap();
    }

    fn bundles(home: &PathBuf) -> Vec<String> {
        let content = fs::read_to_string(home.join("profiles").join("web").join("package.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        v["dsh"]["profile"]["bundles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn quarantine_named_then_reinstate_on_version_change() {
        let home = fixture_home("named");
        // 点名剥离 dshmarket：从 bundles 移除、写入隔离记录（带当前版本）。
        let removed = quarantine_broken_plugins(&home, &[], &["dshmarket".to_string()]);
        assert_eq!(removed, vec!["dshmarket".to_string()]);
        assert!(!bundles(&home).contains(&"dshmarket".to_string()));
        assert!(bundles(&home).contains(&"@deepseek-ai/dsh-base".to_string()));
        let record: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(home.join(".quarantine.json")).unwrap()).unwrap();
        assert_eq!(record["dsh_version"].as_str(), Some("0.1.1-rc.2"));

        // 同版本启动：不恢复（插件大概率仍不兼容）。
        reinstate_quarantined(&home, false);
        assert!(!bundles(&home).contains(&"dshmarket".to_string()));

        // dsh 升级后启动：自动恢复重试，隔离记录清除。
        set_core_version(&home, "0.1.2-rc.1");
        reinstate_quarantined(&home, false);
        assert!(bundles(&home).contains(&"dshmarket".to_string()));
        assert!(!home.join(".quarantine.json").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn rollback_force_reinstates_and_all_third_party_strips_non_core() {
        let home = fixture_home("all");
        // 兜底隔离：只剥第三方，核心 @deepseek-ai/* 与 @iyam/* 保留。
        let removed = quarantine_all_third_party(&home);
        assert_eq!(removed, vec!["dshmarket".to_string()]);
        assert_eq!(bundles(&home), vec!["@deepseek-ai/dsh-base".to_string(), "@iyam/dsh-desktop-shell".to_string()]);

        // 回滚（force）：无视版本恢复全部被禁插件。
        reinstate_quarantined(&home, true);
        assert!(bundles(&home).contains(&"dshmarket".to_string()));
        assert!(!home.join(".quarantine.json").exists());
        let _ = fs::remove_dir_all(&home);
    }
}
