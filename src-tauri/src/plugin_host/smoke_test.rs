//! 插件自检：轻量（health / pandoc --version）与深度（最小样例走 `dispatch_worker` 真实转换路径）。
use crate::core::router::RouteStep;
use crate::plugin_host::PluginMeta;
use crate::AppState;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTestDepth {
    Smoke,
    Deep,
}

#[derive(Debug, Serialize)]
pub struct PluginSmokeTestResult {
    /// `smoke`：轻量自检；`deep`：深度自检（最小样例转换）
    pub depth: String,
    pub ok: bool,
    pub plugin_id: String,
    pub runtime_type: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

fn test_result(
    depth: &str,
    ok: bool,
    plugin_id: String,
    runtime_type: String,
    message: String,
    detail: Option<serde_json::Value>,
) -> PluginSmokeTestResult {
    PluginSmokeTestResult {
        depth: depth.to_string(),
        ok,
        plugin_id,
        runtime_type,
        message,
        detail,
    }
}

pub async fn run_plugin_test(
    state: &AppState,
    meta: &PluginMeta,
    depth: PluginTestDepth,
) -> PluginSmokeTestResult {
    match depth {
        PluginTestDepth::Smoke => smoke_test_plugin(state, meta).await,
        PluginTestDepth::Deep => deep_test_plugin(state, meta).await,
    }
}

pub async fn smoke_test_plugin(state: &AppState, meta: &PluginMeta) -> PluginSmokeTestResult {
    let plugin_id = meta.id.clone();
    let d = "smoke";
    match meta.runtime_type.as_str() {
        "pandoc_wrapper" => smoke_pandoc(state, d, &plugin_id).await,
        "python" => smoke_python(state, d, meta).await,
        other => test_result(
            d,
            false,
            plugin_id,
            other.to_string(),
            format!("暂不支持的 runtime 类型: {other}"),
            None,
        ),
    }
}

async fn deep_test_plugin(state: &AppState, meta: &PluginMeta) -> PluginSmokeTestResult {
    let plugin_id = meta.id.clone();
    let d = "deep";
    match meta.runtime_type.as_str() {
        "pandoc_wrapper" | "python" => deep_dispatch(state, d, meta).await,
        other => test_result(
            d,
            false,
            plugin_id,
            other.to_string(),
            format!("深度自检暂不支持的 runtime: {other}"),
            None,
        ),
    }
}

/// 与生产任务相同：走 `workers::dispatch_worker` + 最小样例文件。
async fn deep_dispatch(state: &AppState, depth: &str, meta: &PluginMeta) -> PluginSmokeTestResult {
    let plugin_id = meta.id.clone();
    let runtime_type = meta.runtime_type.clone();

    let base = state.config.temp_dir().join("plugin_deep_test");
    if let Err(e) = std::fs::create_dir_all(&base) {
        return test_result(
            depth,
            false,
            plugin_id,
            runtime_type,
            format!("无法创建临时目录: {e}"),
            None,
        );
    }

    let temp = match tempfile::tempdir_in(&base) {
        Ok(t) => t,
        Err(e) => {
            return test_result(
                depth,
                false,
                plugin_id,
                runtime_type,
                format!("无法分配临时目录: {e}"),
                None,
            );
        }
    };

    let Some((in_fmt, out_fmt)) = pick_deep_pair(meta) else {
        return test_result(
            depth,
            false,
            plugin_id,
            runtime_type,
            "深度自检：当前插件能力中找不到与内置样例匹配的输入格式（支持 markdown/html/plain/json/rst/latex/rtf 等文本类）"
                .into(),
            None,
        );
    };

    let Some((in_name, in_bytes)) = min_fixture_bytes(&in_fmt) else {
        return test_result(
            depth,
            false,
            plugin_id,
            runtime_type,
            format!("深度自检：无内置样例内容（输入格式 {in_fmt}）"),
            None,
        );
    };

    let input_path = temp.path().join(in_name);
    if let Err(e) = std::fs::write(&input_path, in_bytes) {
        return test_result(
            depth,
            false,
            plugin_id,
            runtime_type,
            format!("写入样例输入失败: {e}"),
            None,
        );
    }

    let out_name = format!("deep_out.{}", extension_for_format(&out_fmt));
    let output_path = temp.path().join(out_name);

    let step = RouteStep {
        plugin_id: meta.id.clone(),
        in_format: in_fmt.clone(),
        out_format: out_fmt.clone(),
        step_index: 0,
    };

    match crate::workers::dispatch_worker(
        state,
        &step,
        &input_path,
        &output_path,
        temp.path(),
        None,
    )
    .await
    {
        Ok(()) => {
            if !output_path.is_file() {
                return test_result(
                    depth,
                    false,
                    plugin_id,
                    runtime_type,
                    "转换完成但未生成输出文件".into(),
                    Some(serde_json::json!({
                        "in_format": in_fmt,
                        "out_format": out_fmt,
                        "expected_output": output_path.display().to_string(),
                    })),
                );
            }
            let len = std::fs::metadata(&output_path)
                .map(|m| m.len())
                .unwrap_or(0);
            test_result(
                depth,
                true,
                plugin_id,
                runtime_type,
                format!("深度自检通过：{in_fmt} → {out_fmt}，输出 {len} 字节"),
                Some(serde_json::json!({
                    "in_format": in_fmt,
                    "out_format": out_fmt,
                    "output_bytes": len,
                    "output_path": output_path.display().to_string(),
                })),
            )
        }
        Err(e) => test_result(
            depth,
            false,
            plugin_id,
            runtime_type,
            format!("深度自检失败（真实转换路径）: {e}"),
            None,
        ),
    }
}

/// 优先选用有内置样例的输入格式，输出优先 `markdown`（若支持）。
fn pick_deep_pair(meta: &PluginMeta) -> Option<(String, String)> {
    let inputs_set: HashSet<String> = meta
        .capabilities
        .inputs
        .iter()
        .map(|s| crate::core::normalize_format(s))
        .collect();

    let mut outputs: Vec<String> = meta
        .capabilities
        .outputs
        .iter()
        .map(|s| crate::core::normalize_format(s))
        .collect();
    outputs.sort();
    outputs.dedup();
    if outputs.is_empty() {
        return None;
    }
    if let Some(i) = outputs.iter().position(|x| x == "markdown") {
        let m = outputs.remove(i);
        outputs.insert(0, m);
    }

    for preferred in ["markdown", "html", "plain", "json", "rst", "latex", "rtf"] {
        if !inputs_set.contains(preferred) {
            continue;
        }
        if min_fixture_bytes(preferred).is_none() {
            continue;
        }
        if let Some(out_fmt) = outputs.first() {
            return Some((preferred.to_string(), out_fmt.clone()));
        }
    }
    None
}

fn min_fixture_bytes(in_fmt: &str) -> Option<(&'static str, &'static [u8])> {
    match in_fmt {
        "markdown" => Some(("deep_in.md", b"# DocConvert deep self-test\n\nok.\n")),
        "html" => Some((
            "deep_in.html",
            b"<!DOCTYPE html><html><body><p>ok</p></body></html>",
        )),
        "plain" => Some(("deep_in.txt", b"ok\n")),
        "json" => Some(("deep_in.json", b"{\"doc\":\"ok\"}\n")),
        "rst" => Some(("deep_in.rst", b"ok\n----\n\n")),
        "latex" | "tex" => Some((
            "deep_in.tex",
            b"\\documentclass{article}\\begin{document}ok\\end{document}\n",
        )),
        "rtf" => Some((
            "deep_in.rtf",
            br"{\rtf1\ansi\deff0 {\fonttbl{\f0 Times New Roman;}}\f0\fs24 ok\par}",
        )),
        _ => None,
    }
}

fn extension_for_format(fmt: &str) -> &'static str {
    match fmt {
        "markdown" => "md",
        "html" => "html",
        "plain" => "txt",
        "json" => "json",
        "docx" => "docx",
        "latex" | "tex" => "tex",
        "rst" => "rst",
        "pdf" => "pdf",
        "rtf" => "rtf",
        _ => "out",
    }
}

