use crate::diagnostics::{request_failure, sanitize_delivery_record};
use crate::outbox::{OutboxReplay, WebhookOutbox};
use crate::security::{
    resolve_bark_target, resolve_public_target, signature, validate_public_https_target,
};
use arc_swap::ArcSwap;
use dashmap::DashSet;
use sha2::{Digest, Sha256};
use shared_types::{
    WebhookApplicationAck, WebhookConfig, WebhookConfigPatch, WebhookDeliveryRecord,
    WebhookDeliveryStatus, WebhookEvent, WebhookEventKind, WebhookProvider, WebhookRuntimeStatus,
};
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const HARD_QUEUE_CAPACITY: usize = 256;
const RECENT_LIMIT: usize = 50;
const SEEN_LIMIT: usize = 10_000;
const MAX_RESPONSE_BYTES: usize = 64 * 1_024;
const MAX_BARK_BODY_CHARS: usize = 900;
const MAX_DELIVERY_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone)]
struct PrivateConfig {
    public: WebhookConfig,
    target_url: Option<Arc<str>>,
    secret: Option<Arc<str>>,
}

#[derive(Debug, PartialEq, Eq)]
struct DeliveryResponse {
    status: u16,
    provider: WebhookProvider,
    application_ack: WebhookApplicationAck,
    message: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct DeliveryAttemptOutcome {
    delivered: bool,
    attempts: u8,
    response_status: Option<u16>,
    provider: WebhookProvider,
    application_ack: WebhookApplicationAck,
    response_message: Option<String>,
    error: Option<String>,
}

#[derive(Debug)]
pub struct WebhookDispatcher {
    config: ArcSwap<PrivateConfig>,
    official_bark_client: Option<reqwest::Client>,
    outbox: WebhookOutbox,
    sender: mpsc::Sender<WebhookEvent>,
    receiver: Mutex<mpsc::Receiver<WebhookEvent>>,
    enqueue_lock: Mutex<()>,
    queue_depth: AtomicUsize,
    delivered_total: AtomicU64,
    failed_total: AtomicU64,
    dropped_total: AtomicU64,
    recent: Mutex<VecDeque<WebhookDeliveryRecord>>,
    pending_completions: Mutex<VecDeque<WebhookDeliveryRecord>>,
    seen: DashSet<String>,
    seen_order: Mutex<VecDeque<String>>,
    requires_bootstrap: AtomicBool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookError(pub String);

impl std::fmt::Display for WebhookError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for WebhookError {}

impl Default for WebhookDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookDispatcher {
    pub fn new() -> Self {
        Self::with_outbox(WebhookOutbox::default(), false)
    }

    pub async fn initialize(path: Option<PathBuf>) -> Result<Self, WebhookError> {
        let (outbox, replay) = WebhookOutbox::initialize(path)
            .await
            .map_err(WebhookError)?;
        Self::from_replay(outbox, replay)
    }

    fn with_outbox(outbox: WebhookOutbox, requires_bootstrap: bool) -> Self {
        let (sender, receiver) = mpsc::channel(HARD_QUEUE_CAPACITY);
        Self {
            config: ArcSwap::from_pointee(PrivateConfig {
                public: WebhookConfig::default(),
                target_url: None,
                secret: None,
            }),
            official_bark_client: build_official_bark_client(),
            outbox,
            sender,
            receiver: Mutex::new(receiver),
            enqueue_lock: Mutex::new(()),
            queue_depth: AtomicUsize::new(0),
            delivered_total: AtomicU64::new(0),
            failed_total: AtomicU64::new(0),
            dropped_total: AtomicU64::new(0),
            recent: Mutex::new(VecDeque::with_capacity(RECENT_LIMIT)),
            pending_completions: Mutex::new(VecDeque::new()),
            seen: DashSet::new(),
            seen_order: Mutex::new(VecDeque::with_capacity(SEEN_LIMIT)),
            requires_bootstrap: AtomicBool::new(requires_bootstrap),
        }
    }

    fn from_replay(outbox: WebhookOutbox, replay: OutboxReplay) -> Result<Self, WebhookError> {
        if replay.pending.len() > HARD_QUEUE_CAPACITY {
            return Err(WebhookError(format!(
                "webhook outbox contains {} pending events; hard queue capacity is {HARD_QUEUE_CAPACITY}",
                replay.pending.len()
            )));
        }
        let mut dispatcher = Self::with_outbox(outbox, replay.requires_bootstrap);
        for event_id in replay.event_ids {
            dispatcher.seen.insert(event_id.clone());
            dispatcher.seen_order.get_mut().push_back(event_id);
        }
        while dispatcher.seen_order.get_mut().len() > SEEN_LIMIT {
            if let Some(expired) = dispatcher.seen_order.get_mut().pop_front() {
                dispatcher.seen.remove(&expired);
            }
        }
        for event in replay.pending {
            dispatcher.sender.try_send(event).map_err(|_| {
                WebhookError("webhook outbox pending replay exceeded queue capacity".to_owned())
            })?;
            dispatcher.queue_depth.fetch_add(1, Ordering::Relaxed);
        }
        *dispatcher.recent.get_mut() = replay
            .recent
            .into_iter()
            .map(sanitize_delivery_record)
            .collect();
        dispatcher
            .delivered_total
            .store(replay.delivered_total, Ordering::Relaxed);
        dispatcher
            .failed_total
            .store(replay.failed_total, Ordering::Relaxed);
        Ok(dispatcher)
    }

