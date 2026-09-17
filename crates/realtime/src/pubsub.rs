//! Redis Pub/Sub 客户端（跨进程消息分发）。
//!
//! ## V1 范围
//! 实现 `publish` + `subscribe` 两个基础操作。订阅返回 [`tokio::sync::mpsc::Receiver`]，
//! 屏蔽 redis-rs 的连接管理细节。
//!
//! 对应 Python `data-service` → Redis Pub/Sub → `analysis-service` 订阅的链路。

use futures::StreamExt;
use redis::aio::ConnectionManager;
use std::fmt;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

const SUBSCRIBE_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const SUBSCRIBE_BACKOFF_MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum PubSubError {
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("invalid redis url: {0}")]
    InvalidUrl(String),
}

/// Redis Pub/Sub 客户端。内部维护一个共享连接管理器供 publish 使用；
/// 每次 `subscribe` 会新建独立连接（Redis Pub/Sub 协议要求）。
#[derive(Clone)]
pub struct RedisPubSub {
    client: redis::Client,
    publisher: ConnectionManager,
}

impl fmt::Debug for RedisPubSub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedisPubSub").finish_non_exhaustive()
    }
}

impl RedisPubSub {
    pub async fn connect(url: &str) -> Result<Self, PubSubError> {
        let client =
            redis::Client::open(url).map_err(|e| PubSubError::InvalidUrl(e.to_string()))?;
        let publisher = ConnectionManager::new(client.clone()).await?;
        info!(redis_url = %redacted_redis_url(url), "redis pub/sub connected");
        Ok(Self { client, publisher })
    }

    /// 向频道发布字符串消息，返回订阅者数。
    pub async fn publish(&self, channel: &str, payload: &str) -> Result<usize, PubSubError> {
        let mut conn = self.publisher.clone();
        let count: usize = redis::cmd("PUBLISH")
            .arg(channel)
            .arg(payload)
            .query_async(&mut conn)
            .await?;
        debug!(channel, count, "published");
        Ok(count)
    }

    /// 订阅频道，返回 `(handle, receiver)`。
    /// - `handle`：后台任务句柄；调用 `.abort()` 停止
    /// - `receiver`：消息流；每条消息为 `String`
    pub async fn subscribe(
        &self,
        channel: &str,
        buffer: usize,
    ) -> Result<(JoinHandle<()>, mpsc::Receiver<String>), PubSubError> {
        let (tx, rx) = mpsc::channel(buffer.max(1));
        let ch_label = channel.to_owned();
        let client = self.client.clone();

        let handle = tokio::spawn(async move {
            subscribe_with_reconnect(client, ch_label, tx).await;
        });
        Ok((handle, rx))
    }

    /// 健康检查：PING 命令。
    pub async fn ping(&self, timeout: Duration) -> Result<(), PubSubError> {
        let mut conn = self.publisher.clone();
        let cmd = redis::cmd("PING");
        let fut = cmd.query_async::<String>(&mut conn);
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(PubSubError::Redis(e)),
            Err(_) => Err(PubSubError::Redis(redis::RedisError::from((
                redis::ErrorKind::IoError,
                "ping timed out",
            )))),
        }
    }
}

async fn subscribe_with_reconnect(
    client: redis::Client,
    channel: String,
    tx: mpsc::Sender<String>,
) {
    let mut backoff = SUBSCRIBE_BACKOFF_INITIAL;
    loop {
        if should_exit(&channel, &tx) {
            return;
        }
        let exit = run_subscription_once(&client, &channel, &tx).await;
        if handle_subscription_exit(exit, &channel) {
            return;
        }
        backoff = sleep_and_grow(backoff).await;
    }
}

fn should_exit(channel: &str, tx: &mpsc::Sender<String>) -> bool {
    if !tx.is_closed() {
        return false;
    }
    info!(channel, "subscriber dropped; exiting");
    true
}

fn handle_subscription_exit(exit: SubscriptionExit, channel: &str) -> bool {
    match exit {
        SubscriptionExit::ReceiverDropped => true,
        SubscriptionExit::Reconnect(reason) => {
            warn!(channel, %reason, "pubsub subscribe interrupted; reconnecting");
            false
        }
    }
}