async fn smoke_pandoc(state: &AppState, depth: &str, plugin_id: &str) -> PluginSmokeTestResult {
    use tokio::process::Command;

    let pandoc = &state.config.pandoc_executable;
    let p = pandoc.as_path();
    if !p.is_file() {
        return test_result(
            depth,
            false,
            plugin_id.to_string(),
            "pandoc_wrapper".to_string(),
            format!(
                "找不到 Pandoc: {}。请 npm run fetch-pandoc 或设置 DOCCONVERT_PANDOC。",
                pandoc.display()
            ),
            None,
        );
    }

    let output = match Command::new(p).arg("--version").output().await {
        Ok(o) => o,
        Err(e) => {
            return test_result(
                depth,
                false,
                plugin_id.to_string(),
                "pandoc_wrapper".to_string(),
                format!("无法启动 pandoc: {e}"),
                None,
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return test_result(
            depth,
            false,
            plugin_id.to_string(),
            "pandoc_wrapper".to_string(),
            format!(
                "pandoc --version 失败 (exit {:?}): {}",
                output.status.code(),
                if stderr.trim().is_empty() {
                    stdout.to_string()
                } else {
                    stderr.to_string()
                }
            ),
            None,
        );
    }

    let first_line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();

    test_result(
        depth,
        true,
        plugin_id.to_string(),
        "pandoc_wrapper".to_string(),
        if first_line.is_empty() {
            "pandoc --version 成功".into()
        } else {
            first_line
        },
        Some(serde_json::json!({
            "pandoc_executable": pandoc.display().to_string(),
        })),
    )
}

async fn smoke_python(state: &AppState, depth: &str, meta: &PluginMeta) -> PluginSmokeTestResult {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let plugin_id = meta.id.clone();
    let entry = meta
        .entry
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("plugin_main:run");
    let (module, _) = entry.split_once(':').unwrap_or((entry, "run"));
    let plugin_dir = &meta.plugin_dir;
    if !meta.plugin_dir.is_dir() {
        return test_result(
            depth,
            false,
            plugin_id.clone(),
            "python".to_string(),
            format!("插件目录不存在: {}", plugin_dir.display()),
            None,
        );
    }

    let python = &state.config.python_executable;

    let inline_script = format!(
        r#"
import sys, json, importlib
sys.path.insert(0, r'{plugin_dir}')
m = importlib.import_module('{module}')
if not hasattr(m, "health"):
    print(json.dumps({{"jsonrpc": "2.0", "id": 1, "error": {{"code": -32000, "message": "插件未实现 health()，无法做环境自检"}}}}))
    sys.exit(0)
request = json.loads(sys.stdin.read())
try:
    r = m.health(request.get("params") or {{}})
    print(json.dumps({{"jsonrpc": "2.0", "id": 1, "result": r}}))
except Exception as e:
    print(json.dumps({{"jsonrpc": "2.0", "id": 1, "error": {{"code": -32000, "message": str(e)}}}}))
"#,
        plugin_dir = plugin_dir.display(),
        module = module,
    );

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "health",
        "id": 1,
        "params": {}
    });

    let mut child = match Command::new(python)
        .arg("-c")
        .arg(&inline_script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return test_result(
                depth,
                false,
                plugin_id,
                "python".to_string(),
                format!("无法启动 Python: {e}"),
                Some(serde_json::json!({
                    "python_executable": python.display().to_string(),
                })),
            );
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin
            .write_all(&serde_json::to_vec(&request).unwrap_or_default())
            .await;
    }

    let output = match child.wait_with_output().await {
        Ok(o) => o,
        Err(e) => {
            return test_result(
                depth,
                false,
                plugin_id,
                "python".to_string(),
                format!("Python 自检进程异常: {e}"),
                None,
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return test_result(
            depth,
            false,
            plugin_id,
            "python".to_string(),
            format!("Python 自检退出非零: {stderr}"),
            None,
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let response: serde_json::Value = match serde_json::from_str(stdout.trim()) {
        Ok(v) => v,
        Err(e) => {
            return test_result(
                depth,
                false,
                plugin_id,
                "python".to_string(),
                format!("自检输出非 JSON: {e} — {stdout}"),
                None,
            );
        }
    };

    if let Some(err) = response.get("error") {
        let msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or(&err.to_string())
            .to_string();
        return test_result(
            depth,
            false,
            plugin_id,
            "python".to_string(),
            msg,
            Some(err.clone()),
        );
    }

    let result = response
        .get("result")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    if let Some(st) = result.get("status").and_then(|v| v.as_str()) {
        if st == "error" {
            let msg = result
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("health() 返回 error")
                .to_string();
            return test_result(
                depth,
                false,
                plugin_id,
                "python".to_string(),
                msg,
                Some(result),
            );
        }
    }

    let msg = result
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Python 插件自检通过")
        .to_string();

    test_result(
        depth,
        true,
        plugin_id,
        "python".to_string(),
        msg,
        Some(result),
    )
}
