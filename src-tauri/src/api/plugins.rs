use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::infra::AppError;
use crate::plugin_host::PluginMeta;
use crate::AppState;

#[derive(Serialize)]
pub struct PluginListResponse {
    pub plugins: Vec<PluginView>,
}

#[derive(Serialize)]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub enabled: bool,
    pub supported_formats: SupportedFormats,
    pub status: &'static str,
}

#[derive(Serialize)]
pub struct SupportedFormats {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

impl From<&PluginMeta> for PluginView {
    fn from(p: &PluginMeta) -> Self {
        let mut inputs: Vec<String> = p.capabilities.inputs.iter().cloned().collect();
        let mut outputs: Vec<String> = p.capabilities.outputs.iter().cloned().collect();
        inputs.sort();
        outputs.sort();
        PluginView {
            id: p.id.clone(),
            name: p.name.clone(),
            version: p.version.clone(),
            author: p.authors.join(", "),
            description: p.description.clone(),
            enabled: p.enabled,
            supported_formats: SupportedFormats {
                input: inputs,
                output: outputs,
            },
            status: if p.enabled { "active" } else { "inactive" },
        }
    }
}

pub async fn list_plugins(State(state): State<AppState>) -> Json<PluginListResponse> {
    let registry = state.plugin_registry.read().await;
    let plugins: Vec<PluginView> = registry.all_plugins().map(PluginView::from).collect();
    Json(PluginListResponse { plugins })
}

/// 自检插件：`depth=smoke`（默认）为轻量自检；`depth=deep` 为深度自检（最小样例走与任务相同的 worker 路径）。
#[derive(Deserialize)]
pub struct TestPluginQuery {
    #[serde(default = "default_test_depth")]
    depth: String,
}

fn default_test_depth() -> String {
    "smoke".to_string()
}

pub async fn test_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Query(q): Query<TestPluginQuery>,
) -> Result<Json<crate::plugin_host::PluginSmokeTestResult>, AppError> {
    let registry = state.plugin_registry.read().await;
    let meta = registry
        .get_plugin(&plugin_id)
        .cloned()
        .ok_or(AppError::PluginNotFound {
            plugin_id: plugin_id.clone(),
        })?;
    drop(registry);

    let depth = match q.depth.to_ascii_lowercase().as_str() {
        "deep" => crate::plugin_host::PluginTestDepth::Deep,
        _ => crate::plugin_host::PluginTestDepth::Smoke,
    };

    let result = crate::plugin_host::run_plugin_test(&state, &meta, depth).await;
    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct SetEnabledBody {
    pub enabled: bool,
}

pub async fn set_plugin_enabled(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(body): Json<SetEnabledBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut registry = state.plugin_registry.write().await;
    if registry.set_enabled(&plugin_id, body.enabled) {
        tracing::info!(plugin_id = %plugin_id, enabled = body.enabled, "Plugin state changed");
        Ok(Json(serde_json::json!({
            "plugin_id": plugin_id,
            "enabled": body.enabled
        })))
    } else {
        Err(AppError::PluginNotFound { plugin_id })
    }
}

/// `GET /api/v1/tools/status`：供 UI 与支持诊断（M5 运行时快照）
#[derive(Serialize)]
pub struct ToolsStatusResponse {
    pub platform: &'static str,
    pub arch: &'static str,
    pub host_api_version: &'static str,
    pub auto_update_enabled: bool,
    pub core: ToolsStatusCoreSection,
    pub plugins: Vec<PluginToolStatus>,
}

#[derive(Serialize)]
pub struct ToolsStatusCoreSection {
    pub bind_port: u16,
    pub max_concurrent_tasks: usize,
    pub task_result_ttl_secs: u64,
    pub max_file_size_bytes: u64,
    pub data_root: String,
    pub plugins_extra_dir: String,
    pub runtime_dir: String,
}

#[derive(Serialize)]
pub struct PluginToolStatus {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub status: &'static str,
    pub last_checked: String,
}

pub async fn tools_status(State(state): State<AppState>) -> Json<ToolsStatusResponse> {
    let registry = state.plugin_registry.read().await;

    let now = chrono::Utc::now().to_rfc3339();
    let mut plugins: Vec<PluginToolStatus> = registry
        .all_plugins()
        .map(|p| PluginToolStatus {
            id: p.id.clone(),
            name: p.name.clone(),
            version: p.version.clone(),
            enabled: p.enabled,
            status: if p.enabled { "active" } else { "inactive" },
            last_checked: now.clone(),
        })
        .collect();
    plugins.sort_by(|a, b| a.id.cmp(&b.id));

    Json(ToolsStatusResponse {
        platform: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        host_api_version: "1",
        auto_update_enabled: false,
        core: ToolsStatusCoreSection {
            bind_port: state.port,
            max_concurrent_tasks: state.config.max_concurrent_tasks,
            task_result_ttl_secs: state.config.task_result_ttl_secs,
            max_file_size_bytes: state.config.max_file_size_bytes,
            data_root: state.config.data_root.display().to_string(),
            plugins_extra_dir: state.config.plugins_extra_dir().display().to_string(),
            runtime_dir: state.config.runtime_dir().display().to_string(),
        },
        plugins,
    })
}
