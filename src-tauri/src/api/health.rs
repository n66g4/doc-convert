use axum::{extract::State, Json};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub schema: u32,
    pub service: &'static str,
    pub host_api_version: &'static str,
    pub pid: u32,
    pub started_at_unix_ms: u64,
    pub uptime_ms: u64,
    /// 诊断用：当前选用的 Python / Pandoc 路径（仅本机回环）
    pub python_executable: String,
    pub pandoc_executable: String,
    /// 数据根目录（任务、日志、plugins_extra、runtime 等，架构 §10.1）
    pub data_root: String,
    /// 日志目录（通常为 data_root/logs）
    pub logs_directory: String,
    /// 本机 HTTP 监听端口（与 runtime/core.json 一致）
    pub bind_port: u16,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let uptime_ms = state.started_at.elapsed().as_millis() as u64;

    Json(HealthResponse {
        status: "ok",
        schema: 1,
        service: "docconvert-core",
        host_api_version: "1",
        pid: std::process::id(),
        started_at_unix_ms: state.started_at_unix_ms,
        uptime_ms,
        python_executable: state.config.python_executable.display().to_string(),
        pandoc_executable: state.config.pandoc_executable.display().to_string(),
        data_root: state.config.data_root.display().to_string(),
        logs_directory: state.config.logs_dir().display().to_string(),
        bind_port: state.port,
    })
}
