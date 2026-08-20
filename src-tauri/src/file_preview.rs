//! 文件内联预览命令：读取 DSH 会话产出/编辑的文件，供宿主预览层渲染。
//!
//! DSH web iframe 跨域调用不了 Tauri 命令，只能通过 postMessage 把文件路径交给宿主；
//! 这里只暴露给宿主自身页面（我们的 React UI），按需读取文本或二进制。
//! 按需求不设大小上限，完整读取。

use base64::Engine;
use serde::Serialize;

fn file_name_of(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPreview {
    name: String,
    size: u64,
    content: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataPreview {
    name: String,
    size: u64,
    base64: String,
}

/// 读取文本/代码文件全文（按需求不设上限；二进制内容以 lossy 字符串返回）。
#[tauri::command]
pub fn read_text_file(path: String) -> Result<TextPreview, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("读取文件信息失败: {}", e))?;
    if !meta.is_file() {
        return Err("不是文件".to_string());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败: {}", e))?;
    Ok(TextPreview {
        name: file_name_of(&path),
        size: meta.len(),
        content: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

/// 读取二进制文件（图片/音视频），返回 base64 供宿主拼 data URL。
#[tauri::command]
pub fn read_file_data(path: String) -> Result<DataPreview, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("读取文件信息失败: {}", e))?;
    if !meta.is_file() {
        return Err("不是文件".to_string());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("读取文件失败: {}", e))?;
    Ok(DataPreview {
        name: file_name_of(&path),
        size: meta.len(),
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}
