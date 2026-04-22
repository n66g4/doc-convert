//! Tauri 命令：大文件走 Rust 从磁盘流式读取后以 multipart 提交本机 Core，避免 WebView 整文件进内存。

use serde::Deserialize;
use tokio_util::io::ReaderStream;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertLocalBody {
    pub path: String,
    pub output_format: String,
    pub input_format: Option<String>,
    pub preferred_plugins: Option<String>,
    pub options: Option<String>,
}

/// 从本地路径提交 `POST /api/v1/convert`（`tokio::fs::File` + `ReaderStream` 流式上传，不经过 JS 堆）。
#[tauri::command]
pub async fn convert_submit_local_file(
    body: ConvertLocalBody,
) -> Result<serde_json::Value, String> {
    let path = std::path::PathBuf::from(body.path.trim());
    let path = std::fs::canonicalize(&path).map_err(|e| format!("无法访问文件: {}", e))?;
    if !path.is_file() {
        return Err("路径不是可读文件".into());
    }

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();

    let port: u16 = std::env::var("DOCCONVERT_BIND_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(17300);
    let url = format!("http://127.0.0.1:{}/api/v1/convert", port);

    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| format!("打开文件失败: {}", e))?;
    let len = file
        .metadata()
        .await
        .map_err(|e| format!("读取文件信息失败: {}", e))?
        .len();
    if len == 0 {
        return Err("本地文件大小为 0 字节，无法转换".into());
    }

    // 先挂 file，再挂文本字段，避免少数 multipart 实现对字段顺序敏感。
    let mut form = reqwest::multipart::Form::new();
    let stream = ReaderStream::new(file);
    let body_stream = reqwest::Body::wrap_stream(stream);
    let part = reqwest::multipart::Part::stream_with_length(body_stream, len)
        .file_name(file_name)
        .mime_str("application/octet-stream")
        .map_err(|e| e.to_string())?;
    form = form.part("file", part);
    form = form.text("output_format", body.output_format.clone());
    if let Some(s) = body.input_format {
        if !s.trim().is_empty() {
            form = form.text("input_format", s);
        }
    }
    if let Some(s) = body.preferred_plugins {
        if !s.trim().is_empty() {
            form = form.text("preferred_plugins", s);
        }
    }
    if let Some(s) = body.options {
        if !s.trim().is_empty() {
            form = form.text("options", s);
        }
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(7200))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("请求 Core 失败: {}", e))?;

    let status = resp.status();
    let body_text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body_text) {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or(&body_text);
            let code = v.get("error_code").and_then(|c| c.as_str()).unwrap_or("");
            if code.is_empty() {
                return Err(format!("HTTP {}: {}", status, msg));
            }
            return Err(format!("[{}] {}", code, msg));
        }
        return Err(format!("HTTP {}: {}", status, body_text));
    }

    serde_json::from_str(&body_text).map_err(|e| format!("解析响应 JSON: {}", e))
}
