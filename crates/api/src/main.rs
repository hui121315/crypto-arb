//! crypto-arb-api 入口。
//!
//! 加载配置 → 初始化日志 → 构建 Router → 启动 Tokio 异步服务器。

use std::str::FromStr;

use anyhow::Context;
use common::config::AppConfig;
use common::logging::{self, LogFormat};
use tokio::net::TcpListener;
use tracing::info;

mod app;
mod data_source;
mod lifecycle;
mod metrics;
mod middleware;
mod route_specs;
mod routers;
mod services;
mod state;
mod task_registry;
mod trading_errors;
mod trading_service;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::load().context("failed to load config")?;
    let log_format = LogFormat::from_str(&config.log_format).unwrap_or_default();
    logging::init(&config.log_level, log_format);

    // 安全审计日志初始化（必须早于任何敏感 router 创建）。
    middleware::audit::init(config.security.audit_log_path.as_deref());

    info!(
        version = env!("CARGO_PKG_VERSION"),
        host = %config.host,
        port = config.port,
        auth_required = config.security.auth_required(),
        cors_origins = config.security.allowed_origins.len(),
        audit_log = %config.security.audit_log_path.as_deref().unwrap_or("disabled"),
        "starting crypto-arb-api"
    );

    // 硬失败闸门：绑定到非 loopback 接口却未配置 auth_token 时拒绝启动，
    // 防止复制 `.env.example`（APP_HOST=0.0.0.0）把交易 API 无鉴权暴露到公网。
    config.ensure_bind_security()?;

    if !config.security.auth_required() {
        tracing::warn!(
            host = %config.host,
            "SECURITY: auth_token not configured; all endpoints are open. \
             Allowed only because the bind is loopback. \
             Set APP_SECURITY__AUTH_TOKEN before binding a public interface."
        );
    }

    let state = state::AppState::new(config.clone()).await?;

    let router = app::build_router(state.clone());

    let listener = TcpListener::bind(config.bind_addr())
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr()))?;

    info!("listening on {}", config.bind_addr());

    // 启动钩子：注册 10 家交易所 + LLM provider + snapshot updater。
    // 监听绑定先完成，后台预热不再阻塞端口占用与上层健康探测。
    let mut tasks = lifecycle::init_services(&state);

    let shutdown = lifecycle::shutdown_signal();
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
        .context("server error")?;

    info!("draining runtime writers and background tasks");
    lifecycle::drain_runtime(&state, &mut tasks)
        .await
        .ensure_clean()?;
    info!("server stopped cleanly");
    Ok(())
}
