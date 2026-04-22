#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use doc_convert_core::run_core;
use tauri::Manager;

fn main() {
    // 独立 Core 模式（无 Tauri）：用于开发和测试
    if std::env::args().any(|a| a == "--core-only") {
        tokio::runtime::Runtime::new()
            .expect("Failed to create tokio runtime")
            .block_on(async {
                if let Err(e) = run_core(None, None, None).await {
                    eprintln!("Core error: {}", e);
                    std::process::exit(1);
                }
            });
        return;
    }

    // Tauri 模式：与前端生产构建 `VITE_CORE_API_BASE` 对齐，默认固定本机端口（见 vite `.env.production`）
    if std::env::var("DOCCONVERT_BIND_PORT").is_err() {
        std::env::set_var("DOCCONVERT_BIND_PORT", "17300");
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::convert_submit_local_file
        ])
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let resource_root: Option<std::path::PathBuf> = app.path().resource_dir().ok();

            let bundled_plugins_dir = resource_root
                .as_ref()
                .map(|d: &std::path::PathBuf| d.join("plugins").join("bundled"));

            let bundled_pandoc: Option<std::path::PathBuf> =
                resource_root.as_ref().map(|d: &std::path::PathBuf| {
                    #[cfg(target_os = "windows")]
                    {
                        d.join("pandoc").join("pandoc.exe")
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        d.join("pandoc").join("pandoc")
                    }
                });

            // 在后台独立线程启动 Core（不阻塞 Tauri 事件循环）
            std::thread::spawn(move || {
                tokio::runtime::Runtime::new()
                    .expect("tokio runtime")
                    .block_on(async {
                        if let Err(e) =
                            run_core(bundled_plugins_dir, bundled_pandoc, resource_root).await
                        {
                            tracing::error!("Core crashed: {}", e);
                        }
                    });
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
