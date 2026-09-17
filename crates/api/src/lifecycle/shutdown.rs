use tokio::signal;
use tracing::{info, warn};

const STARTUP_SIGNAL_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// 等待 Ctrl+C 或 SIGTERM。
pub(crate) async fn shutdown_signal() {
    let message = wait_for_stable_stop_message().await;
    info!(message);
}

async fn wait_for_stable_stop_message() -> &'static str {
    #[cfg(unix)]
    return wait_for_stable_unix_stop_message().await;

    #[cfg(not(unix))]
    wait_for_stable_ctrl_c_message().await
}

#[cfg(unix)]
async fn wait_for_stable_unix_stop_message() -> &'static str {
    let mut interrupt = signal_stream(signal::unix::SignalKind::interrupt());
    let mut terminate = signal_stream(signal::unix::SignalKind::terminate());
    let started_at = std::time::Instant::now();
    loop {
        let message = tokio::select! {
            message = wait_for_unix_signal(&mut interrupt, "received Ctrl+C, shutting down") => message,
            message = wait_for_unix_signal(&mut terminate, "received SIGTERM, shutting down") => message,
        };
        if started_at.elapsed() < STARTUP_SIGNAL_GRACE {
            warn!(message, "ignored shutdown signal during startup grace");
            continue;
        }
        return message;
    }
}

#[cfg(unix)]
fn signal_stream(kind: signal::unix::SignalKind) -> Option<signal::unix::Signal> {
    match signal::unix::signal(kind) {
        Ok(stream) => Some(stream),
        Err(error) => {
            warn!(%error, "failed to install shutdown signal handler");
            None
        }
    }
}

#[cfg(unix)]
async fn wait_for_unix_signal(
    stream: &mut Option<signal::unix::Signal>,
    message: &'static str,
) -> &'static str {
    let Some(stream) = stream.as_mut() else {
        return std::future::pending::<&'static str>().await;
    };
    if stream.recv().await.is_none() {
        warn!(message, "shutdown signal stream closed");
        return std::future::pending::<&'static str>().await;
    }
    message
}

#[cfg(not(unix))]
async fn wait_for_stable_ctrl_c_message() -> &'static str {
    let started_at = std::time::Instant::now();
    loop {
        if let Err(error) = signal::ctrl_c().await {
            warn!(%error, "failed to install Ctrl+C handler");
            return std::future::pending::<&'static str>().await;
        }
        let message = "received Ctrl+C, shutting down";
        if started_at.elapsed() >= STARTUP_SIGNAL_GRACE {
            return message;
        }
        warn!(message, "ignored shutdown signal during startup grace");
    }
}
