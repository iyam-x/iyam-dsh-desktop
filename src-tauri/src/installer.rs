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
        #[cfg(windows)]
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
/// dsh 的真实入口文件（与 `npm i -g --prefix` 一致）：`<home>/lib/node_modules/@deepseek-ai/dsh/lib/bin.js`。
/// 不要用 `home/bin/dsh`（npm 生成的软链）当作启动对象——部分镜像分发的 tarball 里
/// `bin.js` 是坏掉的 `#!/bin/sh` 自调用壳子，npm 仍会生成指向它的软链，运行时 node 解析
/// 即报 `SyntaxError: Unexpected string`。跨平台统一用"托管 node 直接跑 bin.js"最稳妥。
pub(crate) fn dsh_bin_js(home: &PathBuf) -> PathBuf {
    dsh_core_dir(home).join("lib").join("bin.js")
}

/// 校验某个 dsh 入口是否可用。**跨平台统一用托管 node 直接跑 `bin.js --version`**，
/// 而非依赖 `bin/dsh` 软链 + shebang（shebang 解析依赖 PATH 上的 node，且坏壳子会直接崩）。
///
/// `bin_js`：要校验的真实入口（托管态传 `dsh_bin_js(home)`，系统态传 core_dir 推导出的 bin.js）。
/// `node`：用来运行它的 node 可执行文件（托管态传托管 node；系统态传 `None` 让 OS 自行找 node）。
fn verify_dsh_entry(bin_js: &PathBuf, node: &Option<PathBuf>) -> bool {
    let mut cmd = match node {
        Some(n) => Command::new(n),
        None => {
            // 系统态：OS 上通常 sh 会按 shebang 跑；这里退回用 `node` 命令（依赖 PATH）。
            Command::new("node")
        }
    };
    cmd.arg(bin_js).arg("--version");
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

/// 定位可用的 dsh 入口（真实 `bin.js` 路径），回退顺序（优先用 app 自己托管的 dsh，避免受用户开发环境干扰）：
/// 1. `~/.dsh/lib/node_modules/@deepseek-ai/dsh/lib/bin.js`（本 app 托管安装），用托管 node 直接运行校验。
///    之所以不校验 `home/bin/dsh` 软链：部分镜像 tarball 的 `bin.js` 是坏掉的 `#!/bin/sh` 自调用壳子，
///    npm 仍会生成指向它的软链，靠软链+shebang 必然崩；直接用托管 node 跑 `bin.js` 才能真实反映可用性。
/// 2. PATH 上的 `dsh`/`dsh.cmd`（用户自行 `npm i -g` 安装，最后兜底），用 OS 的 node 跑其 core 下的 bin.js。
/// 都没有 → None（需下载安装）。
///
/// 注意：必须先查托管 dsh，再查系统 PATH。否则当用户开发环境里也装了 `@deepseek-ai/dsh`
/// （如 nvm 全局）时，app 会误用系统 dsh，而系统 dsh 的版本/数据布局与 app 托管的不一致，
/// 导致启动探测/运行异常。app 始终以自身托管的 `~/.dsh` 为准。
pub(crate) fn detect_dsh_cli() -> Option<PathBuf> {
    let home = dsh_home();
    let managed = dsh_bin_js(&home);
    let node = managed_node(&home);
    if managed.exists() && verify_dsh_entry(&managed, &Some(node)) {
        return Some(managed);
    }
    // 兜底：系统/PATH 上的 dsh（用户自行安装，app 不托管），用 OS 的 node 跑。
    let name = if cfg!(windows) { "dsh.cmd" } else { "dsh" };
    if let Some(sys_cli) = resolve_in_path(name) {
        // 系统 dsh 的 bin/dsh 是软链 → bin.js；canonicalize 后即 bin.js 真实路径。
        if let Ok(real) = sys_cli.canonicalize() {
            if verify_dsh_entry(&real, &None) {
                return Some(real);
            }
        }
    }
    None
}

/// npm 全局安装（`npm i -g --prefix <home>`）的依赖根目录。
/// 类 Unix 为 `<home>/lib/node_modules`（npm 全局布局），Windows 为 `<home>/node_modules`。
/// 库内凡引用 dsh/@iyam 安装树处都必须走此函数，不能硬编码 `node_modules`。
pub(crate) fn dsh_node_modules(home: &PathBuf) -> PathBuf {
    if cfg!(windows) {
        home.join("node_modules")
    } else {
        home.join("lib").join("node_modules")
    }
}

/// dsh 核心包根目录（全局安装布局）：`<home>/lib/node_modules/@deepseek-ai/dsh`（类 Unix）。
/// 与手动 `npm i -g @deepseek-ai/dsh` 的产物一致。
pub(crate) fn dsh_core_dir(home: &PathBuf) -> PathBuf {
    dsh_node_modules(home)
        .join("@deepseek-ai")
        .join("dsh")
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
pub(crate) fn dshmarket_installed(home: &PathBuf) -> bool {
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

/// 确保托管 node 目录里装有 pnpm。dsh 的 `plugin` 命令内部 `spawnSync("pnpm", ...)`
/// 要求 pnpm 在 PATH 上；而 GUI 启动的 app 没有用户 shell 的 PATH（nvm 等），
/// 托管 node 又只带 npm/npx，故 pnpm 必须由 app 自己装。
///
/// 幂等：托管 node 目录的 `bin/pnpm` 已存在即跳过。用托管 node 的 npm 全局安装到
/// node 目录自身（`--prefix <node_dir>`），pnpm 落在与 `node` 同目录的 `bin/`，
/// 配合 `prepend_managed_node_path` 前置 PATH 即可被 dsh 解析到。
fn ensure_pnpm(home: &PathBuf) -> Result<(), String> {
    let node_dir = home.join("node").join(node_target());
    // 全局 bin 位置平台不同：类 Unix 在 `<node_dir>/bin/pnpm`，Windows 在 `<node_dir>/pnpm.cmd`
    // （npm 在 Windows 把全局 bin shim 放 --prefix 根目录，不在 bin/ 子目录）。
    let pnpm_bin = if cfg!(windows) {
        node_dir.join("pnpm.cmd")
    } else {
        node_dir.join("bin").join("pnpm")
    };
    if pnpm_bin.exists() {
        return Ok(());
    }
    let node = managed_node(home);
    if !node.exists() {
        return Err("托管 node 不存在，无法安装 pnpm".into());
    }
    // npm-cli.js：Win 在 node_dir/node_modules/npm/bin；类 Unix 在 node_dir/lib/node_modules/npm/bin
    let npm_cli = if cfg!(windows) {
        node_dir.join("node_modules").join("npm").join("bin").join("npm-cli.js")
    } else {
        node_dir.join("lib").join("node_modules").join("npm").join("bin").join("npm-cli.js")
    };
    if !npm_cli.exists() {
        return Err(format!("npm 未找到: {}", npm_cli.display()));
    }

    // 与安装 dsh 相同的镜像回退链（npmmirror → 腾讯 → 华为 → npmjs）。
    let registries: &[&str] = &[
        "https://registry.npmmirror.com",
        "https://mirrors.cloud.tencent.com/npm/",
        "https://repo.huaweicloud.com/repository/npm/",
        "https://registry.npmjs.org",
    ];
    let mut last_err: Option<String> = None;
    for registry in registries {
        let mut cmd = Command::new(&node);
        cmd.arg(&npm_cli)
            .arg("install")
            .arg("-g")
            .arg("pnpm@11")
            .arg("--prefix")
            .arg(&node_dir)
            .arg("--no-audit")
            .arg("--no-fund")
            .arg("--registry")
            .arg(registry)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        // PATH 注入：npm 内部脚本（裸 `node`）需要托管 node 可解析。
        if let Some(dir) = node.parent() {
            if let Some(dir_s) = dir.to_str() {
                let mut new_path = dir_s.to_string();
                if let Ok(existing) = std::env::var("PATH") {
                    if !existing.is_empty() {
                        new_path.push_str(path_separator());
                        new_path.push_str(&existing);
                    }
                }
                cmd.env("PATH", new_path);
            }
        }
        #[cfg(windows)]
        {
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        match cmd.output() {
            Ok(o) if o.status.success() => return Ok(()),
            Ok(o) => {
                last_err = Some(format!(
                    "{}: {}",
                    registry,
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            Err(e) => {
                last_err = Some(format!("{}: {}", registry, e));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "pnpm 安装失败".into()))
}

/// 用托管 node 跑 `dsh plugin --profile web add dshmarket`（DSH 内部 spawnSync pnpm）。
/// 管道化输出、隐藏控制台窗、整体超时（避免无网络时挂死首次启动）。
fn run_dsh_plugin_add(cli: &PathBuf, home: &PathBuf) -> Result<(), String> {
    const TIMEOUT: u64 = 5 * 60; // 5 分钟整体超时
    // dsh plugin 命令内部 `spawnSync("pnpm", ...)` 需要 pnpm 在 PATH 上。
    // GUI 启动的 app 没有用户 shell 的 PATH（nvm 等），先确保托管 pnpm 就位。
    ensure_pnpm(home)?;
    // cli 为真实 bin.js 路径：跨平台统一用 node 直接跑，托管态用托管 node。
    let managed = cli.starts_with(home);
    let mut cmd = if managed {
        Command::new(managed_node(home))
    } else {
        Command::new("node")
    };
    // 托管 node 目录前置到 PATH：dsh 内部 spawn 的 pnpm（node 脚本）才能解析到
    // `node` 与 `pnpm`（二者都在托管 node 的 bin 目录，见 ensure_pnpm）。
    if let Some(node_path) = prepend_managed_node_path(home) {
        cmd.env("PATH", node_path);
    }
    cmd.arg(cli)
        .arg("plugin")
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
        let entry = home.join("bin").join("dsh");
        // 关键：npm 会把 `bin/dsh` 建成指向 `bin.js` 的软链。直接 `fs::write` 软链路径
        // 会顺着软链写穿到 `bin.js` 本身，把真实 ESM 入口改写成 sh 壳子（启动报
        // `SyntaxError: Unexpected string`）。必须先删掉软链，再写独立启动脚本。
        let _ = fs::remove_file(&entry);
        let sh = format!("#!/bin/sh\nexec \"{}\" \"{}\" \"$@\"\n", node.display(), bin_js.display());
        fs::write(&entry, sh).map_err(|e| format!("创建启动脚本失败: {}", e))?;
        fs::set_permissions(&entry, std::os::unix::fs::PermissionsExt::from_mode(0o755)).ok();
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

/// dsh 的 profile 模块解析是"双锚点 + flat fallback"（见 dsh-app-boot 的
/// `healProfilesModuleFallback`）：`$DSH_HOME/profiles/node_modules` 里 dsh 会为
/// **自身依赖闭包**建软链（每个包一条），profile 目录的 Node parent-walk 因此能解析
/// 到所有内置插件。`@iyam/*` 不在 dsh 依赖闭包内，dsh 不会为它建软链 → loader 从
/// `profiles/web` 解析 `@iyam/dsh-desktop-shell` 等报 `ERR_MODULE_NOT_FOUND`。
/// 此函数按同样机制在 `profiles/node_modules/@iyam/<name>` 建软链指向插件真实目录。
fn ensure_profile_iyam_link(home: &PathBuf, plugin_name: &str, real_dir: &std::path::Path) -> Result<(), String> {
    let link = home
        .join("profiles")
        .join("node_modules")
        .join("@iyam")
        .join(plugin_name);
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 profiles/node_modules 失败: {}", e))?;
    }
    // 幂等：清掉旧链接/目录再重建（旧的是软链用 remove_file，普通目录用 remove_dir_all）
    match fs::symlink_metadata(&link) {
        Ok(md) => {
            if md.file_type().is_symlink() {
                let _ = fs::remove_file(&link);
            } else if md.is_dir() {
                let _ = fs::remove_dir_all(&link);
            } else {
                let _ = fs::remove_file(&link);
            }
        }
        Err(_) => {}
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(real_dir, &link)
        .map_err(|e| format!("创建插件软链失败: {}", e))?;
    #[cfg(windows)]
    {
        // Windows 目录符号链接需要开发者模式/管理员权限；失败则退化为复制，同样可解析。
        if std::os::windows::fs::symlink_dir(real_dir, &link).is_err() {
            copy_dir_all(real_dir, &link)
                .map_err(|e| format!("复制插件到 profiles/node_modules 失败: {}", e))?;
        }
    }
    Ok(())
}

/// 安装桌面壳 companion 插件：
/// 1. 复制到 <DSH_HOME>/node_modules/@iyam/dsh-desktop-shell
/// 2. 在 <DSH_HOME>/profiles/node_modules/@iyam 建软链（profile 模块解析需要）
/// 3. 注册到 <DSH_HOME>/profiles/web/package.json 的 dsh.profile.bundles（幂等）
fn install_shell_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = dsh_node_modules(home)
        .join("@iyam")
        .join("dsh-desktop-shell");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制桌面壳插件失败: {}", e))?;
    ensure_profile_iyam_link(home, "dsh-desktop-shell", &dest)?;

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

/// 安装主题 UI 插件：复制到 <DSH_HOME>/node_modules/@iyam/dsh-rtui-ui，建 profile 软链，注册 bundles。
fn install_rtui_ui_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = dsh_node_modules(home).join("@iyam").join("dsh-rtui-ui");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制主题 UI 插件失败: {}", e))?;
    ensure_profile_iyam_link(home, "dsh-rtui-ui", &dest)?;

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

/// 安装文件查看插件：复制到 <DSH_HOME>/node_modules/@iyam/dsh-file-handler，建 profile 软链，注册 bundles。
fn install_file_handler_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = dsh_node_modules(home)
        .join("@iyam")
        .join("dsh-file-handler");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制文件查看插件失败: {}", e))?;
    ensure_profile_iyam_link(home, "dsh-file-handler", &dest)?;

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

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
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

/// 对话框 owner 补丁已生效的特征串（用于幂等判断）。
const PICKER_OWNER_PATCHED: &str =
    "const _h = process.env.DSH_DIALOG_OWNER_HWND; let _o = null; if (_h && /^[0-9]+$/.test(_h)) { const _n = Number(_h); if (_n > 0 && _n <= 0x7fffffff) { _o = _n; } }";

/// 给单个 `dsh-host-directory-picker-native/lib/worker.cjs` 打 owner 补丁（幂等）。
/// DSH 选目录对话框以主窗口 HWND 为 owner → 不占独立任务栏按钮、图标继承应用。
fn patch_picker_worker_file(worker: &PathBuf) {
    let content = match fs::read_to_string(worker) {
        Ok(c) => c,
        Err(_) => return,
    };
    if content.contains(PICKER_OWNER_PATCHED) {
        return;
    }
    const FROM: &str = "show: () => method(dialog, SLOT_SHOW, protoShow)(null),";
    const TO: &str = "show: () => { const _h = process.env.DSH_DIALOG_OWNER_HWND; let _o = null; if (_h && /^[0-9]+$/.test(_h)) { const _n = Number(_h); if (_n > 0 && _n <= 0x7fffffff) { _o = _n; } } return method(dialog, SLOT_SHOW, protoShow)(_o); },";
    const OLD_TO: &str = "show: () => method(dialog, SLOT_SHOW, protoShow)(process.env.DSH_DIALOG_OWNER_HWND ? Number(process.env.DSH_DIALOG_OWNER_HWND) : null),";
    const OLD_KOFFI: &str = "show: () => { const _h = process.env.DSH_DIALOG_OWNER_HWND; let _o = null; if (_h && /^[0-9]+$/.test(_h)) { const _n = Number(_h); if (_n > 0 && _n <= 0x7fffffff) { try { const _u = koffi.load('user32.dll'); const _isw = _u.func('__stdcall', 'IsWindow', 'int32', ['void *']); if (_isw(_n)) _o = _n; } catch (_e) { _o = null; } } } return method(dialog, SLOT_SHOW, protoShow)(_o); },";
    let base = content.replace(OLD_TO, FROM).replace(OLD_KOFFI, FROM);
    if base.contains(FROM) {
        let patched = base.replace(FROM, TO);
        if let Err(e) = fs::write(worker, patched) {
            log::warn!("写目录选择器 owner 补丁失败({}): {e}", worker.display());
        } else {
            log::info!("已为目录选择器打 owner 补丁: {}", worker.display());
        }
    } else {
        log::warn!(
            "worker.cjs 结构变化，未打对话框 owner 补丁（引擎升级后需同步；目录选择仍可用，仅对话框图标为 node）: {}",
            worker.display()
        );
    }
}

/// 为 native 目录选择器的对话框 worker 打 owner 补丁（幂等）。
///
/// DSH 因 npm 嵌套依赖，`dsh-host-directory-picker-native` 可能同时存在于顶层
/// `node_modules/@deepseek-ai/...` 与嵌套 `node_modules/<pkg>/node_modules/@deepseek-ai/...`。
/// Node 解析依赖时就近取嵌套副本——若只补顶层，运行时加载的仍是未补丁副本，对话框无
/// owner → 任务栏多出 node 图标。故递归遍历 `node_modules` 下所有副本一并补丁。
pub(crate) fn ensure_picker_owner_patch(home: &PathBuf) {
    let root = home.join("node_modules");
    if root.is_dir() {
        visit_picker_workers(&root, 0);
    }
}

/// 递归查找并补丁所有 `dsh-host-directory-picker-native/lib/worker.cjs`。
/// 命中终端包即补丁且不再下钻；跳过符号链接防环；限制深度防极端嵌套。
fn visit_picker_workers(dir: &PathBuf, depth: u32) {
    if depth > 24 {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_symlink() {
            continue; // 跳过符号链接，避免环
        }
        if !ft.is_dir() {
            continue;
        }
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name == "dsh-host-directory-picker-native" {
            let worker = path.join("lib").join("worker.cjs");
            patch_picker_worker_file(&worker);
        } else {
            visit_picker_workers(&path, depth + 1);
        }
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

/// 校验本 app 托管安装的 dsh 入口是否真的能跑（`bin.js` 存在且给定 node 跑 `--version` 成功）。
/// 用于安装后自愈：部分镜像分发的 tarball 里 `bin.js` 是坏掉的 `#!/bin/sh` 自调用壳子，
/// npm install 仍会"成功"返回，但 dsh 根本起不来。此函数能捕获这类坏包，触发换源重装。
///
/// `node` 必须显式传入：安装目标 `home` 可能是备货目录（`<home>/.staging`），而托管 node
/// 只装在 `<home>/node` 下——若在此处用 `managed_node(home)` 推导，备货态会找不到 node
/// 而把好包误判为坏包（升级永远报"入口不可用"）。
pub(crate) fn dsh_entry_runs(home: &PathBuf, node: &PathBuf) -> bool {
    let bin_js = dsh_bin_js(home);
    if !bin_js.exists() {
        return false;
    }
    if !node.exists() {
        return false;
    }
    verify_dsh_entry(&bin_js, &Some(node.clone()))
}

/// 托管 node 所在目录（含 `node` 可执行）。dsh 入口（`home/bin/dsh` 软链到 .js，
/// shebang 为 `#!/usr/bin/env node`）与 koffi 等安装脚本都依赖 PATH 上的 `node`，
/// 而托管 node 默认不在系统 PATH 上。把此目录前置到 PATH 后，这些入口才能解析到 node。
pub(crate) fn managed_node_bin_dir(home: &PathBuf) -> Option<PathBuf> {
    managed_node(home).parent().map(|p| p.to_path_buf())
}

/// 平台 PATH 分隔符：类 Unix `:`，Windows `;`。拼接 PATH 时必须用它，
/// 否则 Windows 上 PATH 会被 `:` 拼坏（命令解析全部失败）。
pub(crate) fn path_separator() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

/// 在现有 PATH 前插入托管 node 目录（若托管 node 已就绪），让 dsh/原生脚本里的裸
/// `node` 可用。返回 None 表示无托管 node，调用方应照常不设置 PATH（沿用父进程环境）。
pub(crate) fn prepend_managed_node_path(home: &PathBuf) -> Option<String> {
    let bin_dir = managed_node_bin_dir(home)?;
    if !bin_dir.exists() {
        return None;
    }
    let mut new_path = bin_dir.to_string_lossy().to_string();
    if let Ok(existing) = std::env::var("PATH") {
        if !existing.is_empty() {
            new_path.push_str(path_separator());
            new_path.push_str(&existing);
        }
    }
    Some(new_path)
}

/// 前端弹窗后由用户确认安装 dshmarket 插件市场时调用。
/// 幂等：已装则跳过；失败回传错误（由前端展示），绝不阻断主流程。
#[tauri::command]
pub(crate) async fn install_dshmarket(app: tauri::AppHandle) -> Result<(), String> {
    crate::installer::ensure_dshmarket(&app).await;
    if dshmarket_installed(&dsh_home()) {
        Ok(())
    } else {
        Err("dshmarket 安装失败（请检查网络后重试）".into())
    }
}
