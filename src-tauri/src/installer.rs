use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

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
    Error(String),
}

/// Node 平台目录名，与 scripts/fetch-node.mjs 的 TARGETS key 保持一致
fn node_target() -> &'static str {
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

fn node_exe_name() -> &'static str {
    if cfg!(windows) {
        "node.exe"
    } else {
        "node"
    }
}

/// 定位 bundle 内的内置 DSH 包目录（含 lib/bin.js）
pub(crate) fn bundled_dsh_home(app: &tauri::AppHandle) -> Option<PathBuf> {
    // 1. Production: <app>/Contents/Resources/bin/dsh-package
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = res_dir.join("bin").join("dsh-package");
        if candidate.join("lib").join("bin.js").exists() {
            return Some(candidate);
        }
    }
    // 2. Dev build: build.rs 复制到 exe 同级 target/{profile}/dsh-package
    if let Some(candidate) = exe_dir_candidate("dsh-package", |p| p.join("lib").join("bin.js").exists()) {
        return Some(candidate);
    }
    // 3. 源码树回退（cwd 可能是项目根或 src-tauri）
    if let Ok(cwd) = env::current_dir() {
        for candidate in [
            cwd.join("src-tauri").join("bin").join("dsh-package"),
            cwd.join("bin").join("dsh-package"),
        ] {
            if candidate.join("lib").join("bin.js").exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 定位 bundle 内的内置 Node 运行时二进制
pub(crate) fn bundled_node(app: &tauri::AppHandle) -> Option<PathBuf> {
    let node_dir = |base: PathBuf| base.join("node").join(node_target()).join(node_exe_name());

    // 1. Production: <app>/Contents/Resources/bin/node/<target>/node(.exe)
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = node_dir(res_dir.join("bin"));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    // 2. Dev build: build.rs 复制到 exe 同级 target/{profile}/node/<target>/node
    if let Ok(exe) = env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = node_dir(exe_dir.to_path_buf());
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    // 3. 源码树回退（cwd 可能是项目根或 src-tauri）
    if let Ok(cwd) = env::current_dir() {
        for base in [cwd.join("src-tauri").join("bin"), cwd.join("bin")] {
            let candidate = node_dir(base);
            if candidate.exists() {
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

/// 定位 bundle 内的桌面壳 companion 插件包（注入 DSH 布局 CSS）
pub(crate) fn bundled_shell_plugin(app: &tauri::AppHandle) -> Option<PathBuf> {
    // 1. Production: <app>/Contents/Resources/bin/dsh-shell-plugin
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = res_dir.join("bin").join("dsh-shell-plugin");
        if candidate.join("lib").join("client.js").exists() {
            return Some(candidate);
        }
    }
    // 2. Dev build: build.rs 复制到 exe 同级 target/{profile}/dsh-shell-plugin
    if let Some(candidate) = exe_dir_candidate("dsh-shell-plugin", |p| p.join("lib").join("client.js").exists()) {
        return Some(candidate);
    }
    // 3. 源码树回退（cwd 可能是项目根或 src-tauri）
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

/// 定位 bundle 内的主题 UI 插件包（注入 DSH web UI 主题 token + 设置面板）。
/// 与 dsh-shell-plugin 不同：client 入口在根目录 client.js（非 lib/client.js）。
pub(crate) fn bundled_rtui_ui_plugin(app: &tauri::AppHandle) -> Option<PathBuf> {
    // 1. Production: <app>/Contents/Resources/bin/dsh-rtui-ui
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = res_dir.join("bin").join("dsh-rtui-ui");
        if candidate.join("client.js").exists() && candidate.join("package.json").exists() {
            return Some(candidate);
        }
    }
    // 2. Dev build: build.rs 复制到 exe 同级 target/{profile}/dsh-rtui-ui
    if let Some(candidate) = exe_dir_candidate("dsh-rtui-ui", |p| p.join("client.js").exists() && p.join("package.json").exists()) {
        return Some(candidate);
    }
    // 3. 源码树回退（cwd 可能是项目根或 src-tauri）
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

/// 定位 bundle 内的文件查看插件包（包装 workspaces.openPath，转发文件点击给桌面壳预览）。
/// 与 dsh-rtui-ui 同构：client 入口在根目录 client.js。
pub(crate) fn bundled_file_handler_plugin(app: &tauri::AppHandle) -> Option<PathBuf> {
    // 1. Production: <app>/Contents/Resources/bin/dsh-file-handler
    if let Ok(res_dir) = app.path().resource_dir() {
        let candidate = res_dir.join("bin").join("dsh-file-handler");
        if candidate.join("client.js").exists() && candidate.join("package.json").exists() {
            return Some(candidate);
        }
    }
    // 2. Dev build: build.rs 复制到 exe 同级 target/{profile}/dsh-file-handler
    if let Some(candidate) =
        exe_dir_candidate("dsh-file-handler", |p| p.join("client.js").exists() && p.join("package.json").exists())
    {
        return Some(candidate);
    }
    // 3. 源码树回退（cwd 可能是项目根或 src-tauri）
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

pub(crate) fn dsh_home() -> PathBuf {    DSH_HOME.get().cloned().unwrap_or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".iyam-dsh")
    })
}

#[tauri::command]
pub async fn get_install_status(app: tauri::AppHandle) -> InstallStatus {
    let home = dsh_home();
    let bin = home.join("bin").join(if cfg!(windows) { "dsh.cmd" } else { "dsh" });

    // 以文件存在性判断（跨平台：Windows 下 .cmd 不能直接被 Command::new 执行）
    if bin.exists() && home.join("lib").join("bin.js").exists() {
        return InstallStatus::Installed;
    }

    // 内置包完整（DSH + Node 都在）→ 可离线安装
    if bundled_dsh_home(&app).is_some() && bundled_node(&app).is_some() {
        return InstallStatus::NotInstalled;
    }

    InstallStatus::Error("内置资源不完整（缺少 DSH 包或 Node 运行时），请重新安装应用。".to_string())
}

#[tauri::command]
pub async fn check_and_install(app: tauri::AppHandle) -> Result<InstallStatus, String> {
    let home = dsh_home();

    if get_install_status(app.clone()).await == InstallStatus::Installed {
        return Ok(InstallStatus::Installed);
    }

    let bundled = bundled_dsh_home(&app)
        .ok_or("内置 DSH 包未找到，请重新安装应用。")?;
    let node = bundled_node(&app).ok_or("内置 Node 运行时未找到，请重新安装应用。")?;

    log::info!("Installing DSH from bundle to: {:?}", home);

    // Copy bundled DSH to home
    copy_dir_all(&bundled, &home).map_err(|e| format!("复制 DSH 包失败: {}", e))?;

    // Create bin/dsh wrapper (unix sh + windows cmd), both pointing at bundled node
    create_wrappers(&home, &node)?;

    // 安装桌面壳 companion 插件（向 DSH web UI 注入布局 CSS，避让窗口控件）。
    // 失败不阻断安装，仅记录警告——布局避让属于体验层。
    if let Some(plugin) = bundled_shell_plugin(&app) {
        if let Err(e) = install_shell_plugin(&home, &plugin) {
            log::warn!("install shell plugin failed: {}", e);
        }
    }

    // 安装主题 UI 插件（向 DSH web UI 注入主题预设/控件）。失败不阻断安装，
    // 仅记录警告——主题属于体验层，缺省仍可用官方默认外观。
    if let Some(plugin) = bundled_rtui_ui_plugin(&app) {
        if let Err(e) = install_rtui_ui_plugin(&home, &plugin) {
            log::warn!("install rtui-ui plugin failed: {}", e);
        }
    }

    // 安装文件查看插件（包装 openPath，转发文件点击给桌面壳做内联预览）。失败不阻断安装。
    if let Some(plugin) = bundled_file_handler_plugin(&app) {
        if let Err(e) = install_file_handler_plugin(&home, &plugin) {
            log::warn!("install file-handler plugin failed: {}", e);
        }
    }

    DSH_HOME.set(home.clone()).ok();
    log::info!("DSH installed to: {:?}", home);

    Ok(InstallStatus::Installed)
}

/// 生成可手动执行的 dsh 命令入口（终端使用 + DSH 内部工具调用）。
/// unix 用 sh 脚本，Windows 用 .cmd，均指向 bundle 内的 node 绝对路径。
fn create_wrappers(home: &PathBuf, node: &PathBuf) -> Result<(), String> {
    let bin_js = home.join("lib").join("bin.js");
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

/// 读取 DSH 包根目录 package.json 的 version 字段（读不到返回 None）。
fn package_version(dir: &PathBuf) -> Option<String> {
    let content = fs::read_to_string(dir.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("version").and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// 每次启动校验内置 DSH 与 DSH_HOME 部署版本；不一致则干净重部署核心代码。
///
/// 背景：核心 dsh-package 只在首次安装时复制到 DSH_HOME，之后每次启动仅刷新三个
/// 自定义插件，导致 bundle 升级（如 rc.6→rc.8）后 DSH_HOME 仍是旧核心——新资源
/// （如剥掉 sourceMappingURL 的 client bundle）不生效。本函数对比版本并在不一致时
/// 清掉旧代码目录（config/lib/node_modules）重新复制。用户数据不在 DSH_HOME 树内；
/// bundle 不含 profiles/ 与 bin/，故已注册的插件配置和 bin/dsh wrapper 不受影响，
/// 随后调用的 refresh_*_plugin 会重新装入自定义插件。
pub(crate) fn refresh_dsh_core(app: &tauri::AppHandle) -> Result<(), String> {
    let home = dsh_home();
    let bundled = bundled_dsh_home(app).ok_or("内置 DSH 包未找到，请重新安装应用。")?;

    let bundled_ver = package_version(&bundled);
    let deployed_ver = package_version(&home);
    if deployed_ver.is_some() && deployed_ver == bundled_ver {
        return Ok(()); // 版本一致，无需重部署
    }

    log::info!("DSH 核心版本不一致（bundle={:?}, deployed={:?}），重新部署。", bundled_ver, deployed_ver);

    for dir in ["config", "lib", "node_modules"] {
        fs::remove_dir_all(home.join(dir)).ok();
    }
    fs::remove_file(home.join("package.json")).ok();

    copy_dir_all(&bundled, &home).map_err(|e| format!("复制 DSH 包失败: {}", e))?;
    if let Some(node) = bundled_node(app) {
        create_wrappers(&home, &node)?;
    }
    log::info!("DSH 核心已从 bundle 重新部署（{bundled_ver:?}）。");
    Ok(())
}

/// 每次启动刷新桌面壳插件（幂等）：把 bundle 内的插件覆盖安装到 DSH_HOME，
/// 并确保注册到 web profile 的 bundles。旧安装因此也能获得桌面壳更新
/// （布局 CSS、通知桥等）。
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
///    注意：dsh 的 initProfile 对已存在的 manifest 不做覆盖，因此在首次启动前
///    创建好 profiles/web/package.json，dsh 启动时就会采用我们的 bundles 列表。
fn install_shell_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = home.join("node_modules").join("@iyam").join("dsh-desktop-shell");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制桌面壳插件失败: {}", e))?;

    let profile_pkg = home.join("profiles").join("web").join("package.json");
    let mut v: serde_json::Value = if profile_pkg.exists() {
        let content = fs::read_to_string(&profile_pkg).map_err(|e| format!("读取 profile 配置失败: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("解析 profile 配置失败: {}", e))?
    } else {
        if let Some(parent) = profile_pkg.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建 profile 目录失败: {}", e))?;
        }
        serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": {
                "profile": {
                    "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"]
                }
            }
        })
    };

    let bundles = v["dsh"]["profile"]["bundles"]
        .as_array_mut()
        .ok_or("profile 配置缺少 dsh.profile.bundles")?;
    if !bundles.iter().any(|b| b.as_str() == Some("@iyam/dsh-desktop-shell")) {
        bundles.push(serde_json::Value::String("@iyam/dsh-desktop-shell".into()));
    }

    let out = serde_json::to_string_pretty(&v).map_err(|e| format!("序列化 profile 配置失败: {}", e))?;
    fs::write(&profile_pkg, out + "\n").map_err(|e| format!("写入 profile 配置失败: {}", e))?;

    Ok(())
}

/// 每次启动刷新主题 UI 插件（幂等）：把 bundle 内的插件覆盖安装到 DSH_HOME，
/// 并确保注册到 web profile 的 bundles。旧安装因此也能获得主题预设/控件的更新。
pub(crate) fn refresh_rtui_ui_plugin(app: &tauri::AppHandle) -> Result<(), String> {
    let home = dsh_home();
    if let Some(plugin) = bundled_rtui_ui_plugin(app) {
        install_rtui_ui_plugin(&home, &plugin)
    } else {
        Ok(())
    }
}

/// 安装主题 UI 插件：
/// 1. 复制到 <DSH_HOME>/node_modules/@iyam/dsh-rtui-ui
/// 2. 注册到 <DSH_HOME>/profiles/web/package.json 的 dsh.profile.bundles（幂等）
fn install_rtui_ui_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = home.join("node_modules").join("@iyam").join("dsh-rtui-ui");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制主题 UI 插件失败: {}", e))?;

    let profile_pkg = home.join("profiles").join("web").join("package.json");
    let mut v: serde_json::Value = if profile_pkg.exists() {
        let content = fs::read_to_string(&profile_pkg).map_err(|e| format!("读取 profile 配置失败: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("解析 profile 配置失败: {}", e))?
    } else {
        if let Some(parent) = profile_pkg.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建 profile 目录失败: {}", e))?;
        }
        serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": {
                "profile": {
                    "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"]
                }
            }
        })
    };

    let bundles = v["dsh"]["profile"]["bundles"]
        .as_array_mut()
        .ok_or("profile 配置缺少 dsh.profile.bundles")?;
    if !bundles.iter().any(|b| b.as_str() == Some("@iyam/dsh-rtui-ui")) {
        bundles.push(serde_json::Value::String("@iyam/dsh-rtui-ui".into()));
    }

    let out = serde_json::to_string_pretty(&v).map_err(|e| format!("序列化 profile 配置失败: {}", e))?;
    fs::write(&profile_pkg, out + "\n").map_err(|e| format!("写入 profile 配置失败: {}", e))?;

    Ok(())
}

