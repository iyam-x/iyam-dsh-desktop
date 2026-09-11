//! 桌面壳（本 app）自身的更新检查。
//!
//! 与 DSH 的「备货式」升级不同，本 app 是原生安装包（Windows NSIS / macOS dmg /
//! Linux AppImage），运行中无法覆盖自身，故只做**检查 + 引导下载**：发现新版本时
//! 提示用户并打开 GitHub Releases 页，由用户手动安装。
//!
//! 版本来源是发布 tag（`.github/workflows/build.yml` 在 `v*` 标签时构建并上传资产），
//! 因此 `releases/latest` 稳定可用；当前版本取编译期写入的 `CARGO_PKG_VERSION`，
//! 与发布 tag 同源。

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

const REPO: &str = "iyam-x/iyam-dsh-desktop";
/// 自动检查节流：24h 内查过就不再查（手动检查不受限）。
const CHECK_INTERVAL_SECS: u64 = 24 * 3600;

#[derive(Serialize, Clone)]
pub struct AppUpdateInfo {
    /// 当前运行的 app 版本。
    pub current: String,
    /// Release 上的最新版本；被节流跳过时为空。
    pub latest: String,
    pub has_update: bool,
    /// 新版本对应的 Release 页面；无更新时为空。
    pub url: String,
}

/// 检查本 app 是否有新版本。
/// `force=false` 走 24h 节流（启动时的自动检查），`force=true` 立即联网（用户手动点）。
#[tauri::command]
pub async fn check_app_update(force: bool) -> Result<AppUpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let stamp = crate::installer::dsh_home().join(".last-app-update-check");

    if !force && !check_due(&stamp) {
        return Ok(AppUpdateInfo {
            current,
            latest: String::new(),
            has_update: false,
            url: String::new(),
        });
    }

    let (latest, url) = latest_release().await?;
    // 查询成功才写节流标记：失败（离线 / 限流）不该让用户 24h 内失去自动检查的机会。
    let _ = fs::write(&stamp, now_secs().to_string());

    let has_update = crate::updater::is_newer(&latest, &current);
    Ok(AppUpdateInfo {
        current,
        latest,
        has_update,
        url: if has_update { url } else { String::new() },
    })
}

/// 查询 GitHub 上最新一个正式 Release，返回 (版本号, Release 页面地址)。
async fn latest_release() -> Result<(String, String), String> {
    // GitHub API 规范要求带 User-Agent（缺失时可能被拒 403），reqwest 默认不发，故显式指定。
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("iyam-dsh-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub 返回 HTTP {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let latest = json["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string();
    if latest.is_empty() {
        return Err("Release 缺少 tag_name".into());
    }
    Ok((latest, json["html_url"].as_str().unwrap_or("").to_string()))
}

/// 是否到了该检查的时间（无标记文件 / 内容不可解析 → 该查）。
fn check_due(stamp: &PathBuf) -> bool {
    match fs::read_to_string(stamp) {
        Ok(content) => !within_interval(&content, now_secs()),
        Err(_) => true,
    }
}

/// 上次检查时间（unix 秒）距今是否不足节流间隔。
/// 解析不了 → 视为该查；时间戳晚于当前时间（时钟回拨/被改）→ 视为该查。
fn within_interval(stamp_content: &str, now: u64) -> bool {
    match stamp_content.trim().parse::<u64>() {
        Ok(t) if t <= now => now - t < CHECK_INTERVAL_SECS,
        _ => false,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_interval_only_inside_window() {
        let now = 1_000_000_000u64;
        assert!(within_interval(&(now - 10).to_string(), now));
        assert!(within_interval(&(now - (CHECK_INTERVAL_SECS - 1)).to_string(), now));
        assert!(!within_interval(&(now - CHECK_INTERVAL_SECS).to_string(), now));
        assert!(!within_interval(&(now - CHECK_INTERVAL_SECS * 3).to_string(), now));
    }

    #[test]
    fn within_interval_treats_bad_or_future_stamp_as_due() {
        let now = 1_000_000_000u64;
        assert!(!within_interval("", now)); // 空文件
        assert!(!within_interval("not-a-number", now));
        assert!(!within_interval(&(now + 60).to_string(), now)); // 时钟回拨
        assert!(within_interval(" 123 \n", 1000)); // 带空白的有效时间戳（trim 后解析）
    }
}