    pub fn durable_outbox(&self) -> bool {
        self.outbox.is_durable()
    }

    pub fn enabled_for(&self, kind: WebhookEventKind) -> bool {
        let config = self.config.load();
        config.public.enabled && config.target_url.is_some() && config.public.event_kinds.contains(&kind)
    }

    pub fn event_known(&self, event_id: &str) -> bool {
        self.seen.contains(event_id)
    }

    pub fn requires_outbox_bootstrap(&self) -> bool {
        self.requires_bootstrap.load(Ordering::Acquire)
    }

    pub async fn bootstrap_event_ids(&self, event_ids: Vec<String>) -> Result<(), WebhookError> {
        if !self.requires_outbox_bootstrap() {
            return Ok(());
        }
        self.outbox
            .bootstrap(event_ids.clone())
            .await
            .map_err(WebhookError)?;
        for event_id in event_ids {
            self.remember_seen(event_id).await;
        }
        self.requires_bootstrap.store(false, Ordering::Release);
        Ok(())
    }

    pub async fn status(&self, now_ms: i64) -> WebhookRuntimeStatus {
        let config = self.config.load_full();
        WebhookRuntimeStatus {
            config: config.public.clone(),
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            delivered_total: self.delivered_total.load(Ordering::Relaxed),
            failed_total: self.failed_total.load(Ordering::Relaxed),
            dropped_total: self.dropped_total.load(Ordering::Relaxed),
            recent_deliveries: self
                .recent
                .lock()
                .await
                .iter()
                .cloned()
                .map(sanitize_delivery_record)
                .collect(),
            updated_at_ms: now_ms,
        }
    }

    pub fn update_config(&self, patch: WebhookConfigPatch) -> Result<WebhookConfig, WebhookError> {
        let current = self.config.load_full();
        let mut public = current.public.clone();
        let mut target_url = current.target_url.clone();
        let mut secret = current.secret.clone();
        if let Some(value) = patch.enabled {
            public.enabled = value;
        }
        if let Some(value) = patch.provider {
            public.provider = value;
        }
        if let Some(value) = patch.url {
            let value = value.trim();
            if value.contains("***") {
                return Err(WebhookError(
                    "redacted webhook URL cannot be saved as a delivery target".to_owned(),
                ));
            }
            target_url = if value.is_empty() {
                None
            } else {
                Some(Arc::from(normalize_target(public.provider, value)?))
            };
        } else if patch.provider.is_some() {
            target_url = target_url
                .as_deref()
                .map(|value| normalize_target(public.provider, value).map(Arc::from))
                .transpose()?;
        }
        if let Some(value) = patch.event_kinds {
            public.event_kinds = value;
        }
        if let Some(value) = patch.timeout_ms {
            public.timeout_ms = value;
        }
        if let Some(value) = patch.max_attempts {
            public.max_attempts = value;
        }
        if let Some(value) = patch.base_backoff_ms {
            public.base_backoff_ms = value;
        }
        if let Some(value) = patch.queue_capacity {
            public.queue_capacity = value;
        }
        if patch.clear_secret.unwrap_or(false) {
            secret = None;
        }
        if let Some(value) = patch.secret.filter(|value| !value.trim().is_empty()) {
            secret = Some(Arc::from(value));
        }
        public.url_configured = target_url.is_some();
        public.url = target_url
            .as_deref()
            .map(redacted_target_label)
            .unwrap_or_default();
        public.secret_configured = secret.is_some();
        validate_config(&public, target_url.as_deref(), secret.as_deref())?;
        self.config.store(Arc::new(PrivateConfig {
            public: public.clone(),
            target_url,
            secret,
        }));
        Ok(public)
    }

