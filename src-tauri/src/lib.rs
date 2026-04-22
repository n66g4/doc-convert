pub mod api;
pub mod core;
pub mod infra;
pub mod plugin_host;
pub mod workers;

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post, put},
    Router,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

use crate::core::{
    CoreLock, LockfileManager, Router as ConvertRouter, RoutesConfig, StalenessResult, TaskManager,
};
use crate::infra::AppConfig;
use crate::plugin_host::{PluginRegistry, PluginSource};

/// 全局应用状态（Arc 包装，可 Clone）
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub task_manager: Arc<TaskManager>,
    pub plugin_registry: Arc<RwLock<PluginRegistry>>,
    pub router: Arc<ConvertRouter>,
    pub port: u16,
    pub started_at: Arc<Instant>,
    pub started_at_unix_ms: u64,
}

pub async fn run_core(
    bundled_plugins_dir: Option<std::path::PathBuf>,
    bundled_pandoc: Option<std::path::PathBuf>,
    resource_root: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    // 1. 解析数据目录
    let data_root = if let Ok(v) = std::env::var("DOCCONVERT_DATA_DIR") {
        std::path::PathBuf::from(v)
    } else {
        dirs_next::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("DocConvert")
    };
    std::fs::create_dir_all(&data_root)?;

    // 2. 初始化日志
    infra::logging::init_logging(&data_root.join("logs"))?;
    tracing::info!(data_root = %data_root.display(), "DocConvert Core starting");

    // 3. 加载配置（Python / Pandoc：随包 resource、开发目录、PATH）
    let mut config = AppConfig::load_or_default(&data_root);
    config.resolve_python_executable(resource_root.as_deref());
    config.resolve_pandoc_executable(bundled_pandoc);
    tracing::info!(
        python = %config.python_executable.display(),
        pandoc = %config.pandoc_executable.display(),
        "Runtime executables"
    );

    // 4. 创建所需目录
    for dir in [
        config.runtime_dir(),
        config.temp_dir(),
        config.tasks_dir(),
        config.logs_dir(),
        config.plugins_extra_dir(),
        config.rapidocr_models_dir(),
    ] {
        std::fs::create_dir_all(&dir)?;
    }

    // 5. Lockfile 管理：检测陈旧锁
    let lockfile_mgr = LockfileManager::new(&config.runtime_dir());
    match lockfile_mgr.check_stale() {
        StalenessResult::Alive(lock) => {
            let requested_port = std::env::var("DOCCONVERT_BIND_PORT")
                .ok()
                .and_then(|s| s.parse::<u16>().ok());
            // 未固定端口（随机分配）时：已有 Core 存活则单实例退出。
            // 已固定端口（如桌面版 17300）但锁文件指向其它端口时：继续启动，否则前端仍请求固定端口会全部失败。
            let same_binding_as_lock = match requested_port {
                Some(req) => req == lock.port,
                None => true,
            };
            if same_binding_as_lock {
                tracing::info!(
                    pid = lock.pid,
                    port = lock.port,
                    "Another Core instance is alive, exiting"
                );
                return Ok(());
            }
            tracing::warn!(
                existing_port = lock.port,
                requested_port = ?requested_port,
                "Existing Core on another port; removing lock and starting this instance"
            );
            lockfile_mgr.remove();
        }
        StalenessResult::Stale => {
            tracing::warn!("Stale lockfile detected, removing");
            lockfile_mgr.remove();
        }
        StalenessResult::NoLock => {}
    }

    // 6. 扫描插件（双根：bundled + extra）
    let mut registry = PluginRegistry::new();

    // 内置插件目录（安装包 resource 或开发仓库 plugins/bundled）
    let bundled_dir = crate::infra::bundled_paths::resolve_bundled_plugins_dir(bundled_plugins_dir);
    tracing::info!(dir = %bundled_dir.display(), "Scanning bundled plugins");
    registry.discover_from_dir(&bundled_dir, PluginSource::Bundled);

    // 用户扩展插件目录
    let extra_dir = config.plugins_extra_dir();
    tracing::info!(dir = %extra_dir.display(), "Scanning extra plugins");
    registry.discover_from_dir(&extra_dir, PluginSource::Extra);

    // 7. 加载路由配置
    let routes_config = load_routes_config(resource_root.as_deref(), &data_root);
    let router = ConvertRouter::new(routes_config);

    // 8. 绑定端口：若设置 DOCCONVERT_BIND_PORT 则固定（便于与 Vite 代理对齐），否则 OS 分配
    let (listener, port) = bind_core_listener().await?;
    tracing::info!(port = port, "Core listening on 127.0.0.1:{}", port);

    // 9. 写入 lockfile
    let lock = CoreLock::new(port);
    lockfile_mgr.write(&lock)?;

    // 10. 构建 AppState
    let started_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let max_concurrent_tasks = config.max_concurrent_tasks;
    let state = AppState {
        config: Arc::new(config),
        task_manager: Arc::new(TaskManager::new(max_concurrent_tasks)),
        plugin_registry: Arc::new(RwLock::new(registry)),
        router: Arc::new(router),
        port,
        started_at: Arc::new(Instant::now()),
        started_at_unix_ms,
    };

    // 11. 构建 Axum 路由
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 默认 body 限制约 2MB，大于此的 multipart 会被截断 → multer 报「解析失败」
    let max_body_bytes = state
        .config
        .max_file_size_bytes
        .saturating_add(4 * 1024 * 1024) as usize;

    let app = Router::new()
        .route("/health", get(api::health::health))
        .route(
            "/api/v1/convert/preview-route",
            post(api::convert::preview_route),
        )
        .route("/api/v1/convert", post(api::convert::post_convert))
        .route(
            "/api/v1/tasks",
            get(api::convert::list_tasks).delete(api::convert::delete_tasks_cleared),
        )
        // 必须在 `/api/v1/tasks/{task_id}` 之前注册，否则 `clear-finished` 会被当成 task_id
        .route(
            "/api/v1/tasks/clear-finished",
            post(api::convert::delete_tasks_cleared),
        )
        .route(
            "/api/v1/tasks/{task_id}/cancel",
            post(api::convert::cancel_task),
        )
        .route(
            "/api/v1/tasks/{task_id}/remove",
            post(api::convert::delete_task),
        )
        .route(
            "/api/v1/tasks/{task_id}/download",
            get(api::convert::download_result),
        )
        .route(
            "/api/v1/tasks/{task_id}",
            get(api::convert::get_task).delete(api::convert::delete_task),
        )
        .route("/api/v1/plugins", get(api::plugins::list_plugins))
        .route(
            "/api/v1/plugins/{plugin_id}/test",
            post(api::plugins::test_plugin),
        )
        .route(
            "/api/v1/plugins/{plugin_id}/enable",
            put(api::plugins::set_plugin_enabled),
        )
        .route("/api/v1/tools/status", get(api::plugins::tools_status))
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .layer(cors)
        .with_state(state.clone());

    // 12. 启动心跳任务（更新 last_heartbeat_ms）
    let lockfile_path = lockfile_mgr;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let _ = lockfile_path.update_heartbeat();
        }
    });

    // 13. 启动 GC 任务
    let state_gc = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            state_gc
                .task_manager
                .gc_expired(state_gc.config.task_result_ttl_secs);
        }
    });

    tracing::info!("DocConvert Core ready");
    axum::serve(listener, app).await?;
    Ok(())
}

