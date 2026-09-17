//! 共享 HTTP 客户端。
//!
//! 所有交易所共用一份 [`reqwest::Client`] 单例（连接池），通过 [`HttpClient`]
//! 提供统一的重试 / 429 退避 / 超时 / 限频钩子。

use crate::error::{ExchangeError, ExchangeResult};
use crate::http_metrics::{record_http_outcome, record_http_request, HttpOutcomeSample};
use crate::services::host_gate::HostGate;
use crate::services::rate_limiter::RateLimiter;
use crate::venue_spec::{endpoint_weight, HttpMethod as SpecHttpMethod};
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{Client, Method, RequestBuilder, Response};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{debug, warn};

/// 默认超时（秒）。
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// 默认连接超时（秒）。
pub const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
/// 默认重试次数。
pub const DEFAULT_MAX_RETRIES: u32 = 3;
/// 默认指数退避基数（毫秒）。
pub const DEFAULT_BASE_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_EXPONENT: u32 = 10;
const MAX_REQUEST_CONTEXT_ITEMS: usize = 4;
const MAX_REQUEST_CONTEXT_VALUE_CHARS: usize = 80;
const MAX_REQUEST_CONTEXT_BODY_BYTES: usize = 16 * 1024;
const PATH_ID_TEMPLATE: &str = "{id}";
const GATE_RATE_LIMIT_RESET_HEADER: &str = "x-gate-ratelimit-reset-timestamp";
const MIN_UNIX_SECONDS: i64 = 1_000_000_000;
const UNIX_MILLISECONDS_THRESHOLD: i64 = 100_000_000_000;

#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    exchange: String,
    max_retries: u32,
    base_backoff_ms: u64,
    timeout_secs: u64,
    rate_limiter: Option<Arc<RateLimiter>>,
}

impl fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpClient")
            .field("exchange", &self.exchange)
            .field("max_retries", &self.max_retries)
            .field("base_backoff_ms", &self.base_backoff_ms)
            .field("timeout_secs", &self.timeout_secs)
            .field("has_rate_limiter", &self.rate_limiter.is_some())
            .finish_non_exhaustive()
    }
}

pub struct HttpClientBuilder {
    exchange: String,
    timeout_secs: u64,
    connect_timeout_secs: u64,
    max_retries: u32,
    base_backoff_ms: u64,
    pool_max_idle_per_host: usize,
    rate_limiter: Option<Arc<RateLimiter>>,
    default_headers: HeaderMap,
}

impl fmt::Debug for HttpClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpClientBuilder")
            .field("exchange", &self.exchange)
            .field("timeout_secs", &self.timeout_secs)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .field("max_retries", &self.max_retries)
            .field("base_backoff_ms", &self.base_backoff_ms)
            .field("pool_max_idle_per_host", &self.pool_max_idle_per_host)
            .field("has_rate_limiter", &self.rate_limiter.is_some())
            .field("default_headers", &self.default_headers)
            .finish()
    }
}

enum AttemptOutcome {
    Success {
        response: Response,
        transport_rtt_ms: u64,
    },
    Retry {
        error: ExchangeError,
        delay: Duration,
        transport_rtt_ms: u64,
    },
    Fail {
        error: ExchangeError,
        transport_rtt_ms: u64,
    },
}

struct RequestGate {
    gate: Arc<HostGate>,
    request_key: String,
    method: String,
    path: String,
    request_context: Vec<String>,
    weight: u32,
}

impl HttpClientBuilder {
    pub fn new(exchange: impl Into<String>) -> Self {
        Self {
            exchange: exchange.into(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
            max_retries: DEFAULT_MAX_RETRIES,
            base_backoff_ms: DEFAULT_BASE_BACKOFF_MS,
            pool_max_idle_per_host: 20,
            rate_limiter: None,
            default_headers: HeaderMap::new(),
        }
    }

    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn connect_timeout_secs(mut self, secs: u64) -> Self {
        self.connect_timeout_secs = secs;
        self
    }

    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    pub fn base_backoff_ms(mut self, ms: u64) -> Self {
        self.base_backoff_ms = ms;
        self
    }

    pub fn rate_limiter(mut self, rl: Arc<RateLimiter>) -> Self {
        self.rate_limiter = Some(rl);
        self
    }

    pub fn default_headers(mut self, headers: HeaderMap) -> Self {
        self.default_headers = headers;
        self
    }

    pub fn build(self) -> ExchangeResult<HttpClient> {
        let client = Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .connect_timeout(Duration::from_secs(self.connect_timeout_secs))
            .pool_max_idle_per_host(self.pool_max_idle_per_host)
            .default_headers(self.default_headers)
            .build()
            .map_err(|e| ExchangeError::Network(format!("client build: {e}")))?;

        Ok(HttpClient {
            client,
            exchange: self.exchange,
            max_retries: self.max_retries,
            base_backoff_ms: self.base_backoff_ms,
            timeout_secs: self.timeout_secs,
            rate_limiter: self.rate_limiter,
        })
    }
}

impl HttpClient {
    pub fn builder(exchange: impl Into<String>) -> HttpClientBuilder {
        HttpClientBuilder::new(exchange)
    }

    pub fn new(exchange: impl Into<String>) -> ExchangeResult<Self> {
        Self::builder(exchange).build()
    }

    pub fn name(&self) -> &str {
        &self.exchange
    }

    /// 直接获取底层 [`Client`]，用于无需重试的高级用法（如流式响应）。
    pub fn raw(&self) -> &Client {
        &self.client
    }