    pub async fn enqueue(&self, event: WebhookEvent, force: bool) -> Result<(), WebhookError> {
        let _serial = self.enqueue_lock.lock().await;
        let config = self.config.load_full();
        if !force && (!config.public.enabled || !config.public.event_kinds.contains(&event.kind)) {
            return Ok(());
        }
        if config.target_url.is_none() {
            return Err(WebhookError("webhook URL is required".to_owned()));
        }
        if config.public.provider == WebhookProvider::Generic && config.secret.is_none() {
            return Err(WebhookError(
                "generic webhook signing secret is required".to_owned(),
            ));
        }
        if self.seen.contains(&event.id) {
            return Ok(());
        }
        if self.queue_depth.load(Ordering::Relaxed) >= config.public.queue_capacity {
            self.dropped_total.fetch_add(1, Ordering::Relaxed);
            return Err(WebhookError("webhook queue is full".to_owned()));
        }
        let event_id = event.id.clone();
        if !self
            .outbox
            .reserve(event.clone())
            .await
            .map_err(WebhookError)?
        {
            self.remember_seen(event_id).await;
            return Ok(());
        }
        if self.sender.try_send(event).is_err() {
            let release = self.outbox.release_pending(event_id).await;
            self.dropped_total.fetch_add(1, Ordering::Relaxed);
            release.map_err(WebhookError)?;
            return Err(WebhookError("webhook queue is full".to_owned()));
        }
        self.remember_seen(event_id).await;
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub async fn process_next(&self, max_wait: std::time::Duration) -> Result<bool, WebhookError> {
        self.flush_pending_completion().await?;
        let event = {
            let mut receiver = self.receiver.lock().await;
            tokio::time::timeout(max_wait, receiver.recv())
                .await
                .ok()
                .flatten()
        };
        let Some(event) = event else {
            return Ok(false);
        };
        self.queue_depth.fetch_sub(1, Ordering::Relaxed);
        let record = self.deliver(&event).await;
        self.record(record.clone()).await;
        if let Err(error) = self.outbox.complete(record.clone()).await {
            self.pending_completions.lock().await.push_back(record);
            return Err(WebhookError(error));
        }
        Ok(true)
    }

    async fn deliver(&self, event: &WebhookEvent) -> WebhookDeliveryRecord {
        let config = self.config.load_full();
        let outcome = attempt_delivery(
            config.public.provider,
            config.public.max_attempts,
            config.public.base_backoff_ms,
            || send_once(&config, event, self.official_bark_client.as_ref()),
        )
        .await;
        let status = if outcome.delivered {
            self.delivered_total.fetch_add(1, Ordering::Relaxed);
            WebhookDeliveryStatus::Delivered
        } else {
            self.failed_total.fetch_add(1, Ordering::Relaxed);
            WebhookDeliveryStatus::Failed
        };
        sanitize_delivery_record(delivery(event, status, outcome))
    }

    async fn record(&self, record: WebhookDeliveryRecord) {
        let mut recent = self.recent.lock().await;
        recent.push_front(record);
        recent.truncate(RECENT_LIMIT);
    }

    async fn remember_seen(&self, event_id: String) {
        if !self.seen.insert(event_id.clone()) {
            return;
        }
        let mut order = self.seen_order.lock().await;
        order.push_back(event_id);
        while order.len() > SEEN_LIMIT {
            if let Some(expired) = order.pop_front() {
                self.seen.remove(&expired);
            }
        }
    }

    async fn flush_pending_completion(&self) -> Result<(), WebhookError> {
        let record = self.pending_completions.lock().await.front().cloned();
        let Some(record) = record else {
            return Ok(());
        };
        self.outbox
            .complete(record.clone())
            .await
            .map_err(WebhookError)?;
        let mut pending = self.pending_completions.lock().await;
        if pending
            .front()
            .is_some_and(|queued| queued.event_id == record.event_id)
        {
            pending.pop_front();
        }
        Ok(())
    }
}

async fn attempt_delivery<Send, SendFuture>(
    provider: WebhookProvider,
    max_attempts: u8,
    base_backoff_ms: u64,
    mut send: Send,
) -> DeliveryAttemptOutcome
where
    Send: FnMut() -> SendFuture,
    SendFuture: Future<Output = Result<DeliveryResponse, String>>,
{
    let mut outcome = DeliveryAttemptOutcome {
        delivered: false,
        attempts: 0,
        response_status: None,
        provider,
        application_ack: WebhookApplicationAck::Unknown,
        response_message: None,
        error: None,
    };
    for attempt in 1..=max_attempts {
        outcome.attempts = attempt;
        match send().await {
            Ok(response) => {
                let response_error = delivery_response_error(&response);
                outcome.provider = response.provider;
                outcome.response_status = Some(response.status);
                outcome.application_ack = response.application_ack;
                outcome.response_message = response.message;
                if (200..300).contains(&response.status)
                    && matches!(
                        response.application_ack,
                        WebhookApplicationAck::Accepted | WebhookApplicationAck::TransportOnly
                    )
                {
                    outcome.delivered = true;
                    outcome.error = None;
                    return outcome;
                }
                outcome.error = Some(response_error);
                if !retryable_status(response.status)
                    || matches!(
                        response.application_ack,
                        WebhookApplicationAck::Rejected | WebhookApplicationAck::InvalidResponse
                    )
                {
                    return outcome;
                }
            }
            Err(error) => {
                outcome.response_status = None;
                outcome.error = Some(error);
            }
        }
        if attempt < max_attempts && base_backoff_ms > 0 {
            let factor = 1_u64 << u32::from(attempt.saturating_sub(1).min(10));
            tokio::time::sleep(std::time::Duration::from_millis(
                base_backoff_ms.saturating_mul(factor),
            ))
            .await;
        }
    }
    outcome
}

fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500..=599)
}

fn delivery_response_error(response: &DeliveryResponse) -> String {
    let message = response.message.as_deref().unwrap_or("no response message");
    if (200..300).contains(&response.status) {
        format!(
            "webhook application acknowledgement {:?}: {message}",
            response.application_ack
        )
    } else {
        format!("webhook returned HTTP {}: {message}", response.status)
    }
}

fn normalize_target(provider: WebhookProvider, raw: &str) -> Result<String, WebhookError> {
    let mut url = validate_public_https_target(raw).map_err(WebhookError)?;
    if provider == WebhookProvider::Bark
        && url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("api.day.app"))
    {
        let device_key = url
            .path_segments()
            .and_then(|mut segments| segments.find(|segment| !segment.is_empty()))
            .ok_or_else(|| WebhookError("Bark URL must include a device key".to_owned()))?
            .to_owned();
        if device_key.eq_ignore_ascii_case("push") || device_key.eq_ignore_ascii_case("mcp") {
            return Err(WebhookError(
                "Bark URL must include the device key in its path".to_owned(),
            ));
        }
        let canonical_path = format!("/{device_key}");
        url.set_path(&canonical_path);
    }
    Ok(url.to_string())
}

