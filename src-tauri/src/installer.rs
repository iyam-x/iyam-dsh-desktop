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

pub(crate) fn dsh_home() -> PathBuf {
    DSH_HOME.get().cloned().unwrap_or_else(|| {
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
