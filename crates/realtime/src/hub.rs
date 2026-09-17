//! WebSocket 推送 Hub。
//!
//! 基于 [`tokio::sync::broadcast`]：每个频道一个发送端，订阅者通过 `subscribe()`
//! 获得独立的接收端；高频频道可用 `publish_throttled()` 合并为 100ms 最新值。
//!
//! 对应 Python `WebSocketManager._broadcast(channel, ...)`。

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tracing::debug;

use crate::Throttle;

const THROTTLE_MS: u64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsMessage {
    Text(String),
    Json(Value),
    /// 发布侧一次性序列化好的 JSON 文本：broadcast 给 N 个订阅者时克隆退化为
    /// Arc 指针拷贝（此前 `Json(Value)` 每个订阅者 recv 都深拷贝整棵 Value 树，
    /// 出站侧再各自 `to_string` 一遍）。
    JsonText(std::sync::Arc<str>),
    Binary(Vec<u8>),
}

impl WsMessage {
    pub fn json<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        Ok(WsMessage::JsonText(std::sync::Arc::from(
            serde_json::to_string(value)?,
        )))
    }

    /// 把任一 JSON 载体解析回 `Value`（测试与诊断用；热路径不应调用）。
    pub fn payload_json(&self) -> Option<Value> {
        match self {
            WsMessage::Json(value) => Some(value.clone()),
            WsMessage::JsonText(text) => serde_json::from_str(text).ok(),
            WsMessage::Text(_) | WsMessage::Binary(_) => None,
        }
    }
}

/// 发布订阅 Hub。所有方法均同步（避免 await 死锁），订阅者用 `tokio::spawn` 消费。
#[derive(Clone)]
pub struct WsHub {
    channels: Arc<DashMap<String, broadcast::Sender<WsMessage>>>,
    activity: Arc<DashMap<String, watch::Sender<u64>>>,
    throttles: Arc<DashMap<String, Throttle<WsMessage>>>,
    runtime: Arc<DashMap<String, WsChannelRuntimeCounters>>,
    default_capacity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsChannelRuntimeSnapshot {
    pub channel: String,
    pub subscribers: usize,
    pub lag_events: u64,
    pub skipped_messages: u64,
    pub last_lag_at_ms: Option<i64>,
}

#[derive(Debug, Default)]
struct WsChannelRuntimeCounters {
    lag_events: u64,
    skipped_messages: u64,
    last_lag_at_ms: Option<i64>,
}

impl fmt::Debug for WsHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WsHub")
            .field("channel_count", &self.channels.len())
            .field("activity_count", &self.activity.len())
            .field("throttle_count", &self.throttles.len())
            .field("runtime_count", &self.runtime.len())
            .field("default_capacity", &self.default_capacity)
            .finish()
    }
}

impl WsHub {
    pub fn new(default_capacity: usize) -> Self {
        Self {
            channels: Arc::new(DashMap::new()),
            activity: Arc::new(DashMap::new()),
            throttles: Arc::new(DashMap::new()),
            runtime: Arc::new(DashMap::new()),
            default_capacity: default_capacity.max(1),
        }
    }

    /// 向频道发布消息。频道不存在或无订阅者时不创建发送端。
    /// 返回**成功投递到的订阅者数量**。
    pub fn publish(&self, channel: impl Into<String>, msg: WsMessage) -> usize {
        let ch = channel.into();
        self.notify_activity(&ch);
        publish_to(&self.channels, &ch, msg)
    }

    /// 合并高频频道消息，100ms 只发送最新一条；订单和告警类频道应继续用 `publish()`。
    pub fn publish_throttled(&self, channel: impl Into<String>, msg: WsMessage) -> usize {
        let ch = channel.into();
        self.notify_activity(&ch);
        let subscribers = self.subscriber_count(&ch);
        if subscribers == 0 {
            return 0;
        }
        self.throttle_for(&ch).push(msg);
        subscribers
    }

    /// 订阅频道。如频道不存在则创建。
    pub fn subscribe(&self, channel: impl Into<String>) -> broadcast::Receiver<WsMessage> {
        let ch = channel.into();
        let entry = self
            .channels
            .entry(ch)
            .or_insert_with(|| broadcast::channel(self.default_capacity).0);
        entry.value().subscribe()
    }