fn redacted_target_label(raw: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw) else {
        return "已配置（地址已隐藏）".to_owned();
    };
    url.set_path("/***");
    url.set_query(None);
    url.to_string()
}

/// Encodes the provider payload without sending it or exposing the configured target.
pub fn delivery_body(provider: WebhookProvider, event: &WebhookEvent) -> Result<Vec<u8>, String> {
    let payload = match provider {
        WebhookProvider::Generic => serde_json::to_value(event),
        WebhookProvider::Bark => {
            let mut body = serde_json::json!({
                "title": bark_title(event.kind),
                "body": bark_body(event),
                "id": bark_collapse_id(&event.id),
            });
            // Bark's explicit copy action retains the whole code even when body text is bounded.
            if event.kind == WebhookEventKind::Opportunity {
                if let Some(code) = event
                    .payload
                    .get("handoffCode")
                    .and_then(serde_json::Value::as_str)
                    .filter(|code| {
                        shared_types::ExecutionArtifactValidationRequest::from_handoff_code(code)
                            .is_ok()
                    })
                {
                    body["copy"] = code.into();
                }
            }
            Ok(body)
        }
    }
    .map_err(|error| format!("webhook encode failed: {error}"))?;
    serde_json::to_vec(&payload).map_err(|error| format!("webhook encode failed: {error}"))
}

fn bark_collapse_id(event_id: &str) -> String {
    hex::encode(Sha256::digest(event_id.as_bytes()))
}

fn bark_title(kind: WebhookEventKind) -> &'static str {
    match kind {
        WebhookEventKind::Opportunity => "CROSSLINE · 确定性机会",
        WebhookEventKind::OpportunityMonitor => "CROSSLINE · 价差与充提监控",
        WebhookEventKind::AutomationDecision => "CROSSLINE · 自动化决策",
        WebhookEventKind::ExecutionResult => "CROSSLINE · 执行结果",
        WebhookEventKind::Compensation => "CROSSLINE · 补偿动作",
        WebhookEventKind::RiskAlert => "CROSSLINE · 风险告警",
        WebhookEventKind::SystemDegradation => "CROSSLINE · 系统降级",
        WebhookEventKind::OnchainSpread => "CROSSLINE · 链上价差",
        WebhookEventKind::StockSpread => "CROSSLINE · 股票价差观察",
        WebhookEventKind::Test => "CROSSLINE · Webhook 测试",
    }
}

