/// Worker 分发层：根据插件 runtime 类型调用对应 worker（架构 §4.1, §6.2）
use crate::core::router::RouteStep;
use crate::infra::AppError;
use crate::AppState;
use std::path::Path;

pub async fn dispatch_worker(
    state: &AppState,
    step: &RouteStep,
    input_path: &Path,
    output_path: &Path,
    temp_dir: &Path,
    options_json: Option<&str>,
) -> Result<(), AppError> {
    let registry = state.plugin_registry.read().await;
    let meta = registry
        .get_plugin(&step.plugin_id)
        .ok_or_else(|| AppError::PluginNotFound {
            plugin_id: step.plugin_id.clone(),
        })?;

    let runtime_type = meta.runtime_type.clone();
    let plugin_dir = meta.plugin_dir.clone();
    let entry = meta.entry.clone();
    drop(registry);

    match runtime_type.as_str() {
        "python" => {
            python_worker(
                state,
                &step.plugin_id,
                &plugin_dir,
                entry.as_deref().unwrap_or("plugin_main:run"),
                input_path,
                output_path,
                &step.in_format,
                &step.out_format,
                temp_dir,
                options_json,
            )
            .await
        }
        "pandoc_wrapper" => {
            pandoc_worker(
                state,
                input_path,
                output_path,
                &step.in_format,
                &step.out_format,
                temp_dir,
            )
            .await
        }
        other => Err(AppError::Internal(format!(
            "Unknown runtime type: {}",
            other
        ))),
    }
}