    /// 暴露 rate limiter，供需要绕过 `execute_with_retry` 的批量调用（如 OKX 单条
    /// funding-rate 快路径）手动节流。
    pub fn rate_limiter(&self) -> Option<&Arc<RateLimiter>> {
        self.rate_limiter.as_ref()
    }

    /// 构造 `RequestBuilder`，便于业务代码组合 query / body。
    pub fn request(&self, method: Method, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.client.request(method, url)
    }

    /// 执行请求并应用重试 / 退避策略。
    ///
    /// `build` 闭包必须每次返回新的 `RequestBuilder`，因为重试需要重新发送
    /// （reqwest 的 `RequestBuilder` 不可重用）。
    pub async fn execute_with_retry<F>(&self, mut build: F) -> ExchangeResult<Response>
    where
        F: FnMut() -> RequestBuilder,
    {
        self.execute_with_retry_inner(&mut build, None).await
    }

    /// Executes an idempotent request whose authentication material must be
    /// created only after host and rate-limit waits have completed.
    ///
    /// The unsigned endpoint supplies stable gate metadata. `build` is invoked
    /// immediately before every transport attempt, so timestamped signatures
    /// cannot expire while waiting for a weighted rate-limit permit.
    pub async fn execute_with_retry_fresh<F>(
        &self,
        method: Method,
        endpoint: &str,
        mut build: F,
    ) -> ExchangeResult<Response>
    where
        F: FnMut() -> ExchangeResult<RequestBuilder>,
    {
        let preview = self.request(method, endpoint);
        let gate = self.request_gate(&preview);
        let mut last_err: Option<ExchangeError> = None;

        for attempt in 0..self.max_retries {
            if let Err(error) = gate.gate.check_ready(common::time::now_ms()) {
                self.record_host_gate_reject_outcome(&gate, &error);
                return Err(error);
            }
            let _singleflight = gate.gate.singleflight(gate.request_key.clone()).await;
            self.wait_rate_limit(gate.weight).await;
            let req = build()?;
            record_http_request(&self.exchange, &gate.method, &gate.path, gate.weight);
            match self.send_attempt(req, attempt).await {
                AttemptOutcome::Success {
                    response,
                    transport_rtt_ms,
                } => {
                    self.record_http_success_outcome(
                        &gate,
                        response.status().as_u16(),
                        transport_rtt_ms,
                    );
                    gate.gate.record_success();
                    return Ok(response);
                }
                AttemptOutcome::Retry {
                    error,
                    delay,
                    transport_rtt_ms,
                } => {
                    self.record_http_error_outcome(&gate, &error, transport_rtt_ms, true);
                    gate.gate.record_error(&error, common::time::now_ms());
                    last_err = Some(error);
                    sleep(delay).await;
                }
                AttemptOutcome::Fail {
                    error,
                    transport_rtt_ms,
                } => {
                    self.record_http_error_outcome(&gate, &error, transport_rtt_ms, false);
                    gate.gate.record_error(&error, common::time::now_ms());
                    return Err(error);
                }
            }
        }

        Err(last_err.unwrap_or(ExchangeError::Network("max retries exceeded".into())))
    }

    /// Executes an idempotent request with an official query-dependent request weight.
    ///
    /// Use this only when one endpoint path has documented weights that vary by query
    /// shape. Host gating, singleflight, retries, and telemetry remain unchanged.
    pub async fn execute_with_retry_weighted<F>(
        &self,
        weight: u32,
        mut build: F,
    ) -> ExchangeResult<Response>
    where
        F: FnMut() -> RequestBuilder,
    {
        self.execute_with_retry_inner(&mut build, Some(weight.max(1)))
            .await
    }

    async fn execute_with_retry_inner<F>(
        &self,
        build: &mut F,
        weight_override: Option<u32>,
    ) -> ExchangeResult<Response>
    where
        F: FnMut() -> RequestBuilder,
    {
        let mut last_err: Option<ExchangeError> = None;
        // gate 元数据（method/path/weight/request_key）只依赖请求本身，同一次
        // 调用的所有 attempt 完全相同：只在首个 attempt 计算一次并复用——
        // request_gate 内部含 try_clone+build、路径模板化、≤16KB body 的 JSON
        // 解析，此前按 attempt 重复执行纯属浪费。
        let mut gate_slot = None;

        for attempt in 0..self.max_retries {
            let req = build();
            let gate = &*gate_slot
                .get_or_insert_with(|| self.request_gate_with_weight(&req, weight_override));
            if let Err(error) = gate.gate.check_ready(common::time::now_ms()) {
                self.record_host_gate_reject_outcome(gate, &error);
                return Err(error);
            }
            let _singleflight = gate.gate.singleflight(gate.request_key.clone()).await;
            self.wait_rate_limit(gate.weight).await;
            record_http_request(&self.exchange, &gate.method, &gate.path, gate.weight);
            match self.send_attempt(req, attempt).await {
                AttemptOutcome::Success {
                    response,
                    transport_rtt_ms,
                } => {
                    self.record_http_success_outcome(
                        gate,
                        response.status().as_u16(),
                        transport_rtt_ms,
                    );
                    gate.gate.record_success();
                    return Ok(response);
                }
                AttemptOutcome::Retry {
                    error,
                    delay,
                    transport_rtt_ms,
                } => {
                    self.record_http_error_outcome(gate, &error, transport_rtt_ms, true);
                    gate.gate.record_error(&error, common::time::now_ms());
                    last_err = Some(error);
                    sleep(delay).await;
                }
                AttemptOutcome::Fail {
                    error,
                    transport_rtt_ms,
                } => {
                    self.record_http_error_outcome(gate, &error, transport_rtt_ms, false);
                    gate.gate.record_error(&error, common::time::now_ms());
                    return Err(error);
                }
            }
        }

        Err(last_err.unwrap_or(ExchangeError::Network("max retries exceeded".into())))
    }