fn bark_body(event: &WebhookEvent) -> String {
    let message = (event.kind == WebhookEventKind::Opportunity)
        .then(|| opportunity_bark_body(&event.payload))
        .flatten()
        .or_else(|| {
            event
                .payload
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| opportunity_bark_body(&event.payload))
        .unwrap_or_else(|| compact_json(&event.payload));
    message.chars().take(MAX_BARK_BODY_CHARS).collect()
}

fn opportunity_bark_body(payload: &serde_json::Value) -> Option<String> {
    let symbol = payload.get("symbol")?.as_str()?;
    let artifact_id = payload
        .get("artifactId")
        .and_then(serde_json::Value::as_str);
    let net = payload
        .get("expectedNetEdgeUsd")
        .and_then(serde_json::Value::as_f64)
        .map(|value| format!("${value:.4}"))
        .unwrap_or_else(|| "待确认".to_owned());
    let cost = payload
        .get("expectedTotalCostUsd")
        .and_then(serde_json::Value::as_f64)
        .map(|value| format!("${value:.4}"))
        .unwrap_or_else(|| "待确认".to_owned());
    let route = artifact_route(payload).unwrap_or_else(|| "双腿路由待确认".to_owned());
    let validity = artifact_validity(payload);
    let evidence = artifact_evidence_summary(payload);
    let mut lines = vec![
        "预检机会 · 预期收益不等于保证盈利".to_owned(),
        format!("{symbol} · {route}"),
        format!("费后预期净收益 {net} · 预计成本 {cost}"),
        format!("证据 {evidence} · 有效期 {validity}"),
    ];
    if let Some(detail) = payload
        .get("transfer")
        .and_then(|row| row.get("detail"))
        .and_then(serde_json::Value::as_str)
    {
        lines.push(format!("充提：{detail}"));
    }
    if let Some(conditions) = artifact_invalidation_summary(payload) {
        lines.push(format!("失效条件 {conditions}"));
    }
    if let Some(id) = artifact_id {
        lines.push(format!("工件 {id}"));
    }
    if payload
        .get("handoffCode")
        .and_then(serde_json::Value::as_str)
        .is_some()
    {
        lines.push("复制校验码 → 对冲执行；需重新核验当前报价，不会直接下单".to_owned());
    } else if let Some(command) = payload
        .get("validationCommand")
        .and_then(serde_json::Value::as_str)
    {
        lines.push(format!("只读校验 {command}"));
    }
    Some(lines.join("\n"))
}

fn artifact_route(payload: &serde_json::Value) -> Option<String> {
    if let Some(route) = payload.get("route").and_then(serde_json::Value::as_str) {
        return Some(route.to_owned());
    }
    let legs = payload.get("legs")?.as_array()?;
    let route = legs
        .iter()
        .filter_map(|leg| {
            let venue = leg.get("venue")?.as_str()?.to_uppercase();
            let symbol = leg.get("symbol")?.as_str()?;
            let side = match leg.get("side").and_then(serde_json::Value::as_str) {
                Some("buy") => "买",
                Some("sell") => "卖",
                _ => "方向待确认",
            };
            Some(format!("{venue} {side} {symbol}"))
        })
        .collect::<Vec<_>>();
    (!route.is_empty()).then(|| route.join(" / "))
}

fn artifact_validity(payload: &serde_json::Value) -> String {
    let Some(mut expires) = payload
        .get("expiresAtMs")
        .and_then(serde_json::Value::as_i64)
    else {
        return "未知".to_owned();
    };
    let Some(legs) = payload
        .get("legs")
        .and_then(serde_json::Value::as_array)
        .filter(|legs| legs.len() == 2)
    else {
        return "行情时效待确认".to_owned();
    };
    for leg in legs {
        let Some(observed) = leg
            .get("marketObservedAtMs")
            .and_then(serde_json::Value::as_i64)
            .filter(|observed| *observed > 0)
        else {
            return "行情时效待确认".to_owned();
        };
        expires =
            expires.min(observed.saturating_add(shared_types::HEDGE_PREVIEW_MARKET_MAX_AGE_MS));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(i64::MAX);
    if expires <= now {
        "已过期，需重新构建".to_owned()
    } else {
        format!(
            "生成消息时剩余 {}s；接收后须重新校验",
            (expires - now) / 1_000
        )
    }
}

fn artifact_evidence_summary(payload: &serde_json::Value) -> String {
    let Some(rows) = payload
        .get("evidence")
        .and_then(serde_json::Value::as_array)
    else {
        return "待确认".to_owned();
    };
    let passed = rows
        .iter()
        .filter(|row| row.get("passed").and_then(serde_json::Value::as_bool) == Some(true))
        .count();
    format!("{passed}/{}", rows.len())
}

fn artifact_invalidation_summary(payload: &serde_json::Value) -> Option<String> {
    let rows = payload
        .get("invalidationConditions")?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .take(2)
        .collect::<Vec<_>>();
    (!rows.is_empty()).then(|| rows.join("；"))
}

fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "事件详情编码失败".to_owned())
}

fn application_ack(
    provider: WebhookProvider,
    body: &[u8],
) -> (WebhookApplicationAck, Option<String>) {
    if provider == WebhookProvider::Generic {
        return (WebhookApplicationAck::TransportOnly, None);
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return (
            WebhookApplicationAck::InvalidResponse,
            Some("Bark acknowledgement is not valid JSON".to_owned()),
        );
    };
    let code = value.get("code").and_then(serde_json::Value::as_i64);
    let message = value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if code == Some(200) {
        (WebhookApplicationAck::Accepted, message)
    } else {
        (WebhookApplicationAck::Rejected, message)
    }
}

