use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::Serialize;
use tauri::Manager;

/// Global DSH_HOME — set on first install
static DSH_HOME: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Serialize, Clone, PartialEq)]
pub enum InstallStatus {
    Installed,
    #[allow(dead_code)]
    Installing { progress: f64, stage: String },
    NotInstalled,
    #[allow(dead_code)]
    Error(String),
}

/// Node 平台目录名，与 scripts/fetch-node.mjs 的 TARGETS key 保持一致
pub(crate) fn node_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "darwin-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "darwin-x64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "win32-x64"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "win32-arm64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-arm64"
    }
}

pub(crate) fn node_exe_name() -> &'static str {
    if cfg!(windows) {
        "node.exe"
    } else {
        "node"
    }
}

/// DSH 数据/运行时统一目录：优先 `$DSH_HOME`，否则 `~/.dsh`。
/// 与用户自行 `npm i -g @deepseek-ai/dsh` 的默认 home 一致——不再使用隔离的
/// `~/.iyam-dsh`，以便复用用户已装的标准插件，且 app 帮装时也落在同一目录。
pub(crate) fn dsh_home() -> PathBuf {
    DSH_HOME
        .get()
        .cloned()
        .unwrap_or_else(|| {
            if let Ok(v) = env::var("DSH_HOME") {
                if !v.is_empty() {
                    return PathBuf::from(v);
                }
            }
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".dsh")
        })
}

/// 在 PATH 上解析命令的完整路径（Windows 用 `where`，类 Unix 用 `which`）。
fn resolve_in_path(name: &str) -> Option<PathBuf> {
    let out = if cfg!(windows) {
        let mut c = Command::new("where");
        c.arg(name);
        // 隐藏控制台窗口：否则每次启动探测 dsh 时都会闪一个 cmd 窗。
        c.creation_flags(0x0800_0000);
        c.output().ok()?
    } else {
        Command::new("which").arg(name).output().ok()?
    };
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let first = line.lines().next()?.trim();
    if first.is_empty() {
        return None;
    }
    Some(PathBuf::from(first))
}

/// 验证给定 cli 是否为可用的 dsh（`dsh --version` 成功，且输出看起来像版本号）。
/// 注意：新版 dsh（如 0.1.1-rc.2）`--version` 只输出版本号（如 `0.1.1-rc.2`），
/// 不含 "dsh" 字样，故不能再用 contains("dsh") 判断，否则会误判为不可用。
/// Windows 上 dsh 是 `.cmd`：直接 `Command::new(cli)` 即可（OS 会经 cmd 运行 .cmd），
/// 不要用 `cmd /c "cli" --version`——那种手动拼接引号的写法在 spawn 时会导致
/// cmd 把整段（含引号）当成命令名而报"不是内部或外部命令"，从而误判为不可用。
/// 这与 `process.rs` 启动 dsh 的方式保持一致（同样直接 `Command::new(&cli)`）。
fn verify_dsh(cli: &PathBuf) -> bool {
    let mut cmd = Command::new(cli);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW：探测时避免闪控制台窗
    }
    match cmd.output() {
        Ok(o) => {
            if !o.status.success() {
                return false;
            }
            // 成功退出后，确认输出是版本号（含数字与点，或含 rc/pre 预发布标记），
            // 而非 npm 类的错误页；空输出也放宽接受（部分构建只退出 0）。
            let s = String::from_utf8_lossy(&o.stdout);
            let looks_like_version = s.trim().is_empty()
                || (s.chars().any(|c| c.is_ascii_digit())
                    && s.contains('.')
                    && !s.to_lowercase().contains("error"));
            looks_like_version
        }
        Err(_) => false,
    }
}

