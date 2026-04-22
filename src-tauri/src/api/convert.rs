use axum::{
    extract::{Multipart, Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::core::download_name::{
    content_disposition_attachment, derive_result_download_filename, unique_filename_in_dir,
};
use crate::core::{ConvertTask, TaskStatus};
use crate::infra::AppError;
use crate::AppState;

#[derive(Serialize)]
pub struct ConvertResponse {
    pub status: String,
    pub task_id: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_url: Option<String>,
}

pub async fn post_convert(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(axum::http::StatusCode, Json<ConvertResponse>), AppError> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut input_format: Option<String> = None;
    let mut output_format: Option<String> = None;
    let mut options: Option<String> = None;
    let mut preferred_plugins: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::InvalidOptions {
            message: e.to_string(),
        })?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                file_name = field.file_name().map(|s| s.to_string());
                let data = field.bytes().await.map_err(|e| AppError::InvalidOptions {
                    message: e.to_string(),
                })?;
                // 检查文件大小
                let limit = state.config.max_file_size_bytes;
                if data.len() as u64 > limit {
                    return Err(AppError::FileTooLarge {
                        size: data.len() as u64,
                        limit,
                    });
                }
                file_bytes = Some(data.to_vec());
            }
            "input_format" => {
                input_format = Some(field.text().await.map_err(|e| AppError::InvalidOptions {
                    message: e.to_string(),
                })?);
            }
            "output_format" => {
                output_format = Some(field.text().await.map_err(|e| AppError::InvalidOptions {
                    message: e.to_string(),
                })?);
            }
            "options" => {
                options = Some(field.text().await.map_err(|e| AppError::InvalidOptions {
                    message: e.to_string(),
                })?);
            }
            "preferred_plugins" => {
                preferred_plugins =
                    Some(field.text().await.map_err(|e| AppError::InvalidOptions {
                        message: e.to_string(),
                    })?);
            }
            _ => {}
        }
    }

    let file_bytes = file_bytes.ok_or_else(|| AppError::InvalidOptions {
        message: "Missing required field: file".into(),
    })?;
    if file_bytes.is_empty() {
        return Err(AppError::InvalidOptions {
            message:
                "上传文件为空（0 字节）。请确认所选路径可读且未损坏；桌面拖放请勿选到空占位文件。"
                    .into(),
        });
    }
    let output_format = output_format.ok_or_else(|| AppError::InvalidOptions {
        message: "Missing required field: output_format".into(),
    })?;

    // 自动检测 input_format（若未提供，从扩展名推断）
    let input_format = input_format.or_else(|| {
        file_name.as_ref().and_then(|n| {
            std::path::Path::new(n)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase())
        })
    });

    // 路由决策
    let registry = state.plugin_registry.read().await;
    let resolved = state.router.resolve(
        input_format.as_deref().unwrap_or("unknown"),
        &output_format,
        preferred_plugins.as_deref(),
        &registry,
    )?;
    drop(registry);

    let steps = resolved.steps;
    let single_hop_fallback_ids = resolved.single_hop_fallback_ids;

    // 创建任务（plugin_chain 必须写入 TaskManager，仅改局部 task 不会落库）
    let task = state
        .task_manager
        .create_task(output_format.clone(), input_format);

    let task_id = task.task_id.clone();

    let plugin_chain: Vec<crate::core::PluginInvocation> = steps
        .iter()
        .map(|s| {
            let version = state
                .plugin_registry
                .try_read()
                .ok()
                .and_then(|r| r.get_plugin(&s.plugin_id).map(|p| p.version.clone()))
                .unwrap_or_else(|| "unknown".to_string());
            crate::core::PluginInvocation {
                plugin_id: s.plugin_id.clone(),
                version,
            }
        })
        .collect();

    // 将文件落盘到 temp/<task_id>/
    let temp_task_dir = state.config.temp_dir().join(&task_id);
    std::fs::create_dir_all(&temp_task_dir)?;
    let input_ext = file_name
        .as_ref()
        .and_then(|n| std::path::Path::new(n).extension().and_then(|e| e.to_str()))
        .unwrap_or("bin");
    let input_path = temp_task_dir.join(format!("input.{}", input_ext));
    std::fs::write(&input_path, &file_bytes)?;

    let result_download_filename =
        derive_result_download_filename(file_name.as_deref(), &output_format);
    let result_disk_filename = result_download_filename.clone();

    // 保存任务：固化 plugin_chain、hint、下载名
    state.task_manager.update_task(&task_id, |t| {
        t.plugin_chain = plugin_chain.clone();
        t.input_filename_hint =
            crate::core::download_name::input_file_label_for_task(file_name.as_deref());
        t.result_download_filename = Some(result_download_filename);
    })?;

    // 提交异步执行
    let state_clone = state.clone();
    let task_id_clone = task_id.clone();
    let steps_clone = steps;
    let output_format_clone = output_format;
    let options_clone = options;
    tokio::spawn(async move {
        execute_task(
            state_clone,
            task_id_clone,
            input_path,
            steps_clone,
            single_hop_fallback_ids,
            output_format_clone,
            options_clone,
            result_disk_filename,
        )
        .await;
    });

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(ConvertResponse {
            status: "accepted".to_string(),
            task_id,
            message: "Task queued".to_string(),
            result_url: None,
        }),
    ))
}