async fn send_once(
    config: &PrivateConfig,
    event: &WebhookEvent,
    official_bark_client: Option<&reqwest::Client>,
) -> Result<DeliveryResponse, String> {
    let target = config
        .target_url
        .as_deref()
        .ok_or_else(|| "webhook URL is missing".to_owned())?;
    let url = validate_public_https_target(target)?;
    let addresses = match config.public.provider {
        WebhookProvider::Bark => resolve_bark_target(&url).await?,
        WebhookProvider::Generic => resolve_public_target(&url).await?,
    };
    let host = url
        .host_str()
        .ok_or_else(|| "webhook host is required".to_owned())?;
    let pinned_client;
    let client = if is_official_bark_target(config.public.provider, &url) {
        official_bark_client.ok_or_else(|| "Bark HTTP client is unavailable".to_owned())?
    } else {
        pinned_client = build_pinned_client(host, addresses[0])?;
        &pinned_client
    };
    let body = delivery_body(config.public.provider, event)?;
    let mut request = client
        .post(url)
        .timeout(std::time::Duration::from_millis(config.public.timeout_ms))
        .header("content-type", "application/json")
        .header("x-crossline-event-id", &event.id)
        .header("x-crossline-event-version", &event.version)
        .header("x-crossline-timestamp", event.occurred_at_ms.to_string());
    if let Some(secret) = config.secret.as_deref() {
        request = request.header(
            "x-crossline-signature",
            signature(secret.as_bytes(), event.occurred_at_ms, &body)?,
        );
    }
    let response = request
        .body(body)
        .send()
        .await
        .map_err(|error| request_failure(&error))?;
    let status = response.status().as_u16();
    let content_length = response.content_length().unwrap_or(0);
    if content_length > MAX_RESPONSE_BYTES as u64 {
        return Err("webhook response exceeded the bounded acknowledgement size".to_owned());
    }
    let response_body = response
        .bytes()
        .await
        .map_err(|error| request_failure(&error))?;
    if response_body.len() > MAX_RESPONSE_BYTES {
        return Err("webhook response exceeded the bounded acknowledgement size".to_owned());
    }
    let (application_ack, message) = application_ack(config.public.provider, &response_body);
    Ok(DeliveryResponse {
        status,
        provider: config.public.provider,
        application_ack,
        message,
    })
}

fn build_official_bark_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(2)
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .build()
        .ok()
}

fn build_pinned_client(
    host: &str,
    address: std::net::SocketAddr,
) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve(host, address)
        .build()
        .map_err(|_| "webhook client build failed".to_owned())
}

fn is_official_bark_target(provider: WebhookProvider, url: &url::Url) -> bool {
    provider == WebhookProvider::Bark
        && url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("api.day.app"))
}

fn validate_config(
    config: &WebhookConfig,
    target_url: Option<&str>,
    secret: Option<&str>,
) -> Result<(), WebhookError> {
    if let Some(target_url) = target_url {
        validate_public_https_target(target_url).map_err(WebhookError)?;
    }
    if config.timeout_ms < 100
        || config.timeout_ms > MAX_DELIVERY_TIMEOUT_MS
        || config.max_attempts == 0
        || config.max_attempts > 5
        || config.base_backoff_ms < 100
        || config.base_backoff_ms > 10_000
        || config.queue_capacity == 0
        || config.queue_capacity > HARD_QUEUE_CAPACITY
    {
        return Err(WebhookError(
            "webhook delivery limits are invalid".to_owned(),
        ));
    }
    if config.enabled && target_url.is_none() {
        return Err(WebhookError("enabled webhook requires a URL".to_owned()));
    }
    if config.enabled && config.provider == WebhookProvider::Generic && secret.is_none() {
        return Err(WebhookError(
            "enabled generic webhook requires a signing secret".to_owned(),
        ));
    }
    Ok(())
}