    /// Subscribe to payload-free state-change pulses without registering an `AppWS` receiver.
    pub fn subscribe_activity(&self, channel: impl Into<String>) -> watch::Receiver<u64> {
        let channel = channel.into();
        self.activity
            .entry(channel)
            .or_insert_with(|| watch::channel(0).0)
            .subscribe()
    }

    /// Wake internal state consumers without creating or serializing an `AppWS` payload.
    pub fn notify_activity(&self, channel: &str) {
        let Some(sender) = self.activity.get(channel) else {
            return;
        };
        sender.send_modify(|generation| *generation = generation.saturating_add(1));
    }

    /// 已注册的频道名列表（按字典序排序）。
    pub fn channels(&self) -> Vec<String> {
        let mut v: Vec<String> = self.channels.iter().map(|e| e.key().clone()).collect();
        v.sort();
        v
    }

    /// 指定频道的活跃订阅者数。
    pub fn subscriber_count(&self, channel: &str) -> usize {
        self.channels
            .get(channel)
            .map(|e| e.value().receiver_count())
            .unwrap_or(0)
    }

    pub fn record_lag(&self, channel: &str, skipped_messages: u64, observed_at_ms: i64) {
        let mut counters = self.runtime.entry(channel.to_owned()).or_default();
        counters.lag_events = counters.lag_events.saturating_add(1);
        counters.skipped_messages = counters.skipped_messages.saturating_add(skipped_messages);
        counters.last_lag_at_ms = Some(observed_at_ms);
    }

    pub fn runtime_snapshots(&self) -> Vec<WsChannelRuntimeSnapshot> {
        let mut channels = self
            .channels
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<BTreeSet<_>>();
        channels.extend(self.runtime.iter().map(|entry| entry.key().clone()));
        channels
            .into_iter()
            .map(|channel| {
                let counters = self.runtime.get(&channel);
                WsChannelRuntimeSnapshot {
                    subscribers: self.subscriber_count(&channel),
                    lag_events: counters.as_ref().map_or(0, |entry| entry.lag_events),
                    skipped_messages: counters.as_ref().map_or(0, |entry| entry.skipped_messages),
                    last_lag_at_ms: counters.as_ref().and_then(|entry| entry.last_lag_at_ms),
                    channel,
                }
            })
            .collect()
    }

    /// 移除无订阅者的频道。返回被移除的数量。
    pub fn prune_empty(&self) -> usize {
        let to_remove: Vec<String> = self
            .channels
            .iter()
            .filter(|e| e.value().receiver_count() == 0)
            .map(|e| e.key().clone())
            .collect();
        let n = to_remove.len();
        for k in to_remove {
            self.channels.remove(&k);
            if let Some((_, throttle)) = self.throttles.remove(&k) {
                throttle.shutdown();
            }
        }
        self.activity
            .retain(|_, sender| sender.receiver_count() > 0);
        n
    }

    fn throttle_for(&self, channel: &str) -> Throttle<WsMessage> {
        let channels = Arc::clone(&self.channels);
        let channel = channel.to_owned();
        self.throttles
            .entry(channel.clone())
            .or_insert_with(|| {
                let publish_channel = channel.clone();
                Throttle::new(Duration::from_millis(THROTTLE_MS), move |msg| {
                    publish_to(&channels, &publish_channel, msg);
                })
            })
            .clone()
    }
}

fn publish_to(
    channels: &DashMap<String, broadcast::Sender<WsMessage>>,
    channel: &str,
    msg: WsMessage,
) -> usize {
    let Some((sender, subscribers)) = active_sender(channels, channel) else {
        return 0;
    };
    deliver_to(&sender, channel, msg, subscribers)
}

fn active_sender(
    channels: &DashMap<String, broadcast::Sender<WsMessage>>,
    channel: &str,
) -> Option<(broadcast::Sender<WsMessage>, usize)> {
    let Some(entry) = channels.get(channel) else {
        debug!(channel, "publish: channel has no receivers");
        return None;
    };
    let subscribers = entry.value().receiver_count();
    if subscribers == 0 {
        debug!(channel, "publish: no active receivers");
        return None;
    }
    Some((entry.value().clone(), subscribers))
}

fn deliver_to(
    sender: &broadcast::Sender<WsMessage>,
    channel: &str,
    msg: WsMessage,
    subscribers: usize,
) -> usize {
    if let Ok(delivered) = sender.send(msg) {
        return delivered.min(subscribers);
    }
    debug!(channel, "publish: no active receivers");
    0
}

impl Default for WsHub {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
#[path = "hub/tests.rs"]
mod tests;
