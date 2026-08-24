use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::installer::{dsh_core_dir, dsh_home};

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub installed: String,
    pub latest: String,
    pub has_update: bool,
    /// 该 dsh 是否由本 app 托管（app 帮装的才由 app 升级；用户自管的不动）。
    pub managed: bool,
}

#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let installed = get_installed_version().await?;
    let latest = crate::downloader::latest_dsh_version().await?;
    let has_update = is_newer(&latest, &installed);
    let managed = crate::installer::is_managed();

    // 托管态且有新版本：后台自动备货（24h 节流，不阻塞返回）
    if managed && has_update {
        maybe_auto_stage(&app, &latest).await;
    }

    Ok(UpdateInfo {
        installed,
        latest,
        has_update,
        managed,
    })
}

/// 手动触发检查并更新（前端"检查并更新"按钮）：立即备货到下次启动生效。
#[tauri::command]
pub async fn trigger_dsh_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let latest = crate::downloader::latest_dsh_version().await?;
    let installed = get_installed_version().await?;
    // 版本未变则不下备货、不重新下载（仅返回当前状态，前端据此提示"已是最新版本"）。
    let a = latest.trim_start_matches('v');
    let b = installed.trim_start_matches('v');
    if crate::installer::is_managed() && a != b {
        crate::downloader::stage_update(&app, &dsh_home(), &latest)
            .await
            .map_err(|e| format!("备货失败: {}", e))?;
    }
    let has_update = is_newer(&latest, &installed);
    Ok(UpdateInfo {
        installed,
        latest: latest.clone(),
        has_update,
        managed: crate::installer::is_managed(),
    })
}

/// 托管态下，超过 24h 未自动检查则后台发起一次备货（fire-and-forget）。
async fn maybe_auto_stage(app: &tauri::AppHandle, latest: &str) {
    let home = dsh_home();
    let stamp = home.join(".last-update-check");
    if let Ok(c) = fs::read_to_string(&stamp) {
        if let Ok(t) = c.trim().parse::<u64>() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now.saturating_sub(t) < 24 * 3600 {
                return; // 24h 内已查过
            }
        }
    }
    let _ = fs::write(
        &stamp,
        format!(
            "{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ),
    );

    let app_c = app.clone();
    let latest_c = latest.to_string();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::downloader::stage_update(&app_c, &dsh_home(), &latest_c).await {
            log::warn!("自动备货失败: {}", e);
        }
    });
}

async fn get_installed_version() -> Result<String, String> {
    let home = dsh_home();
    // 读 dsh 核心包自身的 package.json（npm 全局安装布局：<home>/lib/node_modules/@deepseek-ai/dsh/
    // 类 Unix；<home>/node_modules/@deepseek-ai/dsh/ Windows）。不要读 home/package.json——
    // 那个文件不存在，会一直返回 "unknown"。
    let pkg = dsh_core_dir(&home).join("package.json");

    if !pkg.exists() {
        return Ok("unknown".to_string());
    }

    let content =
        fs::read_to_string(&pkg).map_err(|e| format!("读取 dsh package.json 失败: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 dsh package.json 失败: {}", e))?;

    Ok(json["version"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string()))
}

fn is_newer(latest: &str, installed: &str) -> bool {
    match (semver::Version::parse(latest), semver::Version::parse(installed)) {
        (Ok(l), Ok(i)) => l > i,
        (Ok(_), Err(_)) => true, // 本地版本无法解析（如 unknown）→ 视为有更新
        _ => false,
    }
}