fn delivery(
    event: &WebhookEvent,
    status: WebhookDeliveryStatus,
    outcome: DeliveryAttemptOutcome,
) -> WebhookDeliveryRecord {
    WebhookDeliveryRecord {
        event_id: event.id.clone(),
        kind: event.kind,
        provider: outcome.provider,
        status,
        attempts: outcome.attempts,
        response_status: outcome.response_status,
        application_ack: outcome.application_ack,
        response_message: outcome.response_message,
        error: outcome.error,
        updated_at_ms: now_ms(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| {
            i64::try_from(value.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod stock_alert_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::WEBHOOK_EVENT_VERSION;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn disabled_dispatcher_does_not_queue_normal_events() -> Result<(), WebhookError> {
        let dispatcher = WebhookDispatcher::new();
        dispatcher.enqueue(event("one"), false).await?;
        assert_eq!(dispatcher.status(1).await.queue_depth, 0);
        Ok(())
    }

    #[tokio::test]
    async fn event_ids_are_idempotent_and_queue_limit_is_enforced() -> Result<(), WebhookError> {
        let dispatcher = WebhookDispatcher::new();
        dispatcher.update_config(WebhookConfigPatch {
            url: Some("https://example.com/crossline".to_owned()),
            secret: Some("test-secret".to_owned()),
            queue_capacity: Some(1),
            ..WebhookConfigPatch::default()
        })?;

        dispatcher.enqueue(event("one"), true).await?;
        dispatcher.enqueue(event("one"), true).await?;
        assert!(dispatcher.enqueue(event("two"), true).await.is_err());
        let status = dispatcher.status(2).await;
        assert_eq!(status.queue_depth, 1);
        assert_eq!(status.dropped_total, 1);
        Ok(())
    }

    #[test]
    fn invalid_retry_and_timeout_limits_fail_validation() {
        let dispatcher = WebhookDispatcher::new();
        assert!(dispatcher
            .update_config(WebhookConfigPatch {
                timeout_ms: Some(99),
                max_attempts: Some(6),
                ..WebhookConfigPatch::default()
            })
            .is_err());
        assert!(dispatcher
            .update_config(WebhookConfigPatch {
                timeout_ms: Some(MAX_DELIVERY_TIMEOUT_MS + 1),
                ..WebhookConfigPatch::default()
            })
            .is_err());
    }

    #[test]
    fn retries_only_transient_http_failures() {
        assert!(retryable_status(408));
        assert!(retryable_status(429));
        assert!(retryable_status(503));
        assert!(!retryable_status(400));
        assert!(!retryable_status(401));
        assert!(!retryable_status(404));
    }

    #[tokio::test]
    async fn transient_failure_retries_until_delivery() {
        let calls = AtomicUsize::new(0);
        let outcome = attempt_delivery(WebhookProvider::Generic, 3, 0, || {
            let call = calls.fetch_add(1, Ordering::Relaxed);
            async move {
                Ok(response(
                    if call == 0 { 503 } else { 204 },
                    WebhookApplicationAck::TransportOnly,
                ))
            }
        })
        .await;

        assert_eq!(outcome.attempts, 2);
        assert_eq!(outcome.response_status, Some(204));
        assert!(outcome.delivered);
    }

    #[tokio::test]
    async fn permanent_client_error_stops_without_retrying() {
        let calls = AtomicUsize::new(0);
        let outcome = attempt_delivery(WebhookProvider::Generic, 5, 0, || {
            calls.fetch_add(1, Ordering::Relaxed);
            async { Ok(response(400, WebhookApplicationAck::Rejected)) }
        })
        .await;

        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(outcome.attempts, 1);
        assert_eq!(outcome.response_status, Some(400));
        assert!(!outcome.delivered);
    }

    #[tokio::test]
    async fn timeout_is_bounded_and_visible_in_recent_diagnostics() {
        let dispatcher = WebhookDispatcher::new();
        let outcome = attempt_delivery(WebhookProvider::Bark, 3, 0, || async {
            Err::<DeliveryResponse, _>("webhook delivery failed: operation timed out".to_owned())
        })
        .await;
        assert_eq!(outcome.attempts, 3);
        assert_eq!(outcome.provider, WebhookProvider::Bark);
        assert!(!outcome.delivered);

        dispatcher
            .record(delivery(
                &event("timeout"),
                WebhookDeliveryStatus::Failed,
                outcome,
            ))
            .await;
        let status = dispatcher.status(3).await;
        assert_eq!(status.recent_deliveries.len(), 1);
        assert!(status.recent_deliveries[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("timed out")));
    }

    #[tokio::test]
    async fn idle_queue_poll_returns_at_its_deadline() {
        let dispatcher = WebhookDispatcher::new();
        let started = tokio::time::Instant::now();

        assert_eq!(
            dispatcher
                .process_next(std::time::Duration::from_millis(2))
                .await,
            Ok(false)
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[tokio::test]
    async fn public_status_never_serializes_the_signing_secret() {
        let dispatcher = WebhookDispatcher::new();
        let updated = dispatcher.update_config(WebhookConfigPatch {
            url: Some("https://example.com/crossline".to_owned()),
            secret: Some("do-not-leak-this-secret".to_owned()),
            ..WebhookConfigPatch::default()
        });
        assert!(updated.is_ok());

        let encoded = serde_json::to_string(&dispatcher.status(4).await).unwrap_or_default();
        assert!(!encoded.contains("do-not-leak-this-secret"));
        assert!(encoded.contains("secretConfigured"));
    }

    #[tokio::test]
    async fn bark_target_and_device_key_are_never_returned() -> Result<(), WebhookError> {
        let dispatcher = WebhookDispatcher::new();
        dispatcher.update_config(WebhookConfigPatch {
            provider: Some(WebhookProvider::Bark),
            url: Some("https://api.day.app/device-key/private-body?group=crossline".to_owned()),
            enabled: Some(true),
            ..WebhookConfigPatch::default()
        })?;

        let status = dispatcher.status(5).await;
        let private = dispatcher.config.load_full();
        let encoded = serde_json::to_string(&status).unwrap_or_default();
        assert!(status.config.url_configured);
        assert_eq!(status.config.url, "https://api.day.app/***");
        assert!(!encoded.contains("device-key"));
        assert!(!encoded.contains("private-body"));
        assert!(!status.config.secret_configured);
        assert_eq!(
            private.target_url.as_deref(),
            Some("https://api.day.app/device-key?group=crossline")
        );
        Ok(())
    }

    #[test]
    fn bark_ack_requires_application_success_code() {
        assert_eq!(
            application_ack(
                WebhookProvider::Bark,
                br#"{"code":200,"message":"success"}"#
            )
            .0,
            WebhookApplicationAck::Accepted
        );
        assert_eq!(
            application_ack(
                WebhookProvider::Bark,
                br#"{"code":400,"message":"bad key"}"#
            )
            .0,
            WebhookApplicationAck::Rejected
        );
        assert_eq!(
            application_ack(WebhookProvider::Bark, b"not-json").0,
            WebhookApplicationAck::InvalidResponse
        );
        assert_eq!(
            application_ack(WebhookProvider::Generic, b"").0,
            WebhookApplicationAck::TransportOnly
        );
    }

    #[test]
    fn only_the_official_bark_host_uses_the_reusable_client() {
        let official = url::Url::parse("https://api.day.app/device-key");
        let self_hosted = url::Url::parse("https://bark.example.com/device-key");

        assert!(official.is_ok_and(|url| is_official_bark_target(WebhookProvider::Bark, &url)));
        assert!(self_hosted.is_ok_and(|url| {
            !is_official_bark_target(WebhookProvider::Bark, &url)
                && !is_official_bark_target(WebhookProvider::Generic, &url)
        }));
    }

    #[test]
    fn bark_payload_uses_event_content_and_stable_notification_id() {
        let event = WebhookEvent {
            id: "deterministic-opportunity-1".to_owned(),
            version: WEBHOOK_EVENT_VERSION.to_owned(),
            kind: WebhookEventKind::Opportunity,
            occurred_at_ms: 1,
            payload: serde_json::json!({
                "artifactId": "artifact-1",
                "symbol": "BTC",
                "expectedNetEdgeUsd": 1.25,
                "expectedTotalCostUsd": 0.25,
                "generatedAtMs": 30_000,
                "expiresAtMs": 60_000,
                "legs": [
                    {"venue": "binance", "symbol": "BTCUSDT", "side": "buy"},
                    {"venue": "okx", "symbol": "BTC-USDT-SWAP", "side": "sell"}
                ],
                "evidence": [
                    {"passed": true},
                    {"passed": true}
                ],
                "invalidationConditions": ["任一腿行情超过 30 秒", "盘口深度低于目标"],
                "validationCommand": "curl --request POST http://127.0.0.1:8000/api/automation/execution-artifacts/validate",
            }),
        };

        let body = delivery_body(WebhookProvider::Bark, &event).unwrap_or_default();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        assert_eq!(value["id"], bark_collapse_id(&event.id));
        assert_eq!(value["id"].as_str().map(str::len), Some(64));
        let text = value["body"].as_str().unwrap_or_default();
        assert!(text.contains("BTC"));
        assert!(text.contains("BINANCE 买 BTCUSDT / OKX 卖 BTC-USDT-SWAP"));
        assert!(text.contains("预计成本 $0.2500"));
        assert!(text.contains("证据 2/2 · 有效期 行情时效待确认"));
        assert!(text.contains("只读校验 curl"));
        assert_eq!(value["title"], "CROSSLINE · 确定性机会");
    }

    #[test]
    fn bark_title_keeps_candidate_monitoring_separate_from_deterministic_opportunities() {
        assert_eq!(
            bark_title(WebhookEventKind::OpportunityMonitor),
            "CROSSLINE · 价差与充提监控"
        );
        assert_ne!(
            bark_title(WebhookEventKind::OpportunityMonitor),
            bark_title(WebhookEventKind::Opportunity)
        );
    }

    #[test]
    fn bark_collapse_id_is_stable_and_within_the_apns_byte_limit() {
        let long_id = "execution-run-hedge-c5e0a1ad-374d-4683-be9d-5ec14dbffa81-1785667361696";
        let id = bark_collapse_id(long_id);

        assert_eq!(id.len(), 64);
        assert_eq!(id, bark_collapse_id(long_id));
        assert_ne!(id, bark_collapse_id("another-event"));
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    fn response(status: u16, application_ack: WebhookApplicationAck) -> DeliveryResponse {
        DeliveryResponse {
            status,
            provider: WebhookProvider::Generic,
            application_ack,
            message: None,
        }
    }

    fn event(id: &str) -> WebhookEvent {
        WebhookEvent {
            id: id.to_owned(),
            version: WEBHOOK_EVENT_VERSION.to_owned(),
            kind: shared_types::WebhookEventKind::Test,
            occurred_at_ms: 1,
            payload: serde_json::json!({ "ok": true }),
        }
    }
}
