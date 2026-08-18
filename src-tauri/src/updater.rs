use std::fs;

use serde::Serialize;

use crate::installer::dsh_home;

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub installed: String,
    pub latest: String,
    pub has_update: bool,
}

#[tauri::command]
pub async fn check_for_update() -> Result<UpdateInfo, String> {
    let installed = get_installed_version().await?;
    let latest = get_latest_version().await?;

    let has_update = match (semver::Version::parse(&latest), semver::Version::parse(&installed)) {
        (Ok(v), Ok(installed_v)) => v > installed_v,
        (Ok(_v), Err(_)) => true,
        _ => false,
    };

    Ok(UpdateInfo {
        installed,
        latest,
        has_update,
    })
}

async fn get_installed_version() -> Result<String, String> {
    let home = dsh_home();
    let pkg = home.join("package.json");

    if !pkg.exists() {
        return Ok("unknown".to_string());
    }

    let content =
        fs::read_to_string(&pkg).map_err(|e| format!("读取 package.json 失败: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 package.json 失败: {}", e))?;

    Ok(json["version"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string()))
}

async fn get_latest_version() -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://registry.npmjs.org/@deepseek-ai/dsh/latest")
        .send()
        .await
        .map_err(|e| format!("无法获取最新版本: {}", e))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析版本信息失败: {}", e))?;

    body["dist-tags"]["latest"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "无法解析最新版本号".to_string())
}