/// Python worker：通过 JSON-RPC over stdio 调用（ADR-001 方案 A）
#[allow(clippy::too_many_arguments)] // 单次调用上下文完整，拆结构体收益有限
async fn python_worker(
    state: &AppState,
    plugin_id: &str,
    plugin_dir: &Path,
    entry: &str,
    input_path: &Path,
    output_path: &Path,
    in_format: &str,
    out_format: &str,
    temp_dir: &Path,
    options_json: Option<&str>,
) -> Result<(), AppError> {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let python = &state.config.python_executable;

    // 构造调用 entry 的 Python 启动脚本
    // entry 格式：module:function
    let (module, func) = entry.split_once(':').unwrap_or((entry, "run"));

    let mut params = serde_json::json!({
        "plugin_id": plugin_id,
        "input_path": input_path.to_string_lossy(),
        "output_path": output_path.to_string_lossy(),
        "in_format": in_format,
        "out_format": out_format,
        "temp_dir": temp_dir.to_string_lossy(),
    });
    if let Some(raw) = options_json {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
            params["options"] = v;
        }
    }
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "convert",
        "id": 1,
        "params": params
    });

    tracing::info!(
        plugin_id = %plugin_id,
        input = %input_path.display(),
        in_format = %in_format,
        out_format = %out_format,
        "python plugin convert starting"
    );

    // RapidOCR 默认把模型下载到随包 site-packages；改为用户数据目录（macOS 即 Application Support/DocConvert/...）
    let rapidocr_models_dir = match std::env::var("DOCCONVERT_RAPIDOCR_MODEL_DIR") {
        Ok(s) if !s.trim().is_empty() => std::path::PathBuf::from(s.trim()),
        _ => state.config.rapidocr_models_dir(),
    };
    if let Err(e) = tokio::fs::create_dir_all(&rapidocr_models_dir).await {
        tracing::warn!(
            error = %e,
            path = %rapidocr_models_dir.display(),
            "无法创建 RapidOCR 模型目录（若后续 OCR 需下载模型可能失败）"
        );
    }

    let inline_script = format!(
        r#"
import sys, json
sys.path.insert(0, r'{plugin_dir}')
from {module} import {func}
request = json.loads(sys.stdin.read())
try:
    result = {func}(request['params'])
    print(json.dumps({{"jsonrpc": "2.0", "id": 1, "result": result}}))
except Exception as e:
    print(json.dumps({{"jsonrpc": "2.0", "id": 1, "error": {{"code": -32000, "message": str(e)}}}}))
"#,
        plugin_dir = plugin_dir.display(),
        module = module,
        func = func,
    );

    let mut child = Command::new(python)
        // 管道场景下 Python 可能对 stderr 做块缓冲，导致 Core 读到的 stderr 为空；与插件诊断日志配合
        .env("PYTHONUNBUFFERED", "1")
        .env(
            "DOCCONVERT_DATA_DIR",
            state.config.data_root.as_os_str(),
        )
        .env(
            "DOCCONVERT_RAPIDOCR_MODEL_DIR",
            rapidocr_models_dir.as_os_str(),
        )
        .arg("-c")
        .arg(&inline_script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| AppError::Internal(format!("Failed to spawn Python: {}", e)))?;

    if let Some(mut stdin) = child.stdin.take() {
        let req_bytes = serde_json::to_vec(&request)?;
        stdin.write_all(&req_bytes).await?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let stderr_lossy = String::from_utf8_lossy(&output.stderr);
    // Docling+RapidOCR 会向 stderr 打大量进度行；过小会截掉末尾槽位诊断与 convert done（看起来像「没日志」）
    const MAX_PLUGIN_STDERR_DEFAULT: usize = 64 * 1024;
    const MAX_PLUGIN_STDERR_DOCLING: usize = 1024 * 1024;
    let max_plugin_stderr_bytes = if plugin_id == "docling_adapter" {
        MAX_PLUGIN_STDERR_DOCLING
    } else {
        MAX_PLUGIN_STDERR_DEFAULT
    };
    let stderr_for_log = if stderr_lossy.len() > max_plugin_stderr_bytes {
        let end = stderr_lossy.floor_char_boundary(max_plugin_stderr_bytes);
        format!(
            "{}… [truncated, total {} bytes]",
            &stderr_lossy[..end],
            stderr_lossy.len()
        )
    } else {
        stderr_lossy.into_owned()
    };

    if !output.status.success() {
        tracing::error!(plugin_id = %plugin_id, stderr = %stderr_for_log, "Python worker exited non-zero");
        return Err(AppError::PluginFailed {
            plugin_id: plugin_id.to_string(),
            step_index: 0,
            detail: stderr_for_log,
        });
    }

    if !stderr_for_log.trim().is_empty() {
        tracing::info!(
            plugin_id = %plugin_id,
            stderr = %stderr_for_log,
            "Python plugin stderr (process succeeded; diagnostics from plugin)"
        );
    } else if plugin_id == "docling_adapter" {
        tracing::warn!(
            plugin_id = %plugin_id,
            "docling_adapter stderr was empty (no plugin diagnostics in docconvert.log for this step)"
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let response: serde_json::Value =
        serde_json::from_str(&stdout).map_err(|e| AppError::PluginFailed {
            plugin_id: plugin_id.to_string(),
            step_index: 0,
            detail: format!("Invalid JSON-RPC response: {} — stdout: {}", e, stdout),
        })?;

    if let Some(err) = response.get("error") {
        return Err(AppError::PluginFailed {
            plugin_id: plugin_id.to_string(),
            step_index: 0,
            detail: err.to_string(),
        });
    }

    tracing::info!(
        plugin_id = %plugin_id,
        stderr_bytes = stderr_for_log.len(),
        "python plugin convert finished"
    );
    Ok(())
}

/// 旧版 Word 二进制 `.doc` → `.docx`。随包 Pandoc 无 `doc` 读入器，需先转 Office Open XML 再走 docx 管线。
async fn legacy_doc_to_docx(input_path: &Path, docx_path: &Path) -> Result<(), AppError> {
    use tokio::process::Command;

    #[cfg(target_os = "macos")]
    {
        let out = Command::new("textutil")
            .arg("-convert")
            .arg("docx")
            .arg("-output")
            .arg(docx_path)
            .arg(input_path)
            .output()
            .await
            .map_err(|e| AppError::PluginFailed {
                plugin_id: "pandoc_wrapper".to_string(),
                step_index: 0,
                detail: format!(
                    "无法调用系统 textutil 将 .doc 转为 .docx（{e}）。请确认文件为有效 Word 97–2004 文档。"
                ),
            })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            return Err(AppError::PluginFailed {
                plugin_id: "pandoc_wrapper".to_string(),
                step_index: 0,
                detail: format!(
                    "textutil 未能将 .doc 转为 .docx。\nstderr:\n{stderr}\nstdout:\n{stdout}"
                ),
            });
        }
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        let outdir = docx_path.parent().ok_or_else(|| AppError::PluginFailed {
            plugin_id: "pandoc_wrapper".to_string(),
            step_index: 0,
            detail: "无法确定 .doc 转 .docx 的输出目录（路径无父级）。".to_string(),
        })?;
        let mut last: Option<String> = None;
        for exe in ["soffice", "libreoffice"] {
            let r = Command::new(exe)
                .arg("--headless")
                .arg("--convert-to")
                .arg("docx")
                .arg("--outdir")
                .arg(outdir)
                .arg(input_path)
                .output()
                .await;
            let out = match r {
                Ok(o) => o,
                Err(e) => {
                    last = Some(format!("{exe}: {e}"));
                    continue;
                }
            };
            if !out.status.success() {
                last = Some(format!(
                    "{}: {}",
                    exe,
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
                continue;
            }
            if tokio::fs::metadata(docx_path).await.map(|m| m.len()).unwrap_or(0) > 0 {
                return Ok(());
            }
            last = Some(format!("{exe}: 未生成有效的 .docx 文件"));
        }
        return Err(AppError::PluginFailed {
            plugin_id: "pandoc_wrapper".to_string(),
            step_index: 0,
            detail: format!(
                "未检测到可用的 LibreOffice（需在 PATH 中提供 soffice/libreoffice）。\
                 在 macOS 上可使用系统自带的 textutil；当前错误：{}",
                last.unwrap_or_else(|| "未知".into())
            ),
        });
    }
}

/// Pandoc worker：直接调用 pandoc 二进制（架构 §4.1）
async fn pandoc_worker(
    state: &AppState,
    input_path: &Path,
    output_path: &Path,
    in_format: &str,
    out_format: &str,
    temp_dir: &Path,
) -> Result<(), AppError> {
    use tokio::process::Command;

    let pandoc = &state.config.pandoc_executable;
    let pandoc_path = pandoc.as_path();
    if !pandoc_path.is_file() {
        return Err(AppError::PluginFailed {
            plugin_id: "pandoc_wrapper".to_string(),
            step_index: 0,
            detail: format!(
                "找不到 Pandoc 可执行文件: {}。请在构建前运行 npm run fetch-pandoc，或设置环境变量 DOCCONVERT_PANDOC 指向 pandoc 二进制。",
                pandoc.display()
            ),
        });
    }

    tracing::trace!(task_temp = %temp_dir.display(), "pandoc_worker");

    let intermediate_docx = if in_format == "doc" {
        let docx_path = input_path.with_extension("docx");
        let _ = tokio::fs::remove_file(&docx_path).await;
        legacy_doc_to_docx(input_path, &docx_path).await?;
        let len = tokio::fs::metadata(&docx_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        if len == 0 {
            let _ = tokio::fs::remove_file(&docx_path).await;
            return Err(AppError::PluginFailed {
                plugin_id: "pandoc_wrapper".to_string(),
                step_index: 0,
                detail: ".doc 经预处理后得到的 .docx 为空（0 字节），请确认源文件未损坏。".to_string(),
            });
        }
        tracing::info!(
            task_temp = %temp_dir.display(),
            docx = %docx_path.display(),
            bytes = len,
            "legacy .doc preprocessed to docx before pandoc"
        );
        Some(docx_path)
    } else {
        None
    };

    let pandoc_src = intermediate_docx.as_deref().unwrap_or(input_path);
    // 格式映射：canonical id → pandoc --from / --to（输入/输出对 plain 的处理不同）
    let pandoc_in = if in_format == "doc" {
        "docx"
    } else {
        pandoc_format_in(in_format)
    };
    let pandoc_out = pandoc_format_out(out_format);

    let mut cmd = Command::new(pandoc_path);
    cmd.arg("--from")
        .arg(pandoc_in)
        .arg("--to")
        .arg(pandoc_out)
        .arg("-o")
        .arg(output_path)
        .arg(pandoc_src);

    tracing::debug!(
        cmd = ?cmd,
        "Invoking pandoc"
    );

    let output = cmd
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to spawn pandoc: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        let mut detail = format!("pandoc 退出码 {code}\n");
        if !stderr.trim().is_empty() {
            detail.push_str(&stderr);
        }
        if !stdout.trim().is_empty() {
            if !stderr.trim().is_empty() {
                detail.push('\n');
            }
            detail.push_str(&stdout);
        }
        if detail.lines().count() <= 1 {
            detail.push_str("(无 stderr/stdout 输出)");
        }
        return Err(AppError::PluginFailed {
            plugin_id: "pandoc_wrapper".to_string(),
            step_index: 0,
            detail,
        });
    }

    let out_len = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    if out_len == 0 {
        return Err(AppError::PluginFailed {
            plugin_id: "pandoc_wrapper".to_string(),
            step_index: 0,
            detail: "pandoc 已退出成功但输出文件为空（0 字节）。请检查输入是否为有效文档。"
                .to_string(),
        });
    }

    Ok(())
}

fn pandoc_format_in(fmt: &str) -> &str {
    match fmt {
        "markdown" | "md" => "markdown",
        "html" => "html",
        "docx" => "docx",
        // Pandoc 无名为 plain 的输入格式：纯文本按 markdown 读入
        "plain" | "txt" | "text" => "markdown",
        "latex" | "tex" => "latex",
        "rst" => "rst",
        "json" => "json",
        "rtf" => "rtf",
        other => other,
    }
}

fn pandoc_format_out(fmt: &str) -> &str {
    match fmt {
        "markdown" | "md" => "markdown",
        "html" => "html",
        "docx" => "docx",
        "plain" | "txt" | "text" => "plain",
        "latex" | "tex" => "latex",
        "rst" => "rst",
        "json" => "json",
        "pdf" => "pdf",
        "rtf" => "rtf",
        other => other,
    }
}

#[cfg(test)]
mod pandoc_format_tests {
    use super::{pandoc_format_in, pandoc_format_out};

    #[test]
    fn plain_text_input_uses_markdown_reader() {
        assert_eq!(pandoc_format_in("plain"), "markdown");
        assert_eq!(pandoc_format_in("txt"), "markdown");
    }

    #[test]
    fn plain_text_output_stays_plain() {
        assert_eq!(pandoc_format_out("plain"), "plain");
    }

    #[test]
    fn text_alias_maps_like_plain() {
        assert_eq!(pandoc_format_in("text"), "markdown");
        assert_eq!(pandoc_format_out("text"), "plain");
    }
}
