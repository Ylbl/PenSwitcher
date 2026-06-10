use std::{
    env, fs,
    path::PathBuf,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

pub fn init_logging() {
    let log_dir = log_dir();
    if let Err(error) = fs::create_dir_all(&log_dir) {
        eprintln!("创建日志目录失败: {error}");
    }

    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or_default();
    let file_appender = tracing_appender::rolling::never(&log_dir, format!("run-{started_at}.log"));
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let _ = LOG_GUARD.set(guard);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,penswitcher=debug"));
    let console_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_writer(std::io::stdout);
    let file_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_ansi(false)
        .with_writer(file_writer);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .try_init();

    std::panic::set_hook(Box::new(|panic_info| {
        tracing::error!(%panic_info, "程序发生 panic，已写入本次运行日志");
        eprintln!("程序发生 panic: {panic_info}");
    }));

    tracing::info!(path = %log_dir.display(), "日志系统已初始化");
}

pub fn log_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PenSwitcher")
        .join("logs")
}
