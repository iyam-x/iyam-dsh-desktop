//! 运行时下载 + 备货升级模块。
//!
//! 取代 build 期把 dsh/node 打进安装包的做法：首次启动（系统无 dsh 时）由 app
//! 代用户安装 node(含 npm) + 安装 dsh 到 `~/.dsh`（与用户自行 `npm i -g` 一致）；
//! 之后 dsh 版本由 app 托管，支持后台备货升级（下载到 `.staging`，下次启动提升）
//! 与失败回滚。
//!
//! 跨平台与镜像策略见项目 PLAN；要点：
//! - Node 归档：npmmirror 二进制镜像优先 → nodejs.org 兜底；
//! - dsh 安装 registry：npmmirror 优先 → npmjs 兜底；
//! - Windows 解压用 `C:\Windows\System32\tar.exe`（bsdtar 支持 zip/tar.gz）；类 Unix 用 `tar`；
//! - macOS 下载的 node 需 `xattr -dr com.apple.quarantine` 清除 Gatekeeper 隔离标记。

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri::async_runtime;

/// 下载/解压阶段进度事件
#[derive(Debug, Clone, Serialize)]
pub struct DshInstallProgress {
    pub stage: String,
    pub progress: f64,
}

/// Node 二进制镜像（按顺序回退）
const NODE_MIRRORS: &[&str] = &[
    "https://registry.npmmirror.com/-/binary/node",
    "https://nodejs.org/dist",
];
/// dsh 的 npm registry（按顺序回退）
const NPM_REGISTRIES: &[&str] = &[
    "https://registry.npmmirror.com",
    "https://registry.npmjs.org",
];

const NODE_VERSION: &str = "v24.19.0";

fn emit_progress(app: &AppHandle, stage: &str, progress: f64) {
    let _ = app.emit(
        "dsh-install-progress",
        DshInstallProgress {
            stage: stage.to_string(),
            progress,
        },
    );
}

fn node_archive(target: &str) -> (&str, &str, &str) {
    match target {
        "darwin-arm64" => ("darwin", "arm64", "tar.gz"),
        "darwin-x64" => ("darwin", "x64", "tar.gz"),
        "win32-x64" => ("win", "x64", "zip"),
        "win32-arm64" => ("win", "arm64", "zip"),
        "linux-x64" => ("linux", "x64", "tar.gz"),
        "linux-arm64" => ("linux", "arm64", "tar.gz"),
        _ => ("linux", "x64", "tar.gz"),
    }
}

/// 查询 registry 上 @deepseek-ai/dsh 的最新版本（镜像回退）。
pub(crate) async fn latest_dsh_version() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    for reg in NPM_REGISTRIES {
        let url = format!("{}/@deepseek-ai/dsh/latest", reg);
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(v) = json["version"].as_str() {
                    return Ok(v.to_string());
                }
            }
        }
    }
    Err("无法获取 DSH 最新版本".into())
}

/// 首次安装：确保 node 存在，以全局方式把 dsh 装到 home（落在 ~/.dsh 同树，npm 才采纳
/// 该 prefix）。装完即全局布局（`home/node_modules/@deepseek-ai/dsh`），无需平铺搬运，
/// 与手动 `npm i -g` 效果一致。返回 node 路径。
pub async fn bootstrap_dsh(app: &AppHandle, home: &PathBuf) -> Result<PathBuf, String> {
    let node = ensure_node(app, home).await?;
    emit_progress(app, "installing-dsh", 0.6);
    let version = latest_dsh_version().await?;
    log::info!("安装 dsh {} (全局) 到 {:?}", version, home);
    install_dsh_to_tmp(app, &node, &version, home).await?;
    emit_progress(app, "done", 1.0);
    Ok(node)
}

