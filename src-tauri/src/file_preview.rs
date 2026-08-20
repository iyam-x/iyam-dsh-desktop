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

/// 写回文本/代码文件（预览面板编辑后保存）。覆盖原文件，不做备份。
#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("读取文件信息失败: {}", e))?;
    if !meta.is_file() {
        return Err("不是文件".to_string());
    }
    std::fs::write(&path, content).map_err(|e| format!("写入文件失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("iyam-dsh-test-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// 图片预览链路：read_file_data → data:image/png;base64,<b64> → 解码应还原原始字节。
    #[test]
    fn image_data_url_round_trip() {
        // 最小 PNG 头 + 一些字节，模拟真实图片文件
        let png: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
            0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        ];
        let path = temp_file("img.png", &png);
        let preview = read_file_data(path.to_string_lossy().into_owned()).expect("read_file_data 失败");
        assert_eq!(preview.size as usize, png.len());
        assert_eq!(preview.name, path.file_name().unwrap().to_string_lossy());

        let data_url = format!("data:image/png;base64,{}", preview.base64);
        let b64 = data_url.split(',').nth(1).expect("data url 格式错误");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("base64 解码失败");
        assert_eq!(decoded, png, "data URL 解码应与原始字节一致");
        std::fs::remove_file(&path).ok();
    }

    /// 文本预览链路：read_text_file 返回 UTF-8 全文（含中文）。
    #[test]
    fn text_file_content_round_trip() {
        let content = "你好，世界\nline2\n```rust\nlet x = 1;\n```\n";
        let path = temp_file("notes.md", content.as_bytes());
        let preview = read_text_file(path.to_string_lossy().into_owned()).expect("read_text_file 失败");
        assert_eq!(preview.content, content);
        assert_eq!(preview.name, path.file_name().unwrap().to_string_lossy());
        std::fs::remove_file(&path).ok();
    }

    /// 目录/不存在路径应报错而非 panic。
    #[test]
    fn missing_file_errors() {
        let path = std::env::temp_dir().join("iyam-dsh-no-such-file-xyz.png");
        assert!(read_file_data(path.to_string_lossy().into_owned()).is_err());
    }

    /// 写回链路：write_text_file → 再次 read_text_file 应得到相同内容（含中文）。
    #[test]
    fn write_then_read_round_trip() {
        let content = "fn main() {\n    println!(\"你好\");\n}\n";
        let path = temp_file("write.rs", b"old content");
        write_text_file(path.to_string_lossy().into_owned(), content.to_string())
            .expect("write_text_file 失败");
        let preview = read_text_file(path.to_string_lossy().into_owned()).expect("read_text_file 失败");
        assert_eq!(preview.content, content);
        std::fs::remove_file(&path).ok();
    }

    /// 写入不存在路径应报错而非 panic。
    #[test]
    fn write_missing_file_errors() {
        let path = std::env::temp_dir().join("iyam-dsh-no-such-file-xyz.txt");
        assert!(write_text_file(path.to_string_lossy().into_owned(), "x".to_string()).is_err());
    }
}