/// 每次启动刷新文件查看插件（幂等）：把 bundle 内的插件覆盖安装到 DSH_HOME，
/// 并确保注册到 web profile 的 bundles。旧安装因此也能获得文件预览更新的能力。
pub(crate) fn refresh_file_handler_plugin(app: &tauri::AppHandle) -> Result<(), String> {
    let home = dsh_home();
    if let Some(plugin) = bundled_file_handler_plugin(app) {
        install_file_handler_plugin(&home, &plugin)
    } else {
        Ok(())
    }
}

/// 安装文件查看插件：
/// 1. 复制到 <DSH_HOME>/node_modules/@iyam/dsh-file-handler
/// 2. 注册到 <DSH_HOME>/profiles/web/package.json 的 dsh.profile.bundles（幂等）
fn install_file_handler_plugin(home: &PathBuf, plugin: &PathBuf) -> Result<(), String> {
    let dest = home.join("node_modules").join("@iyam").join("dsh-file-handler");
    copy_dir_all(plugin, &dest).map_err(|e| format!("复制文件查看插件失败: {}", e))?;

    let profile_pkg = home.join("profiles").join("web").join("package.json");
    let mut v: serde_json::Value = if profile_pkg.exists() {
        let content = fs::read_to_string(&profile_pkg).map_err(|e| format!("读取 profile 配置失败: {}", e))?;
        serde_json::from_str(&content).map_err(|e| format!("解析 profile 配置失败: {}", e))?
    } else {
        if let Some(parent) = profile_pkg.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建 profile 目录失败: {}", e))?;
        }
        serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": {
                "profile": {
                    "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"]
                }
            }
        })
    };

    let bundles = v["dsh"]["profile"]["bundles"]
        .as_array_mut()
        .ok_or("profile 配置缺少 dsh.profile.bundles")?;
    if !bundles.iter().any(|b| b.as_str() == Some("@iyam/dsh-file-handler")) {
        bundles.push(serde_json::Value::String("@iyam/dsh-file-handler".into()));
    }

    let out = serde_json::to_string_pretty(&v).map_err(|e| format!("序列化 profile 配置失败: {}", e))?;
    fs::write(&profile_pkg, out + "\n").map_err(|e| format!("写入 profile 配置失败: {}", e))?;

    Ok(())
}