/// 升级备货：以全局方式把目标版本 dsh 装到 `~/.dsh/.staging`（全局布局），写 `.update.json`。
pub async fn stage_update(app: &AppHandle, home: &PathBuf, target_version: &str) -> Result<(), String> {
    let node = ensure_node(app, home).await?;
    emit_progress(app, "staging-download", 0.1);
    let staging = home.join(".staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| format!("创建 staging 失败: {}", e))?;
    install_dsh_to_tmp(app, &node, target_version, &staging).await?;
    emit_progress(app, "staging-deploy", 0.8);
    // staging 已是全局布局（staging/node_modules/@deepseek-ai/dsh），无需平铺。
    let update = serde_json::json!({
        "staged_version": target_version,
        "status": "ready",
    });
    fs::write(
        home.join(".update.json"),
        serde_json::to_string_pretty(&update).unwrap(),
    )
    .map_err(|e| format!("写升级状态失败: {}", e))?;
    emit_progress(app, "staging-ready", 1.0);
    let _ = app.emit("dsh-staged-ready", target_version.to_string());
    Ok(())
}

/// 启动早期调用：若 `.update.json` 标记有已备货的新版本且高于当前，提升到正式目录。
/// 返回是否执行了提升。
pub fn apply_staged_if_ready(home: &PathBuf) -> bool {
    let update_path = home.join(".update.json");
    if !update_path.exists() {
        return false;
    }
    let content = match fs::read_to_string(&update_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if v["status"].as_str() != Some("ready") {
        return false;
    }
    let staged = match v["staged_version"].as_str() {
        Some(s) => s.to_string(),
        None => return false,
    };
    let cur = package_version(home);
    if let Some(c) = &cur {
        if !is_newer(&staged, c) {
            let _ = fs::remove_file(&update_path);
            return false;
        }
    }

    log::info!("提升备货版本 {} → 正式目录", staged);
    // 全局布局下 dsh 核心包位于 home/node_modules/@deepseek-ai/dsh。
    let core = home.join("node_modules").join("@deepseek-ai").join("dsh");
    let staged_core = home
        .join(".staging")
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh");

    // 1. 备份当前 core 到 .backup
    let backup = home.join(".backup");
    let _ = fs::remove_dir_all(&backup);
    fs::create_dir_all(&backup).ok();
    let backup_core = backup
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh");
    if core.exists() {
        copy_dir_follow_symlinks(&core, &backup_core).ok();
    }

    // 2. 标记 applying（供启动成功清除 / 失败回滚判断）
    let applying = serde_json::json!({
        "status": "applying",
        "staged_version": staged,
        "from_version": cur,
    });
    let _ = fs::write(
        &update_path,
        serde_json::to_string_pretty(&applying).unwrap(),
    );

    // 3. 清旧 core，复制 staging core 到正式位置
    let _ = fs::remove_dir_all(&core);
    if let Err(e) = copy_dir_follow_symlinks(&staged_core, &core) {
        log::warn!("提升 staging 失败: {}", e);
        return false;
    }
    let _ = fs::remove_dir_all(&home.join(".staging"));

    // 4. 重建 dsh 启动器 wrapper
    let node = crate::installer::managed_node(home);
    let _ = crate::installer::create_wrappers(home, &node);
    log::info!("已提升 dsh 到 {}", staged);
    true
}

/// 启动成功后调用：清除 applying 标记（升级成功）。
pub fn clear_applying(home: &PathBuf) {
    let update_path = home.join(".update.json");
    if !update_path.exists() {
        return;
    }
    let content = match fs::read_to_string(&update_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    if let Ok(v) = serde_json::Value::from_str(&content) {
        if v["status"].as_str() == Some("applying") {
            let _ = fs::remove_file(&update_path);
        }
    }
}

/// 启动超时调用：若本次来自刚提升的版本（applying 状态），回滚到上一个可用版本并标记 bad。
/// 返回是否执行了回滚（供调用方决定是否 emit 失败事件）。
pub fn rollback_after_failure(home: &PathBuf) -> bool {
    let update_path = home.join(".update.json");
    if !update_path.exists() {
        return false;
    }
    let content = match fs::read_to_string(&update_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if v["status"].as_str() != Some("applying") {
        return false;
    }
    let bad = v["staged_version"].as_str().unwrap_or("").to_string();
    log::warn!("升级版本 {} 启动失败，回滚到上一个可用版本", bad);

    // 还原备份（全局布局：core 在 node_modules/@deepseek-ai/dsh）
    let backup = home.join(".backup");
    let core = home.join("node_modules").join("@deepseek-ai").join("dsh");
    let backup_core = backup
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh");
    let _ = fs::remove_dir_all(&core);
    if backup_core.exists() {
        let _ = copy_dir_follow_symlinks(&backup_core, &core);
    }
    let _ = fs::remove_dir_all(&backup);

    // 标记 bad，避免再次升级到该版本
    let failed = serde_json::json!({
        "status": "failed",
        "bad_version": bad,
    });
    let _ = fs::write(
        &update_path,
        serde_json::to_string_pretty(&failed).unwrap(),
    );
    log::warn!("已回滚；bad_version={}", bad);
    true
}

// ---------- 内部实现 ----------

async fn ensure_node(app: &AppHandle, home: &PathBuf) -> Result<PathBuf, String> {
    let node = crate::installer::managed_node(home);
    // 完整性校验：node 可执行 + 自带 npm 必须都在，否则视为残损（如解压被中断）
    // 重新下载，避免 "npm 未找到" 卡死。
    if node_is_complete(home) {
        return Ok(node);
    }
    download_node(app, home).await
}

/// 托管 node 是否完整：node 可执行文件存在且自带 npm-cli.js 存在。
fn node_is_complete(home: &PathBuf) -> bool {
    let node = crate::installer::managed_node(home);
    if !node.exists() {
        return false;
    }
    let npm_cli = node
        .parent()
        .map(|d| d.join("node_modules").join("npm").join("bin").join("npm-cli.js"))
        .unwrap_or_default();
    npm_cli.exists()
}

async fn download_node(app: &AppHandle, home: &PathBuf) -> Result<PathBuf, String> {
    let target = crate::installer::node_target();
    let (os, arch, ext) = node_archive(target);
    let ver = std::env::var("DSH_NODE_VERSION").unwrap_or_else(|_| NODE_VERSION.to_string());
    let archive_name = format!("node-{}-{}-{}.{}", ver, os, arch, ext);
    let extract = home.join("node").join(target);
    if extract.join(crate::installer::node_exe_name()).exists() {
        return Ok(crate::installer::managed_node(home));
    }
    fs::create_dir_all(home.join("node")).map_err(|e| format!("创建 node 目录失败: {}", e))?;

    let tmp_archive = std::env::temp_dir().join(format!("iyam-node-{}.{}", target, ext));
    let mut last_err: Option<String> = None;
    for mirror in NODE_MIRRORS {
        let url = format!("{}/{}/{}", mirror, ver, archive_name);
        emit_progress(app, "downloading-node", 0.2);
        log::info!("下载 node 归档: {}", url);
        match download_file(app, &url, &tmp_archive).await {
            Ok(_) => {
                last_err = None;
                break;
            }
            Err(e) => {
                log::warn!("node 镜像 {} 失败: {}", mirror, e);
                last_err = Some(e);
            }
        }
    }
    if let Some(e) = last_err {
        return Err(format!("下载 node 失败: {}", e));
    }

    // 解压到临时目录，再把 node-vX-... 整体移入 extract
    let extract_tmp = home.join("node").join(format!("._extract_{}", target));
    let _ = fs::remove_dir_all(&extract_tmp);
    fs::create_dir_all(&extract_tmp).map_err(|e| format!("创建解压目录失败: {}", e))?;
    extract_archive(&tmp_archive, &extract_tmp)?;
    let _ = fs::remove_file(&tmp_archive);

    // 找到 node-vX-... 子目录
    let node_dir_name = fs::read_dir(&extract_tmp)
        .map_err(|e| format!("读取解压结果失败: {}", e))?
        .filter_map(|e| e.ok())
        .find(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name());
    let node_dir_name = match node_dir_name {
        Some(n) => n,
        None => return Err("node 归档结构异常".into()),
    };
    let _ = fs::remove_dir_all(&extract);
    fs::rename(extract_tmp.join(&node_dir_name), &extract)
        .map_err(|e| format!("移动 node 失败: {}", e))?;
    let _ = fs::remove_dir_all(&extract_tmp);

    // 权限 / quarantine
    let node_exe = crate::installer::managed_node(home);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&node_exe, fs::Permissions::from_mode(0o755));
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("xattr")
            .args(["-dr", "com.apple.quarantine", &extract.to_string_lossy()])
            .output();
    }
    if !node_exe.exists() {
        return Err("node 可执行文件未找到".into());
    }
    Ok(node_exe)
}

fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let tar = if cfg!(windows) {
        "C:\\Windows\\System32\\tar.exe".to_string()
    } else {
        "tar".to_string()
    };
    let out = Command::new(&tar)
        .args(["-xf", &archive.to_string_lossy(), "-C", &dest.to_string_lossy()])
        .output()
        .map_err(|e| format!("解压命令失败: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "解压失败: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

async fn download_file(_app: &AppHandle, url: &str, dest: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    fs::write(dest, &bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// 用托管 node 的 npm 把 dsh 以全局方式装到 `prefix`（落在 ~/.dsh 同树）。
///
/// 注意：dsh 依赖树极大（~40 直接依赖，每层又嵌套）。实测 npm 局部 `--prefix` 安装
/// 解析极慢（15min 未完成），而全局 `-g --prefix` 走 hoisted 布局 ~1min 完成。
/// 为避免前端"永久转圈=卡死"，这里还：
/// 1. 用 `recv_timeout` 实现整体安装超时（超时报错而非挂死）；
/// 2. 实时读取 npm stdout，把解析/下载/安装阶段以 `dsh-install-progress` 事件透出；
/// 3. 镜像按 `NPM_REGISTRIES` 顺序回退（npmmirror 优先）。
async fn install_dsh_to_tmp(
    app: &AppHandle,
    node: &PathBuf,
    version: &str,
    prefix: &PathBuf,
) -> Result<(), String> {
    // npm-cli.js 路径：win 在 node_dir/node_modules/npm/bin，unix 在 node_dir/lib/node_modules/npm/bin
    let node_dir = if cfg!(windows) {
        node.parent().unwrap().to_path_buf()
    } else {
        node.parent().and_then(|p| p.parent()).unwrap().to_path_buf()
    };
    let npm_cli = if cfg!(windows) {
        node_dir
            .join("node_modules")
            .join("npm")
            .join("bin")
            .join("npm-cli.js")
    } else {
        node_dir
            .join("lib")
            .join("node_modules")
            .join("npm")
            .join("bin")
            .join("npm-cli.js")
    };
    if !npm_cli.exists() {
        return Err(format!("npm 未找到: {}", npm_cli.display()));
    }

    let node_c = node.clone();
    let npm_c = npm_cli.clone();
    let prefix_c = prefix.clone();
    let version_c = version.to_string();
    let app_c = app.clone();
    let install_res = async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut last_err: Option<String> = None;
        for registry in NPM_REGISTRIES {
            emit_progress(&app_c, "installing-dsh", 0.05);
            match run_npm_install(&app_c, &node_c, &npm_c, &prefix_c, &version_c, registry) {
                Ok(()) => {
                    last_err = None;
                    break;
                }
                Err(e) => {
                    log::warn!("npm install dsh 走 {} 失败: {}", registry, e);
                    last_err = Some(e);
                }
            }
        }
        match last_err {
            None => Ok(()),
            Some(e) => Err(format!("npm install dsh 失败: {}", e)),
        }
    })
    .await
    .map_err(|e| format!("npm 安装线程失败: {}", e))?;
    install_res?;

    // 后处理：剥 sourceMappingURL（规避 404 刷屏）。全局布局下不做 flatten——
    // npm 已 hoisted，且 flatten_node_modules 会破坏兄弟包的自引用软链。
    let dsh_pkg = prefix
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh");
    if !dsh_pkg.exists() {
        return Err("dsh 安装产物结构异常".into());
    }
    emit_progress(app, "finalizing", 0.95);
    strip_source_mapping_urls(&dsh_pkg);
    Ok(())
}

/// 单次 npm install 尝试：piped stdout 实时发进度，整体 `DSH_INSTALL_TIMEOUT` 超时则杀整棵进程树。
fn run_npm_install(
    app: &AppHandle,
    node: &PathBuf,
    npm_cli: &PathBuf,
    prefix: &PathBuf,
    version: &str,
    registry: &str,
) -> Result<(), String> {
    const DSH_INSTALL_TIMEOUT: u64 = 15 * 60; // 15 分钟整体超时，避免卡死
    // 用全局安装（-g --prefix）而非局部 --prefix：dsh 依赖树极大，npm 局部安装解析
    // 极慢（实测 15min 未完成），全局安装走 hoisted 布局秒级完成（实测 ~1min）。
    // prefix 落在 ~/.dsh 同树（首次=home，升级=home/.staging），npm 才会采纳该 prefix。
    let mut cmd = Command::new(node);
    cmd.arg(npm_cli)
        .arg("install")
        .arg("-g")
        .arg(format!("@deepseek-ai/dsh@{}", version))
        .arg("--prefix")
        .arg(prefix)
        .arg("--no-save")
        .arg("--no-audit")
        .arg("--no-fund")
        .arg("--dangerously-allow-all-scripts")
        .arg("--loglevel")
        .arg("http")
        .arg("--registry")
        .arg(registry)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("启动 npm 失败: {}", e))?;
    let pid = child.id();

    // 读 stdout 发进度（npm --loglevel=http 会打印 reify 阶段与每个包）
    let app_p = app.clone();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut saw_reify = false;
        for line in reader.lines().map_while(Result::ok) {
            if line.contains("reify") {
                saw_reify = true;
            }
            // 进入 reify=下载/链接阶段后，进度从 0.1 爬到 0.9（粗略，按行数不可靠，仅作体感）
            if saw_reify {
                emit_progress(&app_p, "downloading-deps", 0.5);
            } else {
                emit_progress(&app_p, "resolving-deps", 0.2);
            }
        }
    });

    // 同时排空 stderr，避免 npm 写满管道缓冲区导致进程阻塞挂死（经典 pipe deadlock）。
    let stderr = child.stderr.take().unwrap();
    let err_reader = std::thread::spawn(move || {
        let mut r = BufReader::new(stderr);
        let mut buf = String::new();
        let _ = r.read_to_string(&mut buf);
    });

    // 用 channel + recv_timeout 实现整体超时，而非无限等待 child.wait()
    let (tx, rx) = std::sync::mpsc::channel::<Option<std::process::ExitStatus>>();
    let waiter = std::thread::spawn(move || {
        let _ = tx.send(child.wait().ok());
    });

    match rx.recv_timeout(std::time::Duration::from_secs(DSH_INSTALL_TIMEOUT)) {
        Ok(Some(status)) => {
            let _ = reader.join();
            let _ = err_reader.join();
            let _ = waiter.join();
            if status.success() {
                Ok(())
            } else {
                Err(format!("npm 退出码非零 (registry={})", registry))
            }
        }
        Ok(None) => {
            let _ = reader.join();
            let _ = err_reader.join();
            let _ = waiter.join();
            Err(format!("无法等待 npm 进程 (registry={})", registry))
        }
        Err(_) => {
            // 超时或 channel 断开：杀整棵 npm 进程树（含其派生子进程），避免残留 + 真正终止挂死
            kill_process_tree(pid);
            let _ = reader.join();
            let _ = err_reader.join();
            let _ = waiter.join();
            Err(format!(
                "npm install 超时（>{}s，registry={}）",
                DSH_INSTALL_TIMEOUT, registry
            ))
        }
    }
}

/// 跨平台杀整棵进程树（npm 会派生子进程，仅 kill 直接子进程会留孤儿）。
fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output();
    }
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