/// 定位可用的 dsh CLI，三级回退：
/// 1. PATH 上的 `dsh`/`dsh.cmd`（用户自行安装或 app 之前帮装且加入 PATH）。
/// 2. `~/.dsh/bin/dsh(.cmd)`（app 之前帮装、未加入 PATH）。
/// 3. 都没有 → None（需下载安装）。
pub(crate) fn detect_dsh_cli() -> Option<PathBuf> {
    let name = if cfg!(windows) { "dsh.cmd" } else { "dsh" };
    if let Some(p) = resolve_in_path(name) {
        if verify_dsh(&p) {
            return Some(p);
        }
    }
    let home = dsh_home();
    // 全局安装（npm i -g --prefix ~/.dsh）会在 home 根生成入口：
    // Windows 顶层 dsh.cmd；类 Unix 在 home/bin/dsh（npm 全局 bin）。
    // 顺序：PATH > home/dsh.cmd(全局生成) > home/bin/dsh.cmd(本 app wrapper)。
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        vec![
            home.join("dsh.cmd"),
            home.join("bin").join("dsh.cmd"),
        ]
    } else {
        vec![
            home.join("bin").join("dsh"),
            home.join("bin").join("dsh"),
        ]
    };
    for local in candidates {
        if local.exists() && verify_dsh(&local) {
            return Some(local);
        }
    }
    None
}

/// dsh 核心包根目录（全局安装布局）：`~/.dsh/node_modules/@deepseek-ai/dsh`。
/// 与手动 `npm i -g @deepseek-ai/dsh` 的产物一致。
pub(crate) fn dsh_core_dir(home: &PathBuf) -> PathBuf {
    home.join("node_modules").join("@deepseek-ai").join("dsh")
}

/// 该 dsh 是否由本 app 托管（即 app 帮装的）。仅托管态才由 app 自动/手动升级。
/// 用户自行 `npm i -g` 的 dsh 无此标记，app 不擅自改动。
pub(crate) fn is_managed() -> bool {
    dsh_home().join(".iyam-managed").exists()
}