    /// Executes exactly one transport attempt while preserving the shared rate limit,
    /// host-gate, singleflight, and telemetry contracts.
    ///
    /// Use this for non-idempotent writes whose result must be reconciled through a
    /// read-side identity query instead of replaying the write.
    pub async fn execute_once<F>(&self, mut build: F) -> ExchangeResult<Response>
    where
        F: FnMut() -> RequestBuilder,
    {
        let req = build();
        let gate = self.request_gate(&req);
        if let Err(error) = gate.gate.check_ready(common::time::now_ms()) {
            self.record_host_gate_reject_outcome(&gate, &error);
            return Err(error);
        }
        let _singleflight = gate.gate.singleflight(gate.request_key.clone()).await;
        self.wait_rate_limit(gate.weight).await;
        record_http_request(&self.exchange, &gate.method, &gate.path, gate.weight);
        match self.send_attempt(req, 0).await {
            AttemptOutcome::Success {
                response,
                transport_rtt_ms,
            } => {
                self.record_http_success_outcome(
                    &gate,
                    response.status().as_u16(),
                    transport_rtt_ms,
                );
                gate.gate.record_success();
                Ok(response)
            }
            AttemptOutcome::Retry {
                error,
                transport_rtt_ms,
                ..
            }
            | AttemptOutcome::Fail {
                error,
                transport_rtt_ms,
            } => {
                self.record_http_error_outcome(&gate, &error, transport_rtt_ms, false);
                gate.gate.record_error(&error, common::time::now_ms());
                Err(error)
            }
        }
    }

    /// Executes one non-idempotent attempt, creating timestamped authentication
    /// only after host-gate and rate-limit waits have completed.
    pub async fn execute_once_fresh<F>(
        &self,
        method: Method,
        endpoint: &str,
        mut build: F,
    ) -> ExchangeResult<Response>
    where
        F: FnMut() -> ExchangeResult<RequestBuilder>,
    {
        let preview = self.request(method, endpoint);
        let gate = self.request_gate(&preview);
        if let Err(error) = gate.gate.check_ready(common::time::now_ms()) {
            self.record_host_gate_reject_outcome(&gate, &error);
            return Err(error);
        }
        let _singleflight = gate.gate.singleflight(gate.request_key.clone()).await;
        self.wait_rate_limit(gate.weight).await;
        let request = build()?;
        record_http_request(&self.exchange, &gate.method, &gate.path, gate.weight);
        match self.send_attempt(request, 0).await {
            AttemptOutcome::Success {
                response,
                transport_rtt_ms,
            } => {
                self.record_http_success_outcome(
                    &gate,
                    response.status().as_u16(),
                    transport_rtt_ms,
                );
                gate.gate.record_success();
                Ok(response)
            }
            AttemptOutcome::Retry {
                error,
                transport_rtt_ms,
                ..
            }
            | AttemptOutcome::Fail {
                error,
                transport_rtt_ms,
            } => {
                self.record_http_error_outcome(&gate, &error, transport_rtt_ms, false);
                gate.gate.record_error(&error, common::time::now_ms());
                Err(error)
            }
        }
    }

    async fn wait_rate_limit(&self, weight: u32) {
        if let Some(rl) = &self.rate_limiter {
            rl.wait_weight(weight).await;
        }
    }

    fn request_gate(&self, req: &RequestBuilder) -> RequestGate {
        self.request_gate_with_weight(req, None)
    }

    fn request_gate_with_weight(
        &self,
        req: &RequestBuilder,
        weight_override: Option<u32>,
    ) -> RequestGate {
        let meta = request_meta(req, &self.exchange)
            .unwrap_or_else(|| RequestMeta::fallback(&self.exchange));
        RequestGate {
            gate: HostGate::shared(&self.exchange, &meta.host),
            request_key: meta.request_key,
            method: meta.method,
            path: meta.path,
            request_context: meta.request_context,
            weight: weight_override.unwrap_or(meta.weight),
        }
    }

    fn record_http_success_outcome(&self, gate: &RequestGate, status_code: u16, latency_ms: u64) {
        let request_id = common::request_id::current();
        record_http_outcome(HttpOutcomeSample {
            exchange: &self.exchange,
            method: &gate.method,
            path: &gate.path,
            outcome: "success",
            status_code: Some(status_code),
            latency_ms,
            retry_after_ms: None,
            request_id: request_id.as_deref(),
            request_context: &gate.request_context,
            retry: false,
            observed_at_ms: common::time::now_ms(),
        });
    }

    fn record_http_error_outcome(
        &self,
        gate: &RequestGate,
        error: &ExchangeError,
        latency_ms: u64,
        retry: bool,
    ) {
        let meta = error_metric_meta(error);
        let request_id = common::request_id::current();
        record_http_outcome(HttpOutcomeSample {
            exchange: &self.exchange,
            method: &gate.method,
            path: &gate.path,
            outcome: meta.outcome,
            status_code: meta.status_code,
            latency_ms,
            retry_after_ms: meta.retry_after_ms,
            request_id: request_id.as_deref(),
            request_context: &gate.request_context,
            retry,
            observed_at_ms: common::time::now_ms(),
        });
    }