async fn execute_task(
    state: AppState,
    task_id: String,
    input_path: std::path::PathBuf,
    steps: Vec<crate::core::router::RouteStep>,
    single_hop_fallback_ids: Vec<String>,
    _output_format: String,
    options_json: Option<String>,
    result_disk_filename: String,
) {
    let temp_dir = state.config.temp_dir().join(&task_id);
    let _temp_cleanup = scopeguard::guard(temp_dir.clone(), |p| {
        if let Err(e) = std::fs::remove_dir_all(&p) {
            tracing::warn!(path = %p.display(), error = %e, "remove task temp dir");
        } else {
            tracing::debug!(path = %p.display(), "removed task temp dir");
        }
    });

    // 申请并发 slot
    let _permit = match state.task_manager.acquire_slot().await {
        Ok(p) => p,
        Err(e) => {
            let _ = state
                .task_manager
                .update_task(&task_id, |t| t.set_failed(e));
            return;
        }
    };

    let _ = state
        .task_manager
        .update_task(&task_id, |t| t.set_processing());

    let total_steps = steps.len();
    let mut current_input = input_path.clone();

    if total_steps == 1 && !single_hop_fallback_ids.is_empty() {
        let step0 = &steps[0];
        let plugin_order: Vec<String> = std::iter::once(step0.plugin_id.clone())
            .chain(single_hop_fallback_ids.into_iter())
            .collect();
        let mut last_err: Option<AppError> = None;
        let mut succeeded = false;

        for (attempt, plugin_id) in plugin_order.iter().enumerate() {
            let _ = state
                .task_manager
                .update_task(&task_id, |t| t.set_progress(0));

            let step = crate::core::router::RouteStep {
                plugin_id: plugin_id.clone(),
                in_format: step0.in_format.clone(),
                out_format: step0.out_format.clone(),
                step_index: 0,
            };
            let out_path = temp_dir.join(format!("step_0_{}.{}", attempt, step.out_format));
            let _ = std::fs::remove_file(&out_path);

            let result = crate::workers::dispatch_worker(
                &state,
                &step,
                &current_input,
                &out_path,
                &temp_dir,
                options_json.as_deref(),
            )
            .await;

            match result {
                Ok(()) => {
                    let len = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
                    if len > 0 {
                        let registry = state.plugin_registry.read().await;
                        let version = registry
                            .get_plugin(plugin_id)
                            .map(|p| p.version.clone())
                            .unwrap_or_else(|| "unknown".to_string());
                        drop(registry);
                        let _ = state.task_manager.update_task(&task_id, |t| {
                            t.plugin_chain = vec![crate::core::PluginInvocation {
                                plugin_id: plugin_id.clone(),
                                version,
                            }];
                        });
                        current_input = out_path;
                        succeeded = true;
                        break;
                    }
                    tracing::warn!(
                        task_id = %task_id,
                        plugin_id = %plugin_id,
                        "Plugin produced empty output, trying fallback"
                    );
                    last_err = Some(AppError::PluginFailed {
                        plugin_id: plugin_id.clone(),
                        step_index: 0,
                        detail: "产出文件为空（0 字节）".to_string(),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %task_id,
                        plugin_id = %plugin_id,
                        error = %e,
                        "Plugin failed, trying fallback"
                    );
                    last_err = Some(e);
                }
            }
        }

        if !succeeded {
            let err = last_err.unwrap_or_else(|| AppError::PluginFailed {
                plugin_id: "convert_pipeline".to_string(),
                step_index: 0,
                detail: "所有候选插件均失败或产出空文件".to_string(),
            });
            let _ = state
                .task_manager
                .update_task(&task_id, |t| t.set_failed(err));
            return;
        }
    } else {
        for (i, step) in steps.iter().enumerate() {
            let progress = ((i as f32 / total_steps as f32) * 100.0) as u8;
            let _ = state
                .task_manager
                .update_task(&task_id, |t| t.set_progress(progress));

            let out_path = temp_dir.join(format!("step_{}.{}", i, step.out_format));

            let result = crate::workers::dispatch_worker(
                &state,
                step,
                &current_input,
                &out_path,
                &temp_dir,
                options_json.as_deref(),
            )
            .await;

            match result {
                Ok(_) => {
                    current_input = out_path;
                }
                Err(e) => {
                    tracing::error!(
                        task_id = %task_id,
                        plugin_id = %step.plugin_id,
                        step = i,
                        error = %e,
                        "Step failed"
                    );
                    let _ = state.task_manager.update_task(&task_id, |t| {
                        t.set_failed(AppError::PluginFailed {
                            plugin_id: step.plugin_id.clone(),
                            step_index: i,
                            detail: e.to_string(),
                        })
                    });
                    return;
                }
            }
        }
    }

    // 将最终结果移至 tasks/<task_id>/<派生文件名>（与下载名一致）；同名已存在则自动 `stem (n).ext`
    let tasks_dir = state.config.tasks_dir().join(&task_id);
    let _ = std::fs::create_dir_all(&tasks_dir);
    let final_disk_name = unique_filename_in_dir(&tasks_dir, &result_disk_filename);
    if final_disk_name != result_disk_filename {
        let _ = state.task_manager.update_task(&task_id, |t| {
            t.result_download_filename = Some(final_disk_name.clone());
        });
    }
    let result_path = tasks_dir.join(&final_disk_name);
    if let Err(e) = std::fs::rename(&current_input, &result_path) {
        let _ = state
            .task_manager
            .update_task(&task_id, |t| t.set_failed(AppError::Io(e)));
        return;
    }

    let out_len = match std::fs::metadata(&result_path) {
        Ok(m) => m.len(),
        Err(e) => {
            let _ = state
                .task_manager
                .update_task(&task_id, |t| t.set_failed(AppError::Io(e)));
            return;
        }
    };
    if out_len == 0 {
        let _ = std::fs::remove_file(&result_path);
        let _ = state.task_manager.update_task(&task_id, |t| {
            t.set_failed(AppError::PluginFailed {
                plugin_id: "convert_pipeline".to_string(),
                step_index: steps.len().saturating_sub(1),
                detail: "转换结果文件为空（0 字节）。若源文件本身非空，通常是当前插件未解析出正文（例如扫描版 PDF 未走 OCR、或版式过复杂）；请查看 Core 日志或尝试其他格式/样例。"
                    .to_string(),
            })
        });
        return;
    }

    let result_url = format!(
        "http://127.0.0.1:{}/api/v1/tasks/{}/download",
        state.port, task_id
    );
    let _ = state
        .task_manager
        .update_task(&task_id, |t| t.set_completed(result_url));

    tracing::info!(task_id = %task_id, "Task completed successfully");
}

pub async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<ConvertTask>, AppError> {
    state
        .task_manager
        .get_task(&task_id)
        .map(Json)
        .ok_or(AppError::TaskNotFound { task_id })
}

pub async fn list_tasks(State(state): State<AppState>) -> Json<serde_json::Value> {
    let tasks = state.task_manager.list_tasks();
    Json(serde_json::json!({ "tasks": tasks }))
}

pub async fn cancel_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.task_manager.cancel_task(&task_id)?;
    Ok(Json(serde_json::json!({ "message": "Task cancelled" })))
}

