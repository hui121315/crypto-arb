//! `tracing` 初始化。
//!
//! 支持两种格式：`pretty`（开发）与 `json`（生产）。

use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Debug, Clone, Copy, Default)]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
}

impl std::str::FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "pretty" | "" => Ok(LogFormat::Pretty),
            "json" => Ok(LogFormat::Json),
            other => Err(format!("unsupported log format: {other}")),
        }
    }
}

/// 初始化全局 tracing 订阅器。
///
/// `level`：默认日志级别（如 `info`、`debug`）。可被环境变量 `RUST_LOG` 覆盖。
pub fn init(level: &str, format: LogFormat) {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(env_filter);

    match format {
        LogFormat::Pretty => {
            registry
                .with(fmt::layer().with_target(true).with_line_number(true))
                .init();
        }
        LogFormat::Json => {
            registry.with(fmt::layer().json()).init();
        }
    }
}