    fn record_host_gate_reject_outcome(&self, gate: &RequestGate, error: &ExchangeError) {
        let meta = host_gate_error_metric_meta(error);
        let request_id = common::request_id::current();
        record_http_outcome(HttpOutcomeSample {
            exchange: &self.exchange,
            method: &gate.method,
            path: &gate.path,
            outcome: meta.outcome,
            status_code: meta.status_code,
            latency_ms: 0,
            retry_after_ms: meta.retry_after_ms,
            request_id: request_id.as_deref(),
            request_context: &gate.request_context,
            retry: false,
            observed_at_ms: common::time::now_ms(),
        });
    }

    async fn send_attempt(&self, req: RequestBuilder, attempt: u32) -> AttemptOutcome {
        let started = Instant::now();
        match req.send().await {
            Ok(resp) => {
                let transport_rtt_ms = elapsed_ms(started);
                self.handle_response(resp, attempt, transport_rtt_ms).await
            }
            Err(e) if e.is_timeout() => self.retry_timeout(attempt, elapsed_ms(started)),
            Err(e) if e.is_connect() => self.retry_connect_error(&e, attempt, elapsed_ms(started)),
            Err(e) => AttemptOutcome::Fail {
                error: ExchangeError::Network(e.to_string()),
                transport_rtt_ms: elapsed_ms(started),
            },
        }
    }

    async fn handle_response(
        &self,
        resp: Response,
        attempt: u32,
        transport_rtt_ms: u64,
    ) -> AttemptOutcome {
        let status = resp.status();
        if status.as_u16() == 429 {
            return self.retry_rate_limited(&resp, attempt, transport_rtt_ms);
        }
        if status.is_server_error() {
            return self
                .retry_server_error(resp, attempt, transport_rtt_ms)
                .await;
        }
        debug!(
            exchange = %self.exchange,
            attempt,
            status = status.as_u16(),
            "request done"
        );
        AttemptOutcome::Success {
            response: resp,
            transport_rtt_ms,
        }
    }

    fn retry_rate_limited(
        &self,
        resp: &Response,
        attempt: u32,
        transport_rtt_ms: u64,
    ) -> AttemptOutcome {
        let retry_after = parse_retry_after(resp)
            .unwrap_or_else(|| self.backoff_ms(attempt) / 1000)
            .max(1);
        warn!(
            exchange = %self.exchange,
            attempt,
            retry_after,
            "429 rate limited"
        );
        AttemptOutcome::Retry {
            error: ExchangeError::RateLimited {
                retry_after_secs: retry_after,
            },
            delay: Duration::from_secs(retry_after.max(1)),
            transport_rtt_ms,
        }
    }

    async fn retry_server_error(
        &self,
        resp: Response,
        attempt: u32,
        transport_rtt_ms: u64,
    ) -> AttemptOutcome {
        let status = resp.status();
        let body = read_error_body(resp).await;
        warn!(
            exchange = %self.exchange,
            attempt,
            status = status.as_u16(),
            "server error, will retry"
        );
        AttemptOutcome::Retry {
            error: ExchangeError::Http {
                status: status.as_u16(),
                body: truncate(&body, 200),
            },
            delay: Duration::from_millis(self.backoff_ms(attempt)),
            transport_rtt_ms,
        }
    }

    fn retry_timeout(&self, attempt: u32, transport_rtt_ms: u64) -> AttemptOutcome {
        warn!(exchange = %self.exchange, attempt, "request timeout");
        AttemptOutcome::Retry {
            error: ExchangeError::Timeout {
                seconds: self.timeout_secs,
            },
            delay: Duration::from_millis(self.backoff_ms(attempt)),
            transport_rtt_ms,
        }
    }

    fn retry_connect_error(
        &self,
        error: &reqwest::Error,
        attempt: u32,
        transport_rtt_ms: u64,
    ) -> AttemptOutcome {
        warn!(exchange = %self.exchange, attempt, error = %error, "connect error");
        AttemptOutcome::Retry {
            error: ExchangeError::Network(error.to_string()),
            delay: Duration::from_millis(self.backoff_ms(attempt)),
            transport_rtt_ms,
        }
    }