/// 任务栏 AUMID 预加载脚本：让 DSH 子进程（目录选择对话框等）与主应用共享
/// AppUserModelID（ai.iyam.dsh），任务栏按钮并入主应用，避免单独弹出 node 图标。
/// 通过 `NODE_OPTIONS=--require=` 注入 DSH 进程树，脚本内任何异常都静默吞掉，
/// 绝不阻断 node 进程启动。
const TASKBAR_AUMID_PRELOAD: &str = r#"// iyam-dsh: 与主应用共享 AppUserModelID
// DSH 的 Win32 目录选择对话框由独立 node 子进程打开，默认以 node.exe 图标单独
// 占据一个任务栏按钮。设置与主应用相同的 AUMID 后，按钮并入主应用任务栏入口。
// 通过 NODE_OPTIONS=--require 注入；异常全部静默，避免影响 DSH 启动。
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

/// 幂等写入任务栏 AUMID 预加载脚本到 DSH_HOME（已存在则跳过）。
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

/// 为 native 目录选择器的对话框 worker 打 owner 补丁(幂等)。
///
/// 官方 `dsh-host-directory-picker-native` 的 worker 用 `Show(null)` 打开
/// IFileOpenDialog:无 owner 的模态对话框会在任务栏单列一个按钮，图标是 node.exe 的。
/// 把 worker 的 Show owner 改为读环境变量 `DSH_DIALOG_OWNER_HWND`，对话框即成为
/// 主窗口的 owned window → 不占任务栏、图标继承应用。
///
/// worker 位于 DSH_HOME 内，DSH 升级会还原；本函数每次启动幂等重打。
/// 目标字符串找不到时只警告不阻断(升级后结构变化)。
///
/// 注意:补丁是防御式的——owner 值无效(负数/0/非数字/超大值)时回退 Show(null)，
/// 保证目录选择器永远可用;只有有效句柄才传 owner，避免 IFileOpenDialog::Show
/// 返回 E_INVALIDARG(0x80070057)。
pub(crate) fn ensure_picker_owner_patch(home: &PathBuf) {
    let worker = home
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh-host-directory-picker-native")
        .join("lib")
        .join("worker.cjs");
    let content = match fs::read_to_string(&worker) {
        Ok(c) => c,
        Err(_) => return, // 包缺失/路径变化:交给引擎自身报错
    };
    // 若已是防御式补丁则跳过;否则(原始代码或旧的非防御补丁)重新打。
    if content.contains("const _h = process.env.DSH_DIALOG_OWNER_HWND") {
        return;
    }
    const FROM: &str = "show: () => method(dialog, SLOT_SHOW, protoShow)(null),";
    const TO: &str = "show: () => { const _h = process.env.DSH_DIALOG_OWNER_HWND; let _o = null; if (_h && /^[0-9]+$/.test(_h)) { const _n = Number(_h); if (_n > 0 && _n <= 0x7fffffff) { try { const _u = koffi.load('user32.dll'); const _isw = _u.func('__stdcall', 'IsWindow', 'int32', ['void *']); if (_isw(_n)) _o = _n; } catch (_e) { _o = null; } } } return method(dialog, SLOT_SHOW, protoShow)(_o); },";
    // 旧的非防御式补丁还原为原始形态，再统一打防御式补丁
    const OLD_TO: &str = "show: () => method(dialog, SLOT_SHOW, protoShow)(process.env.DSH_DIALOG_OWNER_HWND ? Number(process.env.DSH_DIALOG_OWNER_HWND) : null),";
    let base = content.replace(OLD_TO, FROM);
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