/// 删除任务记录及磁盘上的 temp/<task_id>、tasks/<task_id>。
/// 仅允许已完成 / 失败 / 已取消；进行中或排队中请先取消。
pub async fn delete_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let task = state
        .task_manager
        .get_task(&task_id)
        .ok_or(AppError::TaskNotFound {
            task_id: task_id.clone(),
        })?;
    match task.status {
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => {}
        TaskStatus::Pending | TaskStatus::Processing => {
            return Err(AppError::InvalidOptions {
                message:
                    "只能删除已结束的任务（已完成、失败或已取消）。进行中的任务请先点「取消」。"
                        .into(),
            });
        }
    }
    state.task_manager.remove_task_record(&task_id);
    cleanup_task_dirs(&state.config, &task_id).await;
    Ok(Json(serde_json::json!({ "message": "Task deleted" })))
}

/// 删除所有已结束的任务（已完成 / 失败 / 已取消），并清理对应目录。
pub async fn delete_tasks_cleared(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ids: Vec<String> = state
        .task_manager
        .list_tasks()
        .into_iter()
        .filter(|t| {
            matches!(
                t.status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
            )
        })
        .map(|t| t.task_id)
        .collect();
    let n = ids.len();
    for id in ids {
        state.task_manager.remove_task_record(&id);
        cleanup_task_dirs(&state.config, &id).await;
    }
    Ok(Json(serde_json::json!({
        "message": "Cleared finished tasks",
        "removed": n
    })))
}

