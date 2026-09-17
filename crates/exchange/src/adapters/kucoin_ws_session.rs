//! Shared KuCoin classic public WebSocket session bootstrap.
//!
//! KuCoin classic public WS endpoints are gated by one-shot `bullet-public`
//! tokens: `POST /api/v1/bullet-public` returns a short-lived token plus the
//! WebSocket endpoint and recommended ping interval. The token is later used
//! to build the `wss://...?token=<token>` connect URL.
//!
//! Futures and Spot/Margin use the same response shape but different REST
//! hosts, so the bullet helpers live here to avoid duplicating DTO plumbing.
//!
//! Official docs:
//! - Futures: <https://www.kucoin.com/docs-new/websocket-api/base-info/get-public-token-futures>
//! - Spot/Margin: <https://www.kucoin.com/docs-new/websocket-api/base-info/get-public-token-spot-margin>

use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::ws::manager::WsManager;
use reqwest::Method;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

pub(super) const KUCOIN_FUTURES_BASE_URL: &str = "https://api-futures.kucoin.com";
pub(super) const KUCOIN_SPOT_BASE_URL: &str = "https://api.kucoin.com";
pub(super) const DEFAULT_PING_INTERVAL_MS: u64 = 18_000;
const SESSION_REFRESH_AFTER: Duration = Duration::from_secs(23 * 60 * 60);
const SESSION_REFRESH_RETRY: Duration = Duration::from_secs(60);
const HEARTBEAT_SAFETY_MARGIN_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy)]
pub(super) enum PublicSessionKind {
    Futures,
    Spot,
}

#[derive(Debug, Clone)]
pub(super) struct PublicSession {
    /// `wss://...?token=<token>` connect URL, ready to feed into `WsConfig`.
    pub(super) ws_url: String,
    /// Server-recommended ping interval in milliseconds.
    pub(super) ping_interval_ms: u64,
}

pub(super) async fn fetch_public_session(http: &HttpClient) -> ExchangeResult<PublicSession> {
    fetch_session(http, KUCOIN_FUTURES_BASE_URL).await
}

pub(super) async fn fetch_spot_public_session(http: &HttpClient) -> ExchangeResult<PublicSession> {
    fetch_session(http, KUCOIN_SPOT_BASE_URL).await
}

/// KuCoin Classic tokens and connections expire after 24 hours. Refresh the
/// next-handshake URL one hour early so a server-driven reconnect never loops
/// forever on the startup token.
pub(super) fn spawn_public_session_refresh(
    http: HttpClient,
    manager: Arc<WsManager>,
    kind: PublicSessionKind,
) {
    tokio::spawn(async move {
        tokio::time::sleep(SESSION_REFRESH_AFTER).await;
        loop {
            let result = match kind {
                PublicSessionKind::Futures => fetch_public_session(&http).await,
                PublicSessionKind::Spot => fetch_spot_public_session(&http).await,
            };
            match result {
                Ok(session) => match manager.replace_connect_url(session.ws_url) {
                    Ok(()) => {
                        info!(?kind, "kucoin public ws token refreshed for next reconnect");
                        tokio::time::sleep(SESSION_REFRESH_AFTER).await;
                    }
                    Err(error) => {
                        warn!(?kind, error = %error, "kucoin public ws refresh URL rejected");
                        tokio::time::sleep(SESSION_REFRESH_RETRY).await;
                    }
                },
                Err(error) => {
                    warn!(?kind, error = %error, "kucoin public ws token refresh failed");
                    tokio::time::sleep(SESSION_REFRESH_RETRY).await;
                }
            }
        }
    });
}

async fn fetch_session(http: &HttpClient, base_url: &str) -> ExchangeResult<PublicSession> {
    let url = format!("{base_url}/api/v1/bullet-public");
    let resp = http
        .execute_with_retry(|| http.request(Method::POST, &url))
        .await?;
    let wrap: BulletResponse = resp.json().await.map_err(|e| parse_err(&e))?;
    wrap.into_session()
}

fn parse_err(e: &reqwest::Error) -> ExchangeError {
    ExchangeError::Parse(format!("kucoin ws json: {e}"))
}

#[derive(Debug, Deserialize)]
struct BulletResponse {
    code: String,
    data: BulletData,
}

impl BulletResponse {
    fn into_session(self) -> ExchangeResult<PublicSession> {
        if self.code != "200000" {
            return Err(ExchangeError::Api {
                exchange: "kucoin".into(),
                code: self.code,
                message: "bullet-public".into(),
            });
        }
        let server = self
            .data
            .instance_servers
            .into_iter()
            .find(|server| server.protocol == "websocket")
            .ok_or_else(|| ExchangeError::Parse("kucoin ws missing websocket server".into()))?;
        let ping_interval_ms = server
            .ping_interval
            .unwrap_or(DEFAULT_PING_INTERVAL_MS)
            .saturating_sub(HEARTBEAT_SAFETY_MARGIN_MS)
            .max(1_000);
        Ok(PublicSession {
            ws_url: format!(
                "{}?token={}&connectId={}",
                server.endpoint,
                self.data.token,
                next_connect_id()
            ),
            ping_interval_ms,
        })
    }
}

fn next_connect_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("crossline-{}-{sequence}", common::time::now_ms())
}

#[derive(Debug, Deserialize)]
struct BulletData {
    token: String,
    #[serde(default, rename = "instanceServers")]
    instance_servers: Vec<BulletServer>,
}

#[derive(Debug, Deserialize)]
struct BulletServer {
    endpoint: String,
    protocol: String,
    #[serde(default, rename = "pingInterval")]
    ping_interval: Option<u64>,
}

#[cfg(test)]
#[path = "kucoin_ws_session_tests.rs"]
mod tests;
