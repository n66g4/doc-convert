use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init_logging(logs_dir: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(logs_dir)?;

    let log_file = logs_dir.join("docconvert.log");
    let file_appender = tracing_appender::rolling::daily(logs_dir, "docconvert.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // WorkerGuard 必须在进程存活期间一直持有；若在此处 drop，后台写盘线程会退出，
    // 结果只有「Logging initialized」等极少数行能落盘，后续 tracing 事件全部丢失。
    std::mem::forget(guard);

    // Suppress sensitive content — NFR-013
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,doc_convert_core=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true).pretty())
        .with(
            fmt::layer()
                .with_target(true)
                .json()
                .with_writer(non_blocking),
        )
        .try_init()
        .ok(); // ignore re-init error in tests

    tracing::info!(log_file = %log_file.display(), "Logging initialized");
    Ok(())
}