fn copy_dir_follow_symlinks(src: &Path, dst: &Path) -> std::io::Result<()> {
    let mut visited = std::collections::HashSet::new();
    copy_dir_follow_symlinks_impl(src, dst, &mut visited)
}

/// 跟随软链复制目录。npm 包内常见自引用软链（如
/// `node_modules/@deepseek-ai/dsh -> ..`），若直接递归会无限循环。
/// 用 visited 记录已处理的规范路径，遇到指向已复制子树（含自身）的软链直接跳过。
fn copy_dir_follow_symlinks_impl(
    src: &Path,
    dst: &Path,
    visited: &mut std::collections::HashSet<std::path::PathBuf>,
) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    // 记录当前源目录的规范路径，避免循环
    if let Ok(canon) = src.canonicalize() {
        if !visited.insert(canon) {
            return Ok(());
        }
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let name = entry.file_name();
        let dst_path = dst.join(&name);
        if ty.is_symlink() {
            // 解析软链真实目标后按目标类型复制
            if let Ok(target) = entry.path().canonicalize() {
                if target.is_dir() {
                    // 指向已处理子树的软链（自引用）跳过，避免循环
                    if visited.contains(&target) {
                        continue;
                    }
                    copy_dir_follow_symlinks_impl(&target, &dst_path, visited)?;
                } else {
                    fs::copy(&target, &dst_path)?;
                }
            }
            // 失效软链跳过
        } else if ty.is_dir() {
            copy_dir_follow_symlinks_impl(&entry.path(), &dst_path, visited)?;
        } else {
            fs::copy(&entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

fn package_version(dir: &Path) -> Option<String> {
    let content = fs::read_to_string(dir.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("version").and_then(|x| x.as_str()).map(|s| s.to_string())
}

/// 语义版本比较：a 是否比 b 更新。
fn is_newer(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> (Vec<u64>, Vec<String>) {
        let mut nums = Vec::new();
        let mut pre = Vec::new();
        let mut cur = String::new();
        let mut in_pre = false;
        for c in v.trim_start_matches('v').chars() {
            if c == '-' {
                in_pre = true;
                if !cur.is_empty() {
                    nums.push(cur.parse().unwrap_or(0));
                    cur.clear();
                }
            } else if c == '.' {
                if !cur.is_empty() {
                    nums.push(cur.parse().unwrap_or(0));
                    cur.clear();
                }
            } else if c.is_ascii_digit() && !in_pre {
                cur.push(c);
            } else if in_pre {
                if c == '.' {
                    pre.push(cur.clone());
                    cur.clear();
                } else {
                    cur.push(c);
                }
            }
        }
        if !cur.is_empty() {
            if in_pre {
                pre.push(cur);
            } else {
                nums.push(cur.parse().unwrap_or(0));
            }
        }
        (nums, pre)
    }
    let (an, ap) = parse(a);
    let (bn, bp) = parse(b);
    for i in 0..3 {
        let av = an.get(i).copied().unwrap_or(0);
        let bv = bn.get(i).copied().unwrap_or(0);
        if av != bv {
            return av > bv;
        }
    }
    // 主版本相同，比较 prerelease
    match (ap.is_empty(), bp.is_empty()) {
        (true, false) => true,  // a 正式版 > b prerelease
        (false, true) => false,
        (false, false) => ap > bp,
        (true, true) => false,
    }
}

fn strip_source_mapping_urls(root: &Path) {
    if let Ok(entries) = fs::read_dir(root) {
        for e in entries.flatten() {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                strip_source_mapping_urls(&p);
            } else if p.extension().map(|x| x == "js").unwrap_or(false) {
                if let Ok(src) = fs::read_to_string(&p) {
                    // 仅删除以 `//# sourceMappingURL=...` 开头的注释行（避免 404 刷屏），
                    // 不影响其余代码。
                    let cleaned: String = src
                        .lines()
                        .filter(|line| {
                            !(line.trim_start().starts_with("//#")
                                && line.contains("sourceMappingURL"))
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if cleaned != src {
                        let _ = fs::write(&p, cleaned);
                    }
                }
            }
        }
    }
}