/// 定位 bundle 内的桌面壳 companion 插件包（注入 DSH 布局 CSS）
pub(crate) fn bundled_shell_plugin(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = res_dir.join("bin").join("dsh-shell-plugin");
        if candidate.join("lib").join("client.js").exists() {
            return Some(candidate);
        }
    }
    if let Some(candidate) =
        exe_dir_candidate("dsh-shell-plugin", |p| p.join("lib").join("client.js").exists())
    {
        return Some(candidate);
    }
    if let Ok(cwd) = env::current_dir() {
        for candidate in [
            cwd.join("src-tauri").join("bin").join("dsh-shell-plugin"),
            cwd.join("bin").join("dsh-shell-plugin"),
        ] {
            if candidate.join("lib").join("client.js").exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 定位 bundle 内的主题 UI 插件包（注入 DSH web UI 主题 token + 设置面板）
pub(crate) fn bundled_rtui_ui_plugin(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = res_dir.join("bin").join("dsh-rtui-ui");
        if candidate.join("client.js").exists() && candidate.join("package.json").exists() {
            return Some(candidate);
        }
    }
    if let Some(candidate) =
        exe_dir_candidate("dsh-rtui-ui", |p| p.join("client.js").exists() && p.join("package.json").exists())
    {
        return Some(candidate);
    }
    if let Ok(cwd) = env::current_dir() {
        for candidate in [
            cwd.join("src-tauri").join("bin").join("dsh-rtui-ui"),
            cwd.join("bin").join("dsh-rtui-ui"),
        ] {
            if candidate.join("client.js").exists() && candidate.join("package.json").exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 定位 bundle 内的文件查看插件包（包装 openPath，转发文件点击给桌面壳预览）
pub(crate) fn bundled_file_handler_plugin(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = res_dir.join("bin").join("dsh-file-handler");
        if candidate.join("client.js").exists() && candidate.join("package.json").exists() {
            return Some(candidate);
        }
    }
    if let Some(candidate) = exe_dir_candidate("dsh-file-handler", |p| {
        p.join("client.js").exists() && p.join("package.json").exists()
    }) {
        return Some(candidate);
    }
    if let Ok(cwd) = env::current_dir() {
        for candidate in [
            cwd.join("src-tauri").join("bin").join("dsh-file-handler"),
            cwd.join("bin").join("dsh-file-handler"),
        ] {
            if candidate.join("client.js").exists() && candidate.join("package.json").exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 基于可执行文件所在目录解析资源（dev 模式 build.rs 将资源复制到 exe 同级）
fn exe_dir_candidate(subdir: &str, check: impl Fn(&PathBuf) -> bool) -> Option<PathBuf> {
    if let Ok(exe) = env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join(subdir);
            if check(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[tauri::command]
pub(crate) fn get_install_status(_app: tauri::AppHandle) -> InstallStatus {
    let home = dsh_home();
    if detect_dsh_cli().is_some() && dsh_core_dir(&home).join("lib").join("bin.js").exists() {
        return InstallStatus::Installed;
    }
    InstallStatus::NotInstalled
}

/// 确保 dsh 可用：若系统/PATH 已有 dsh 仅注入体验插件与补丁；否则运行时下载
/// node(含 npm) + 安装 dsh 到 `~/.dsh`（与用户自行安装一致），并打托管标记。
#[tauri::command]
pub(crate) async fn check_and_install(app: tauri::AppHandle) -> Result<InstallStatus, String> {
    let home = dsh_home();

    if detect_dsh_cli().is_some() && dsh_core_dir(&home).join("lib").join("bin.js").exists() {
        inject_plugins_and_patches(&app)?;
        DSH_HOME.set(home.clone()).ok();
        return Ok(InstallStatus::Installed);
    }

    // 需要下载安装（系统无 dsh）
    log::info!("安装 DSH 运行环境到: {:?}", home);
    crate::downloader::bootstrap_dsh(&app, &home).await?;
    let node = managed_node(&home);
    create_wrappers(&home, &node)?;
    fs::write(home.join(".iyam-managed"), "")
        .map_err(|e| format!("写托管标记失败: {}", e))?;
    inject_plugins_and_patches(&app)?;
    DSH_HOME.set(home.clone()).ok();
    log::info!("DSH 已安装到: {:?}", home);
    Ok(InstallStatus::Installed)
}

/// 注入三个体验插件 + 任务栏 AUMID 预加载 + 目录选择器 owner 补丁（幂等）。
/// 所有写入都落在 `dsh_home()`（即 `~/.dsh`），与用户自行安装的 dsh 一致。
fn inject_plugins_and_patches(app: &tauri::AppHandle) -> Result<(), String> {
    if let Err(e) = refresh_shell_plugin(app) {
        log::warn!("inject shell plugin failed: {}", e);
    }
    if let Err(e) = refresh_rtui_ui_plugin(app) {
        log::warn!("inject rtui-ui plugin failed: {}", e);
    }
    if let Err(e) = refresh_file_handler_plugin(app) {
        log::warn!("inject file-handler plugin failed: {}", e);
    }
    let home = dsh_home();
    if let Err(e) = ensure_taskbar_preload(&home) {
        log::warn!("ensure taskbar preload failed: {}", e);
    }
    ensure_picker_owner_patch(&home);
    Ok(())
}

/// 预装插件市场 dshmarket：经 DSH 自带的 `dsh plugin --profile web add dshmarket` 安装，
/// 由 DSH 负责 pnpm 安装并自动注册到 web profile 的 `dsh.profile.bundles`。
///
/// 幂等：profile 的 bundles 已含 `dshmarket` 且包已装到 `profiles/web/node_modules/dshmarket`
/// 时直接跳过，避免每次启动重复下载。失败（无网络 / pnpm 缺失）仅告警并 continue，
/// 绝不阻断首次启动——市场是增强项，非核心功能。
pub(crate) async fn ensure_dshmarket(app: &tauri::AppHandle) {
    let home = dsh_home();
    if dshmarket_installed(&home) {
        return;
    }
    let cli = match detect_dsh_cli() {
        Some(c) => c,
        None => {
            log::warn!("ensure_dshmarket: dsh cli 未找到，跳过市场预装");
            return;
        }
    };
    let app_c = app.clone();
    let res = tauri::async_runtime::spawn_blocking(move || run_dsh_plugin_add(&cli, &home))
        .await;
    match res {
        Ok(Ok(())) => log::info!("已预装 dshmarket 插件市场"),
        Ok(Err(e)) => log::warn!("预装 dshmarket 失败（不影响核心功能）: {}", e),
        Err(e) => log::warn!("预装 dshmarket 线程失败: {}", e),
    }
    let _ = app_c;
}

/// 市场是否已安装：profile 的 bundles 含 dshmarket 且包已落地。
fn dshmarket_installed(home: &PathBuf) -> bool {
    let profile_pkg = home.join("profiles").join("web").join("package.json");
    let in_bundles = profile_pkg
        .exists()
        .then(|| fs::read_to_string(&profile_pkg).ok())
        .flatten()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .map(|v| {
            v.get("dsh")
                .and_then(|d| d.get("profile"))
                .and_then(|p| p.get("bundles"))
                .and_then(|b| b.as_array())
                .map(|a| a.iter().any(|x| x.as_str() == Some("dshmarket")))
                .unwrap_or(false)
        })
        .unwrap_or(false);
    in_bundles
        && home
            .join("profiles")
            .join("web")
            .join("node_modules")
            .join("dshmarket")
            .exists()
}

/// 用托管 node 跑 `dsh plugin --profile web add dshmarket`（DSH 内部 spawnSync pnpm）。
/// 管道化输出、隐藏控制台窗、整体超时（避免无网络时挂死首次启动）。
fn run_dsh_plugin_add(cli: &PathBuf, home: &PathBuf) -> Result<(), String> {
    const TIMEOUT: u64 = 5 * 60; // 5 分钟整体超时
    let mut cmd = Command::new(cli);
    cmd.arg("plugin")
        .arg("--profile")
        .arg("web")
        .arg("add")
        .arg("dshmarket")
        .env("DSH_HOME", home.to_string_lossy().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW：避免安装时闪控制台窗
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 dsh plugin add 失败: {}", e))?;
    let pid = child.id();

    // 排空 stdout/stderr，避免管道缓冲区写满导致进程阻塞挂死。
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let drain = |r: std::process::ChildStdout| {
        let reader = BufReader::new(r);
        for line in reader.lines().map_while(Result::ok) {
            log::info!("[dshmarket] {}", line);
        }
    };
    let drain2 = |r: std::process::ChildStderr| {
        let reader = BufReader::new(r);
        for line in reader.lines().map_while(Result::ok) {
            log::info!("[dshmarket] {}", line);
        }
    };
    let h1 = std::thread::spawn(move || drain(stdout));
    let h2 = std::thread::spawn(move || drain2(stderr));

    let (tx, rx) = std::sync::mpsc::channel::<Option<std::process::ExitStatus>>();
    let waiter = std::thread::spawn(move || {
        let _ = tx.send(child.wait().ok());
    });

    match rx.recv_timeout(std::time::Duration::from_secs(TIMEOUT)) {
        Ok(Some(status)) => {
            let _ = h1.join();
            let _ = h2.join();
            let _ = waiter.join();
            if status.success() {
                Ok(())
            } else {
                Err("dsh plugin add 退出码非零".into())
            }
        }
        Ok(None) => {
            let _ = h1.join();
            let _ = h2.join();
            let _ = waiter.join();
            Err("无法等待 dsh plugin add 进程".into())
        }
        Err(_) => {
            kill_process_tree(pid);
            let _ = h1.join();
            let _ = h2.join();
            let _ = waiter.join();
            Err(format!("dsh plugin add 超时（>{}s）", TIMEOUT))
        }
    }
}

/// 跨平台杀整棵进程树（pnpm 会派生子进程）。
fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x0800_0000)
            .output();
    }
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

/// 启动早期调用的升级生效检查：若 `~/.dsh/.update.json` 标记有已备货的新版本，
/// 则把 `.staging` 提升到正式目录并重施插件/补丁/wrapper。返回是否执行了提升。
/// 与旧 `refresh_dsh_core`（复制 bundle）不同：现在数据源是运行时备货目录。
pub(crate) fn refresh_dsh_core(_app: &tauri::AppHandle) -> Result<(), String> {
    if crate::downloader::apply_staged_if_ready(&dsh_home()) {
        log::info!("已应用暂存升级");
    }
    Ok(())
}

/// 生成可手动执行的 dsh 命令入口（终端使用 + DSH 内部工具调用）。
/// unix 用 sh 脚本，Windows 用 .cmd，均指向托管 node 绝对路径。
pub(crate) fn create_wrappers(home: &PathBuf, node: &PathBuf) -> Result<(), String> {
    let bin_js = dsh_core_dir(home).join("lib").join("bin.js");
    fs::create_dir_all(home.join("bin")).map_err(|e| format!("创建目录失败: {}", e))?;

    #[cfg(unix)]
    {
        let sh = format!("#!/bin/sh\nexec \"{}\" \"{}\" \"$@\"\n", node.display(), bin_js.display());
        fs::write(home.join("bin").join("dsh"), sh)
            .map_err(|e| format!("创建启动脚本失败: {}", e))?;
        fs::set_permissions(
            home.join("bin").join("dsh"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .ok();
    }

    #[cfg(windows)]
    {
        let bat = format!("@echo off\r\n\"{}\" \"{}\" %*\r\n", node.display(), bin_js.display());
        fs::write(home.join("bin").join("dsh.cmd"), bat)
            .map_err(|e| format!("创建启动脚本失败: {}", e))?;
    }

    Ok(())
}

/// 每次启动刷新桌面壳插件（幂等）：把 bundle 内的插件覆盖安装到 DSH_HOME，
/// 并确保注册到 web profile 的 bundles。旧安装因此也能获得桌面壳更新。
pub(crate) fn refresh_shell_plugin(app: &tauri::AppHandle) -> Result<(), String> {
    let home = dsh_home();
    if let Some(plugin) = bundled_shell_plugin(app) {
        install_shell_plugin(&home, &plugin)
    } else {
        Ok(())
    }
}

/// 安装桌面壳 companion 插件：
/// 1. 复制到 <DSH_HOME>/node_modules/@iyam/dsh-desktop-shell
/// 2. 注册到 <DSH_HOME>/profiles/web/package.json 的 dsh.profile.bundles（幂等）
fn install_shell_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = home
        .join("node_modules")
        .join("@iyam")
        .join("dsh-desktop-shell");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制桌面壳插件失败: {}", e))?;

    let profile_pkg = home.join("profiles").join("web").join("package.json");
    let mut v: serde_json::Value = if profile_pkg.exists() {
        let content =
            fs::read_to_string(&profile_pkg).map_err(|e| format!("读取 profile 配置失败: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("解析 profile 配置失败: {}", e))?
    } else {
        if let Some(parent) = profile_pkg.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建 profile 目录失败: {}", e))?;
        }
        serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"] } }
        })
    };

    let bundles = v["dsh"]["profile"]["bundles"]
        .as_array_mut()
        .ok_or("profile 配置缺少 dsh.profile.bundles")?;
    if !bundles
        .iter()
        .any(|b| b.as_str() == Some("@iyam/dsh-desktop-shell"))
    {
        bundles.push(serde_json::Value::String("@iyam/dsh-desktop-shell".into()));
    }

    let out = serde_json::to_string_pretty(&v).map_err(|e| format!("序列化 profile 配置失败: {}", e))?;
    fs::write(&profile_pkg, out + "\n").map_err(|e| format!("写入 profile 配置失败: {}", e))?;
    Ok(())
}

/// 每次启动刷新主题 UI 插件（幂等）
pub(crate) fn refresh_rtui_ui_plugin(app: &tauri::AppHandle) -> Result<(), String> {
    let home = dsh_home();
    if let Some(plugin) = bundled_rtui_ui_plugin(app) {
        install_rtui_ui_plugin(&home, &plugin)
    } else {
        Ok(())
    }
}

/// 安装主题 UI 插件：复制到 <DSH_HOME>/node_modules/@iyam/dsh-rtui-ui，注册 bundles。
fn install_rtui_ui_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = home.join("node_modules").join("@iyam").join("dsh-rtui-ui");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制主题 UI 插件失败: {}", e))?;

    let profile_pkg = home.join("profiles").join("web").join("package.json");
    let mut v: serde_json::Value = if profile_pkg.exists() {
        let content =
            fs::read_to_string(&profile_pkg).map_err(|e| format!("读取 profile 配置失败: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("解析 profile 配置失败: {}", e))?
    } else {
        if let Some(parent) = profile_pkg.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建 profile 目录失败: {}", e))?;
        }
        serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"] } }
        })
    };

    let bundles = v["dsh"]["profile"]["bundles"]
        .as_array_mut()
        .ok_or("profile 配置缺少 dsh.profile.bundles")?;
    if !bundles
        .iter()
        .any(|b| b.as_str() == Some("@iyam/dsh-rtui-ui"))
    {
        bundles.push(serde_json::Value::String("@iyam/dsh-rtui-ui".into()));
    }

    let out = serde_json::to_string_pretty(&v).map_err(|e| format!("序列化 profile 配置失败: {}", e))?;
    fs::write(&profile_pkg, out + "\n").map_err(|e| format!("写入 profile 配置失败: {}", e))?;
    Ok(())
}

/// 每次启动刷新文件查看插件（幂等）
pub(crate) fn refresh_file_handler_plugin(app: &tauri::AppHandle) -> Result<(), String> {
    let home = dsh_home();
    if let Some(plugin) = bundled_file_handler_plugin(app) {
        install_file_handler_plugin(&home, &plugin)
    } else {
        Ok(())
    }
}

/// 安装文件查看插件：复制到 <DSH_HOME>/node_modules/@iyam/dsh-file-handler，注册 bundles。
fn install_file_handler_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = home
        .join("node_modules")
        .join("@iyam")
        .join("dsh-file-handler");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制文件查看插件失败: {}", e))?;

    let profile_pkg = home.join("profiles").join("web").join("package.json");
    let mut v: serde_json::Value = if profile_pkg.exists() {
        let content =
            fs::read_to_string(&profile_pkg).map_err(|e| format!("读取 profile 配置失败: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("解析 profile 配置失败: {}", e))?
    } else {
        if let Some(parent) = profile_pkg.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建 profile 目录失败: {}", e))?;
        }
        serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"] } }
        })
    };

    let bundles = v["dsh"]["profile"]["bundles"]
        .as_array_mut()
        .ok_or("profile 配置缺少 dsh.profile.bundles")?;
    if !bundles
        .iter()
        .any(|b| b.as_str() == Some("@iyam/dsh-file-handler"))
    {
        bundles.push(serde_json::Value::String("@iyam/dsh-file-handler".into()));
    }

    let out = serde_json::to_string_pretty(&v).map_err(|e| format!("序列化 profile 配置失败: {}", e))?;
    fs::write(&profile_pkg, out + "\n").map_err(|e| format!("写入 profile 配置失败: {}", e))?;
    Ok(())
}

/// 任务栏 AUMID 预加载脚本（同前，略）
const TASKBAR_AUMID_PRELOAD: &str = r#"// iyam-dsh: 与主应用共享 AppUserModelID
try {
  const koffi = require('koffi');
  const shell32 = koffi.load('shell32.dll');
  const setAumid = shell32.func(
    'int32 __stdcall SetCurrentProcessExplicitAppUserModelID(str16 AppID)'
  );
  setAumid('ai.iyam.dsh');
} catch (_) {
  // noop
}
"#;

/// 幂等写入任务栏 AUMID 预加载脚本到 DSH_HOME
pub(crate) fn ensure_taskbar_preload(home: &std::path::Path) -> Result<(), String> {
    let path = home.join("set-taskbar-aumid.cjs");
    if path.exists() {
        return Ok(());
    }
    fs::write(&path, TASKBAR_AUMID_PRELOAD)
        .map_err(|e| format!("写入任务栏预加载脚本失败: {}", e))
}

fn copy_dir_all(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let name = entry.file_name();
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(name))?;
        } else {
            fs::copy(&entry.path(), &dst.join(name))?;
        }
    }
    Ok(())
}