async fn cleanup_task_dirs(config: &crate::infra::AppConfig, task_id: &str) {
    let temp = config.temp_dir().join(task_id);
    let tasks = config.tasks_dir().join(task_id);
    if let Err(e) = tokio::fs::remove_dir_all(&temp).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %temp.display(), error = %e, "cleanup task temp dir");
        }
    }
    if let Err(e) = tokio::fs::remove_dir_all(&tasks).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(path = %tasks.display(), error = %e, "cleanup task result dir");
        }
    }
}

/// 解析当前输入/输出下的路由链（与 `POST /convert` 一致）；可选 `preferred_plugins` 用于校验自定义链。
#[derive(Deserialize)]
pub struct PreviewRouteBody {
    pub input_format: String,
    pub output_format: String,
    #[serde(default)]
    pub preferred_plugins: Option<String>,
}

#[derive(Serialize)]
pub struct PreviewRouteResponse {
    pub steps: Vec<crate::core::router::RouteStep>,
    /// 与 `POST /convert` 一致：单跳平局时自动重试的备用插件（按尝试顺序）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fallback_plugin_ids: Vec<String>,
}

pub async fn preview_route(
    State(state): State<AppState>,
    Json(body): Json<PreviewRouteBody>,
) -> Result<Json<PreviewRouteResponse>, AppError> {
    let input = crate::core::normalize_format(body.input_format.trim());
    let output = crate::core::normalize_format(body.output_format.trim());
    let registry = state.plugin_registry.read().await;
    let pref = body
        .preferred_plugins
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let resolved = state.router.resolve(&input, &output, pref, &registry)?;
    Ok(Json(PreviewRouteResponse {
        steps: resolved.steps,
        fallback_plugin_ids: resolved.single_hop_fallback_ids,
    }))
}

pub async fn download_result(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<axum::response::Response, AppError> {
    use axum::body::Body;
    use axum::http::{header, StatusCode};

    let task = state
        .task_manager
        .get_task(&task_id)
        .ok_or(AppError::TaskNotFound {
            task_id: task_id.clone(),
        })?;

    if task.status != TaskStatus::Completed {
        return Err(AppError::InvalidOptions {
            message: format!(
                "Task {} is not completed (status: {:?})",
                task_id, task.status
            ),
        });
    }

    let result_dir = state.config.tasks_dir().join(&task_id);
    let ext = &task.output_format;
    let result_basename = task
        .result_download_filename
        .clone()
        .unwrap_or_else(|| format!("result.{ext}"));
    let result_path = result_dir.join(&result_basename);

    if !result_path.exists() {
        return Err(AppError::Internal(format!(
            "Result file not found: {}",
            result_path.display()
        )));
    }

    let content = tokio::fs::read(&result_path).await?;
    let mime = mime_for_format(ext);

    let disposition = content_disposition_attachment(&result_basename);

    Ok(axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(Body::from(content))
        .unwrap())
}

fn mime_for_format(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "markdown" | "md" => "text/markdown; charset=utf-8",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "txt" | "plain" => "text/plain; charset=utf-8",
        "latex" | "tex" => "application/x-tex",
        _ => "application/octet-stream",
    }
}