    fn backoff_ms(&self, attempt: u32) -> u64 {
        let exponent = attempt.min(MAX_BACKOFF_EXPONENT);
        self.base_backoff_ms
            .saturating_mul(2u64.saturating_pow(exponent))
    }
}

async fn read_error_body(resp: Response) -> String {
    match resp.text().await {
        Ok(body) => body,
        Err(error) => format!("<response body read failed: {error}>"),
    }
}

struct ErrorMetricMeta {
    outcome: &'static str,
    status_code: Option<u16>,
    retry_after_ms: Option<u64>,
}

fn error_metric_meta(error: &ExchangeError) -> ErrorMetricMeta {
    match error {
        ExchangeError::RateLimited { retry_after_secs } => ErrorMetricMeta {
            outcome: "rate_limited",
            status_code: Some(429),
            retry_after_ms: Some(retry_after_secs.saturating_mul(1_000)),
        },
        ExchangeError::Timeout { .. } => ErrorMetricMeta {
            outcome: "timeout",
            status_code: None,
            retry_after_ms: None,
        },
        ExchangeError::Network(_) => ErrorMetricMeta {
            outcome: "network",
            status_code: None,
            retry_after_ms: None,
        },
        ExchangeError::Http { status, .. } => ErrorMetricMeta {
            outcome: "http_error",
            status_code: Some(*status),
            retry_after_ms: None,
        },
        ExchangeError::CircuitBreaker { .. } => ErrorMetricMeta {
            outcome: "circuit_open",
            status_code: None,
            retry_after_ms: None,
        },
        _ => ErrorMetricMeta {
            outcome: "other_error",
            status_code: None,
            retry_after_ms: None,
        },
    }
}

fn host_gate_error_metric_meta(error: &ExchangeError) -> ErrorMetricMeta {
    match error {
        ExchangeError::RateLimited { retry_after_secs } => ErrorMetricMeta {
            outcome: "gate_rate_limited",
            status_code: None,
            retry_after_ms: Some(retry_after_secs.saturating_mul(1_000)),
        },
        ExchangeError::CircuitBreaker { .. } => ErrorMetricMeta {
            outcome: "gate_circuit_open",
            status_code: None,
            retry_after_ms: None,
        },
        _ => error_metric_meta(error),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    let elapsed = started.elapsed().as_millis();
    elapsed.min(u128::from(u64::MAX)) as u64
}

struct RequestMeta {
    host: String,
    request_key: String,
    method: String,
    path: String,
    request_context: Vec<String>,
    weight: u32,
}

impl RequestMeta {
    fn fallback(exchange: &str) -> Self {
        Self {
            host: exchange.to_owned(),
            request_key: "UNKNOWN:/".to_owned(),
            method: "UNKNOWN".to_owned(),
            path: "/".to_owned(),
            request_context: Vec::new(),
            weight: 1,
        }
    }
}

fn request_meta(req: &RequestBuilder, exchange: &str) -> Option<RequestMeta> {
    let request = req.try_clone()?.build().ok()?;
    let host = host_key(request.url())?;
    let method = request.method().as_str().to_owned();
    let raw_path = request.url().path();
    let path = path_template(raw_path);
    let request_key = request_key(&method, request.url());
    let request_context = request_context(
        request.url(),
        request.body().and_then(reqwest::Body::as_bytes),
    );
    let weight = spec_method(request.method())
        .and_then(|method| {
            endpoint_weight(exchange, method, raw_path)
                .or_else(|| endpoint_weight(exchange, method, &path))
        })
        .unwrap_or(1);
    Some(RequestMeta {
        host,
        request_key,
        method,
        path,
        request_context,
        weight,
    })
}

fn path_template(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut previous = None;
    let mut changed = false;
    for segment in path.trim_start_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        out.push('/');
        if is_dynamic_path_segment(previous, segment) {
            out.push_str(PATH_ID_TEMPLATE);
            changed = true;
        } else {
            out.push_str(segment);
        }
        previous = Some(segment);
    }
    if out.is_empty() {
        return "/".to_owned();
    }
    if changed {
        out
    } else {
        path.to_owned()
    }
}

fn is_dynamic_path_segment(previous: Option<&str>, segment: &str) -> bool {
    if segment.is_empty() || is_version_segment(segment) {
        return false;
    }
    is_numeric_id(segment)
        || is_long_token(segment)
        || is_uuid_like(segment)
        || previous.is_some_and(|parent| is_dynamic_child_segment(parent, segment))
}

fn is_dynamic_child_segment(parent: &str, segment: &str) -> bool {
    if is_static_subroute(segment) {
        return false;
    }
    match parent {
        "order" | "orders" | "client-order" | "client_order" => is_short_path_id(segment),
        "contract" | "contracts" | "instrument" | "instruments" | "symbol" | "symbols" => {
            is_symbol_path_segment(segment)
        }
        _ => false,
    }
}

fn is_static_subroute(segment: &str) -> bool {
    matches!(
        segment,
        "active" | "client-order" | "closed" | "fills" | "open" | "realtime"
    )
}

fn is_version_segment(segment: &str) -> bool {
    segment
        .strip_prefix('v')
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
}

fn is_numeric_id(segment: &str) -> bool {
    segment.len() >= 4 && segment.chars().all(|ch| ch.is_ascii_digit())
}

fn is_short_path_id(segment: &str) -> bool {
    segment.len() >= 3
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        && segment.chars().any(|ch| ch.is_ascii_digit())
}

fn is_long_token(segment: &str) -> bool {
    segment.len() >= 16
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        && segment.chars().any(|ch| ch.is_ascii_digit())
        && segment.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn is_uuid_like(segment: &str) -> bool {
    segment.len() >= 32
        && segment
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() || ch == '-')
        && segment.chars().filter(|ch| *ch == '-').count() >= 4
}

fn is_symbol_path_segment(segment: &str) -> bool {
    segment.len() >= 5
        && segment.chars().all(|ch| {
            ch.is_ascii_uppercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '%')
        })
        && segment.chars().any(|ch| ch.is_ascii_alphabetic())
        && has_quote_suffix(segment)
}

fn has_quote_suffix(segment: &str) -> bool {
    const QUOTES: [&str; 7] = ["USDTM", "USDT", "USDC", "USDH", "USDM", "USD", "PERP"];
    QUOTES.iter().any(|quote| segment.ends_with(quote))
}

fn request_context(url: &reqwest::Url, body: Option<&[u8]>) -> Vec<String> {
    let mut out = Vec::new();
    collect_query_context(url, &mut out);
    if let Some(body) = body.filter(|body| body.len() <= MAX_REQUEST_CONTEXT_BODY_BYTES) {
        collect_body_context(body, &mut out);
    }
    out
}