fn load_routes_config(
    resource_root: Option<&std::path::Path>,
    data_root: &std::path::Path,
) -> RoutesConfig {
    let bundled_routes = crate::infra::bundled_paths::resolve_bundled_routes_toml(resource_root);

    let mut routes = RoutesConfig::default();

    if let Some(ref path) = bundled_routes {
        match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|s| toml::from_str::<RoutesConfig>(&s).map_err(|e| e.to_string()))
        {
            Ok(r) => {
                tracing::info!(path = %path.display(), "Loaded bundled routes.toml");
                routes = r;
            }
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "Failed to load bundled routes.toml");
            }
        }
    } else {
        tracing::warn!("No bundled routes.toml found (resource or cwd/config)");
    }

    // 用户覆盖 routes.user.toml（深度合并）
    let user_routes = data_root.join("config").join("routes.user.toml");
    if user_routes.exists() {
        match std::fs::read_to_string(&user_routes)
            .map_err(|e| e.to_string())
            .and_then(|s| toml::from_str::<RoutesConfig>(&s).map_err(|e| e.to_string()))
        {
            Ok(user_r) => {
                tracing::info!("Merging user routes.user.toml");
                routes.recipes.extend(user_r.recipes);
                routes.defaults.extend(user_r.defaults);
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to load user routes.user.toml, ignored");
            }
        }
    }

    routes
}

/// `DOCCONVERT_BIND_PORT`：开发时常用 `17300` 与 `vite` 代理一致；未设置则 `127.0.0.1:0` 随机端口。
async fn bind_core_listener() -> anyhow::Result<(tokio::net::TcpListener, u16)> {
    let addr = match std::env::var("DOCCONVERT_BIND_PORT") {
        Ok(s) => {
            let p: u16 = s
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid DOCCONVERT_BIND_PORT: {}", s))?;
            format!("127.0.0.1:{}", p)
        }
        Err(_) => "127.0.0.1:0".to_string(),
    };
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind {} failed: {}", addr, e))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}