async fn sleep_and_grow(backoff: Duration) -> Duration {
    tokio::time::sleep(backoff).await;
    (backoff * 2).min(SUBSCRIBE_BACKOFF_MAX)
}

#[derive(Debug)]
enum SubscriptionExit {
    ReceiverDropped,
    Reconnect(String),
}

async fn run_subscription_once(
    client: &redis::Client,
    channel: &str,
    tx: &mpsc::Sender<String>,
) -> SubscriptionExit {
    let mut pubsub = match open_subscription(client, channel).await {
        Ok(pubsub) => pubsub,
        Err(error) => return SubscriptionExit::Reconnect(error.to_string()),
    };
    let mut stream = pubsub.on_message();
    while let Some(msg) = stream.next().await {
        if forward_message(channel, tx, msg).await {
            info!(channel, "subscriber dropped; exiting");
            return SubscriptionExit::ReceiverDropped;
        }
    }
    SubscriptionExit::Reconnect("stream ended unexpectedly".to_owned())
}

async fn open_subscription(
    client: &redis::Client,
    channel: &str,
) -> Result<redis::aio::PubSub, redis::RedisError> {
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(channel).await?;
    Ok(pubsub)
}

async fn forward_message(channel: &str, tx: &mpsc::Sender<String>, msg: redis::Msg) -> bool {
    let Some(payload) = decode_payload(channel, &msg) else {
        return false;
    };
    tx.send(payload).await.is_err()
}

fn decode_payload(channel: &str, msg: &redis::Msg) -> Option<String> {
    match msg.get_payload() {
        Ok(payload) => Some(payload),
        Err(error) => {
            warn!(channel, %error, "decode payload failed");
            None
        }
    }
}

fn redacted_redis_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "<invalid redis url>".to_owned();
    };
    let without_query = rest
        .split_once('?')
        .map_or(rest, |(before_query, _)| before_query);
    let without_fragment = without_query
        .split_once('#')
        .map_or(without_query, |(before_fragment, _)| before_fragment);
    let authority_end = without_fragment.find('/').unwrap_or(without_fragment.len());
    let (authority, path) = without_fragment.split_at(authority_end);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if host.is_empty() {
        return format!("{scheme}://<redacted>");
    }
    format!("{scheme}://{host}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没有 Redis 时的连接错误路径。
    /// 真实 Redis 集成测试在 CI 中通过 docker-compose 启动 redis 后单独跑（非默认）。
    #[tokio::test]
    async fn connect_invalid_url_returns_error() {
        let result = RedisPubSub::connect("not-a-real-url").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_unreachable_host_fails_quickly() -> Result<(), Box<dyn std::error::Error>> {
        // 不存在的端口
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            RedisPubSub::connect("redis://127.0.0.1:1"),
        )
        .await;
        // 应在超时前返回错误
        match result {
            Ok(Err(_)) => {}
            Ok(Ok(_)) => {
                return Err(std::io::Error::other("unexpected connection success").into());
            }
            Err(_) => {} // 也接受超时
        }
        Ok(())
    }

    #[test]
    fn redacted_url_removes_userinfo() {
        let url = redis_url_with_userinfo("user", "secret", "cache.internal:6379/2");

        let redacted = redacted_redis_url(&url);

        assert_eq!(redacted, "redis://cache.internal:6379/2");
        assert!(!redacted.contains("user"));
        assert!(!redacted.contains("secret"));
    }

    #[test]
    fn redacted_url_preserves_plain_endpoint() {
        let url = "rediss://cache.internal:6380/0";

        let redacted = redacted_redis_url(url);

        assert_eq!(redacted, url);
    }

    #[test]
    fn redacted_url_drops_query_and_fragment() {
        let url = format!(
            "{}?token={}#frag",
            redis_url_with_userinfo("", "secret", "127.0.0.1:6379/1"),
            "secret"
        );

        let redacted = redacted_redis_url(&url);

        assert_eq!(redacted, "redis://127.0.0.1:6379/1");
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("token"));
    }

    fn redis_url_with_userinfo(user: &str, password: &str, host_and_db: &str) -> String {
        let scheme = "redis://";
        format!("{scheme}{user}:{password}@{host_and_db}")
    }
}