fn collect_query_context(url: &reqwest::Url, out: &mut Vec<String>) {
    for (key, value) in url.query_pairs() {
        push_context(out, key.as_ref(), value.as_ref());
        if out.len() >= MAX_REQUEST_CONTEXT_ITEMS {
            return;
        }
    }
}

fn collect_body_context(body: &[u8], out: &mut Vec<String>) {
    if out.len() >= MAX_REQUEST_CONTEXT_ITEMS {
        return;
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        collect_json_context(&value, out, 0);
        return;
    }
    if let Ok(text) = std::str::from_utf8(body) {
        collect_form_context(text, out);
    }
}

fn collect_json_context(value: &serde_json::Value, out: &mut Vec<String>, depth: u8) {
    if depth > 2 || out.len() >= MAX_REQUEST_CONTEXT_ITEMS {
        return;
    }
    match value {
        serde_json::Value::Object(map) => collect_json_object_context(map, out, depth),
        serde_json::Value::Array(values) => collect_json_array_context(values, out, depth),
        _ => {}
    }
}

fn collect_json_object_context(
    map: &serde_json::Map<String, serde_json::Value>,
    out: &mut Vec<String>,
    depth: u8,
) {
    for (key, value) in map {
        push_json_context_value(out, key, value);
        collect_json_context(value, out, depth + 1);
        if out.len() >= MAX_REQUEST_CONTEXT_ITEMS {
            return;
        }
    }
}

fn collect_json_array_context(values: &[serde_json::Value], out: &mut Vec<String>, depth: u8) {
    for value in values {
        collect_json_context(value, out, depth + 1);
        if out.len() >= MAX_REQUEST_CONTEXT_ITEMS {
            return;
        }
    }
}

fn collect_form_context(text: &str, out: &mut Vec<String>) {
    for (key, value) in url::form_urlencoded::parse(text.as_bytes()) {
        push_context(out, key.as_ref(), value.as_ref());
        if out.len() >= MAX_REQUEST_CONTEXT_ITEMS {
            return;
        }
    }
}