/// 为 native 目录选择器的对话框 worker 打 owner 补丁（幂等）。同前。
pub(crate) fn ensure_picker_owner_patch(home: &PathBuf) {
    let worker = home
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh-host-directory-picker-native")
        .join("lib")
        .join("worker.cjs");
    let content = match fs::read_to_string(&worker) {
        Ok(c) => c,
        Err(_) => return,
    };
    if content.contains("const _h = process.env.DSH_DIALOG_OWNER_HWND; let _o = null; if (_h && /^[0-9]+$/.test(_h)) { const _n = Number(_h); if (_n > 0 && _n <= 0x7fffffff) { _o = _n; } }") {
        return;
    }
    const FROM: &str = "show: () => method(dialog, SLOT_SHOW, protoShow)(null),";
    const TO: &str = "show: () => { const _h = process.env.DSH_DIALOG_OWNER_HWND; let _o = null; if (_h && /^[0-9]+$/.test(_h)) { const _n = Number(_h); if (_n > 0 && _n <= 0x7fffffff) { _o = _n; } } return method(dialog, SLOT_SHOW, protoShow)(_o); },";
    const OLD_TO: &str = "show: () => method(dialog, SLOT_SHOW, protoShow)(process.env.DSH_DIALOG_OWNER_HWND ? Number(process.env.DSH_DIALOG_OWNER_HWND) : null),";
    const OLD_KOFFI: &str = "show: () => { const _h = process.env.DSH_DIALOG_OWNER_HWND; let _o = null; if (_h && /^[0-9]+$/.test(_h)) { const _n = Number(_h); if (_n > 0 && _n <= 0x7fffffff) { try { const _u = koffi.load('user32.dll'); const _isw = _u.func('__stdcall', 'IsWindow', 'int32', ['void *']); if (_isw(_n)) _o = _n; } catch (_e) { _o = null; } } } return method(dialog, SLOT_SHOW, protoShow)(_o); },";
    let base = content.replace(OLD_TO, FROM).replace(OLD_KOFFI, FROM);
    if base.contains(FROM) {
        let patched = base.replace(FROM, TO);
        if let Err(e) = fs::write(&worker, patched) {
            log::warn!("写目录选择器 owner 补丁失败({}): {e}", worker.display());
        } else {
            log::info!("已为目录选择器打 owner 补丁: {}", worker.display());
        }
    } else {
        log::warn!(
            "worker.cjs 结构变化，未打对话框 owner 补丁（引擎升级后需同步；目录选择仍可用，仅对话框图标为 node）"
        );
    }
}

/// 返回托管 node 可执行文件路径（若存在）。Windows 在根 `node.exe`；类 Unix 在 `bin/node`。
pub(crate) fn managed_node(home: &PathBuf) -> PathBuf {
    let base = home.join("node").join(node_target());
    if cfg!(windows) {
        base.join("node.exe")
    } else {
        base.join("bin").join("node")
    }
}