fn push_json_context_value(out: &mut Vec<String>, key: &str, value: &serde_json::Value) {
    if !is_request_context_key(key) {
        return;
    }
    match value {
        serde_json::Value::String(value) => push_context(out, key, value),
        serde_json::Value::Number(value) => push_context(out, key, &value.to_string()),
        serde_json::Value::Array(values) => {
            for value in values {
                push_json_context_value(out, key, value);
                if out.len() >= MAX_REQUEST_CONTEXT_ITEMS {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn push_context(out: &mut Vec<String>, key: &str, value: &str) {
    if out.len() >= MAX_REQUEST_CONTEXT_ITEMS || !is_request_context_key(key) {
        return;
    }
    let Some(value) = sanitized_context_value(value) else {
        return;
    };
    let item = format!("{}={}", key.trim(), value);
    if !out.iter().any(|existing| existing == &item) {
        out.push(item);
    }
}

fn is_request_context_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "symbol"
            | "symbols"
            | "instid"
            | "instids"
            | "instfamily"
            | "contract_code"
            | "contractcode"
            | "currency_pair"
            | "currencypair"
            | "coin"
    )
}

fn sanitized_context_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(
        value
            .chars()
            .filter(|ch| !ch.is_control())
            .take(MAX_REQUEST_CONTEXT_VALUE_CHARS)
            .collect(),
    )
}

fn host_key(url: &reqwest::Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

fn request_key(method: &str, url: &reqwest::Url) -> String {
    match url.query() {
        Some(query) => format!("{}:{}?{}", method, url.path(), query),
        None => format!("{}:{}", method, url.path()),
    }
}

fn spec_method(method: &Method) -> Option<SpecHttpMethod> {
    match *method {
        Method::GET => Some(SpecHttpMethod::Get),
        Method::POST => Some(SpecHttpMethod::Post),
        Method::DELETE => Some(SpecHttpMethod::Delete),
        _ => None,
    }
}

fn parse_retry_after(resp: &Response) -> Option<u64> {
    parse_retry_after_headers(resp.headers(), Utc::now())
}

fn parse_retry_after_headers(headers: &HeaderMap, now: DateTime<Utc>) -> Option<u64> {
    headers
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|value| parse_retry_after_value(value, now))
        .or_else(|| {
            headers
                .get(GATE_RATE_LIMIT_RESET_HEADER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| parse_gate_reset_timestamp(value, now))
        })
}

fn parse_retry_after_value(value: &str, now: DateTime<Utc>) -> Option<u64> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    let target = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    let wait_ms = target.signed_duration_since(now).num_milliseconds().max(0) as u64;
    Some(wait_ms.div_ceil(1000))
}

fn parse_gate_reset_timestamp(value: &str, now: DateTime<Utc>) -> Option<u64> {
    let timestamp = value.trim().parse::<i64>().ok()?;
    let target_ms = if timestamp >= UNIX_MILLISECONDS_THRESHOLD {
        timestamp
    } else if timestamp >= MIN_UNIX_SECONDS {
        timestamp.checked_mul(1_000)?
    } else {
        return None;
    };
    let wait_ms = target_ms.saturating_sub(now.timestamp_millis()).max(0) as u64;
    Some(wait_ms.div_ceil(1_000))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn backoff_grows_exponentially() {
        let client = HttpClient::builder("test")
            .max_retries(5)
            .base_backoff_ms(100)
            .build()
            .map_err(|error| error.to_string());
        let Ok(client) = client else {
            panic!("http client builds for backoff test");
        };
        assert_eq!(client.backoff_ms(0), 100);
        assert_eq!(client.backoff_ms(1), 200);
        assert_eq!(client.backoff_ms(2), 400);
        assert_eq!(client.backoff_ms(3), 800);
    }

    #[test]
    fn backoff_clamps_large_attempts() {
        let client = HttpClient::builder("test")
            .base_backoff_ms(500)
            .build()
            .map_err(|error| error.to_string());
        let Ok(client) = client else {
            panic!("http client builds for backoff clamp test");
        };
        assert_eq!(client.backoff_ms(10), 512_000);
        assert_eq!(client.backoff_ms(31), 512_000);
    }

    #[test]
    fn parse_retry_after_accepts_integer_seconds() {
        let now = DateTime::parse_from_rfc2822("Wed, 21 Oct 2015 07:28:00 GMT")
            .expect("valid test date")
            .with_timezone(&Utc);
        assert_eq!(parse_retry_after_value(" 12 ", now), Some(12));
    }

    #[test]
    fn parse_retry_after_accepts_http_date() {
        let now = DateTime::parse_from_rfc2822("Wed, 21 Oct 2015 07:28:00 GMT")
            .expect("valid test date")
            .with_timezone(&Utc);
        assert_eq!(
            parse_retry_after_value("Wed, 21 Oct 2015 07:28:03 GMT", now),
            Some(3)
        );
    }

    #[test]
    fn parse_retry_after_past_http_date_is_zero() {
        let now = DateTime::parse_from_rfc2822("Wed, 21 Oct 2015 07:28:03 GMT")
            .expect("valid test date")
            .with_timezone(&Utc);
        assert_eq!(
            parse_retry_after_value("Wed, 21 Oct 2015 07:28:00 GMT", now),
            Some(0)
        );
    }

    #[test]
    fn gate_reset_header_accepts_seconds_and_milliseconds() {
        let now = DateTime::parse_from_rfc2822("Wed, 21 Oct 2015 07:28:00 GMT")
            .expect("valid test date")
            .with_timezone(&Utc);

        assert_eq!(parse_gate_reset_timestamp("1445412483", now), Some(3));
        assert_eq!(parse_gate_reset_timestamp("1445412483000", now), Some(3));
    }

    #[test]
    fn standard_retry_after_takes_precedence_over_gate_reset_header() {
        let now = DateTime::parse_from_rfc2822("Wed, 21 Oct 2015 07:28:00 GMT")
            .expect("valid test date")
            .with_timezone(&Utc);
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "12".parse().expect("retry header"));
        headers.insert(
            GATE_RATE_LIMIT_RESET_HEADER,
            "1445412483000".parse().expect("gate reset header"),
        );

        assert_eq!(parse_retry_after_headers(&headers, now), Some(12));
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_long_string() {
        let s = "a".repeat(300);
        assert_eq!(truncate(&s, 100).len(), 100);
    }

    #[test]
    fn request_meta_uses_endpoint_spec_weight() {
        let raw = Client::new();
        let req = raw.get("https://api.binance.com/api/v3/ticker/24hr");
        let meta = request_meta(&req, "binance").expect("request metadata");

        assert_eq!(meta.request_key, "GET:/api/v3/ticker/24hr");
        assert_eq!(meta.weight, 80);
    }

    #[test]
    fn request_gate_accepts_official_query_dependent_weight() {
        let client = HttpClient::new("binance").expect("http client");
        let req = client
            .raw()
            .get("https://api.binance.com/api/v3/ticker/24hr?symbols=%5B%22SOLUSDC%22%5D");

        let gate = client.request_gate_with_weight(&req, Some(2));

        assert_eq!(gate.weight, 2);
    }

    #[test]
    fn request_meta_defaults_unknown_weight_to_one() {
        let raw = Client::new();
        let req = raw.get("https://example.test/unknown");
        let meta = request_meta(&req, "binance").expect("request metadata");

        assert_eq!(meta.weight, 1);
    }

    #[test]
    fn request_meta_keeps_port_and_query_in_gate_key() {
        let raw = Client::new();
        let req = raw.get("http://127.0.0.1:38888/v5/market/tickers?category=linear");
        let meta = request_meta(&req, "bybit").expect("request metadata");

        assert_eq!(meta.host, "127.0.0.1:38888");
        assert_eq!(meta.request_key, "GET:/v5/market/tickers?category=linear");
    }

    #[test]
    fn request_meta_templates_high_cardinality_path_only_for_metrics() {
        let raw = Client::new();
        let req =
            raw.get("https://api.gateio.ws/api/v4/futures/usdt/orders/1234567890?symbol=BTC_USDT");
        let meta = request_meta(&req, "gate").expect("request metadata");

        assert_eq!(meta.path, "/api/v4/futures/usdt/orders/{id}");
        assert_eq!(
            meta.request_key,
            "GET:/api/v4/futures/usdt/orders/1234567890?symbol=BTC_USDT"
        );
        assert_eq!(meta.request_context, vec!["symbol=BTC_USDT"]);
    }

    #[test]
    fn request_meta_keeps_short_dynamic_path_raw_in_request_key() {
        let raw = Client::new();
        let req =
            raw.get("https://api.kucoin.com/api/v1/orders/client-order/client-1?symbol=MUUSDTM");
        let meta = request_meta(&req, "kucoin").expect("request metadata");

        assert_eq!(meta.path, "/api/v1/orders/client-order/{id}");
        assert_eq!(
            meta.request_key,
            "GET:/api/v1/orders/client-order/client-1?symbol=MUUSDTM"
        );
        assert_eq!(meta.request_context, vec!["symbol=MUUSDTM"]);
    }

    #[test]
    fn path_template_keeps_static_order_endpoints_and_versions() {
        assert_eq!(path_template("/v5/order/realtime"), "/v5/order/realtime");
        assert_eq!(path_template("/api/v3/ticker/24hr"), "/api/v3/ticker/24hr");
        assert_eq!(
            path_template("/api/v1/contracts/active"),
            "/api/v1/contracts/active"
        );
    }

    #[test]
    fn path_template_redacts_long_client_order_tokens() {
        assert_eq!(
            path_template("/api/v1/orders/client-order/hedge-20260601-abcdef123456"),
            "/api/v1/orders/client-order/{id}"
        );
    }

    #[test]
    fn path_template_redacts_short_order_ids_and_contract_symbols() {
        assert_eq!(
            path_template("/api/v1/orders/t-cid-1"),
            "/api/v1/orders/{id}"
        );
        assert_eq!(
            path_template("/api/v1/orders/client-order/client-1"),
            "/api/v1/orders/client-order/{id}"
        );
        assert_eq!(
            path_template("/api/v1/contracts/BTC_USDT"),
            "/api/v1/contracts/{id}"
        );
        assert_eq!(
            path_template("/api/v1/contracts/XBTUSDTM"),
            "/api/v1/contracts/{id}"
        );
    }

    #[test]
    fn request_meta_extracts_bounded_request_context() {
        let raw = Client::new();
        let req = raw
            .post("https://api.hyperliquid.xyz/info?instId=MU-USDC&signature=secret")
            .body(
                r#"{"type":"l2Book","coin":"SNDK","orders":[{"symbol":"AAPL-USDC"}],"signature":"secret"}"#,
            );
        let meta = request_meta(&req, "hyperliquid").expect("request metadata");

        assert_eq!(
            meta.request_context,
            vec![
                "instId=MU-USDC".to_owned(),
                "coin=SNDK".to_owned(),
                "symbol=AAPL-USDC".to_owned()
            ]
        );
        assert!(!meta
            .request_context
            .iter()
            .any(|item| item.contains("secret")));
    }

    #[tokio::test]
    async fn success_outcome_records_scoped_request_id() {
        let client = HttpClient::builder("unit_http_request_id")
            .build()
            .map_err(|error| error.to_string());
        let Ok(client) = client else {
            panic!("http client builds for request id test");
        };
        let gate = RequestGate {
            gate: HostGate::shared("unit_http_request_id", "example.test"),
            request_key: "GET:/unit/request-id".to_owned(),
            method: "GET".to_owned(),
            path: "/unit/request-id".to_owned(),
            request_context: vec!["symbol=BTCUSDT".to_owned()],
            weight: 1,
        };

        common::request_id::scope("rid-http-1".to_owned(), async {
            client.record_http_success_outcome(&gate, 200, 7);
        })
        .await;

        let rows = crate::http_metrics::http_outcome_metrics_snapshot();
        let Some(row) = rows
            .iter()
            .find(|row| row.exchange == "unit_http_request_id" && row.path == "/unit/request-id")
        else {
            panic!("missing request id outcome row");
        };
        assert_eq!(row.last_request_id.as_deref(), Some("rid-http-1"));
        assert_eq!(row.last_request_context, vec!["symbol=BTCUSDT"]);
        assert_eq!(row.last_latency_ms, 7);
    }

    #[tokio::test]
    async fn execute_once_does_not_replay_server_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/non-idempotent"))
            .respond_with(wiremock::ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let client = HttpClient::builder("unit_http_execute_once")
            .max_retries(3)
            .base_backoff_ms(1)
            .build()
            .expect("http client");

        let error = client
            .execute_once(|| {
                client.request(Method::POST, format!("{}/non-idempotent", server.uri()))
            })
            .await
            .expect_err("server error must return after one write attempt");

        assert!(matches!(error, ExchangeError::Http { status: 503, .. }));
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("request history")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn fresh_request_build_waits_until_after_rate_limit_permit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/signed"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let limiter = Arc::new(RateLimiter::per_second("fresh-request-test", 1));
        assert!(limiter.try_acquire());
        let client = Arc::new(
            HttpClient::builder("fresh-request-test")
                .rate_limiter(Arc::clone(&limiter))
                .build()
                .expect("http client"),
        );
        let builds = Arc::new(AtomicUsize::new(0));
        let endpoint = format!("{}/signed", server.uri());
        let task = tokio::spawn({
            let client = Arc::clone(&client);
            let builds = Arc::clone(&builds);
            let endpoint = endpoint.clone();
            async move {
                client
                    .execute_with_retry_fresh(Method::GET, &endpoint, || {
                        builds.fetch_add(1, Ordering::Relaxed);
                        Ok(client.request(Method::GET, &endpoint))
                    })
                    .await
            }
        });

        sleep(Duration::from_millis(50)).await;
        assert_eq!(builds.load(Ordering::Relaxed), 0);
        task.await
            .expect("fresh request task")
            .expect("fresh request response");
        assert_eq!(builds.load(Ordering::Relaxed), 1);
    }
}
