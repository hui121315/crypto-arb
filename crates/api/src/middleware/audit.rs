//! 审计日志：JSONL 文件追加。
//!
//! 用途：敏感操作（凭证更新、kill-switch、live-mode 切换）的不可变审计记录。
//! 格式：每行一个 JSON 对象，含 `ts`、`actor`、`action`、`resource`、`outcome`、`detail`。
//!
//! 行为契约：
//! - 未配置 `audit_log_path` → 全部 noop（dev / 测试）。
//! - 配置文件路径 → 自动创建父目录，以 append 模式打开。
//! - 请求路径只做 JSON 序列化和有界入队，后台 writer 负责落盘。
//! - 入队/写入失败进入 health snapshot；实盘订单 mutation 会据此 fail-closed。
//! - POSIX 单次 write < `PIPE_BUF` (4096) 是原子的，每行一条 JSON 通常 < 1KB，多进程追加安全。

use std::sync::OnceLock;
use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue};

mod correlation;
mod event;
mod replay;
mod writer;

pub(crate) use correlation::AuditCorrelation;
pub(crate) use event::{AuditEvent, AuditEventContext, AuditResourceKind};
pub(crate) use replay::replay_action_runs;
use writer::AuditSink;

/// 全局审计日志单例。`init` 只能调用一次（main.rs 启动时）。
static AUDIT: OnceLock<AuditSink> = OnceLock::new();
pub(super) const AUDIT_WRITER_QUEUE_CAPACITY: usize = 1024;

/// 初始化审计日志单例。重复调用会被忽略（tracing warn）。
pub(crate) fn init(path: Option<&str>) {
    let sink = AuditSink::new(path);
    if AUDIT.set(sink).is_err() {
        tracing::warn!("audit logger already initialized; ignoring re-init");
    }
}

/// 写入一条审计事件。未初始化或未配置文件时为 noop。
pub(crate) fn record(event: &AuditEvent<'_>) {
    if let Some(sink) = AUDIT.get() {
        sink.write(event);
    }
}

/// 高风险 `ActionRun` 使用逐事件 durable ACK：writer 必须完成 `sync_data` 后才返回。
/// 未初始化或未配置 audit sink 的本地开发/测试模式维持 noop；已配置但不可写时返回错误，
/// 调用方必须在外部副作用前 fail-closed。
pub(crate) fn record_durable(event: &AuditEvent<'_>) -> Result<(), String> {
    AUDIT.get().map_or(Ok(()), |sink| sink.write_durable(event))
}

/// 关闭阶段停止接收新事件，并按 FIFO drain 之前已入队的写入。
pub(crate) fn shutdown(timeout: Duration) -> Result<(), String> {
    AUDIT.get().map_or(Ok(()), |sink| sink.shutdown(timeout))
}

pub(crate) fn record_http_event(
    headers: &HeaderMap,
    action: &'static str,
    resource: &str,
    outcome: &'static str,
    detail: serde_json::Value,
) {
    let actor = extract_actor(headers);
    record(&AuditEvent::now(
        actor.as_str(),
        action,
        resource,
        outcome,
        detail_with_request_id(detail),
    ));
}

pub(crate) fn health_snapshot(now_ms: i64) -> AuditSinkHealthSnapshot {
    AUDIT.get().map_or_else(
        || AuditSinkHealthSnapshot::not_initialized(now_ms),
        |sink| sink.health_snapshot(now_ms),
    )
}

#[cfg(test)]
pub(crate) fn flush_for_test() {
    if let Some(sink) = AUDIT.get() {
        sink.wait_until_idle_for_test();
    }
}

pub(crate) const VERIFIED_ACTOR_HEADER: &str = "x-crossline-verified-actor";

const API_TOKEN_ACTOR_PREFIX: &str = "api-token";
const API_TOKEN_ACTOR_MESSAGE: &[u8] = b"crossline-omni:audit-actor:v1";
const API_TOKEN_FINGERPRINT_HEX_LEN: usize = 16;
const UNKNOWN_ACTOR: &str = "unknown";

/// 从 HTTP headers 中提取调用方标识。
///
/// 浏览器和普通客户端都能伪造 `X-Forwarded-For` / `X-Real-IP`，所以这里不再把这些
/// header 当作 actor。auth middleware 会先清掉客户端伪造的内部 actor header，只在
/// Bearer token 校验成功后写入 `x-crossline-verified-actor`。
pub(crate) fn extract_actor(headers: &HeaderMap) -> String {
    verified_actor(headers).unwrap_or_else(|| UNKNOWN_ACTOR.to_owned())
}

pub(crate) fn clear_verified_actor(headers: &mut HeaderMap) {
    headers.remove(VERIFIED_ACTOR_HEADER);
}

pub(crate) fn insert_verified_bearer_actor(
    headers: &mut HeaderMap,
    token: &str,
    actor_label: Option<&str>,
) {
    let actor = verified_bearer_actor(token, actor_label);
    match HeaderValue::from_str(actor.as_str()) {
        Ok(value) => {
            headers.insert(VERIFIED_ACTOR_HEADER, value);
        }
        Err(error) => tracing::warn!(%error, "audit: verified actor header encode failed"),
    }
}

fn verified_actor(headers: &HeaderMap) -> Option<String> {
    headers
        .get(VERIFIED_ACTOR_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|actor| is_verified_actor(actor))
        .map(ToOwned::to_owned)
}

fn verified_bearer_actor(token: &str, actor_label: Option<&str>) -> String {
    let digest = common::signing::hmac_sha256_hex(token.as_bytes(), API_TOKEN_ACTOR_MESSAGE);
    let fingerprint = digest
        .get(..API_TOKEN_FINGERPRINT_HEX_LEN)
        .unwrap_or(digest.as_str());
    match actor_label.filter(|label| is_valid_actor_label(label)) {
        Some(label) => format!("{API_TOKEN_ACTOR_PREFIX}:{label}:{fingerprint}"),
        None => format!("{API_TOKEN_ACTOR_PREFIX}:{fingerprint}"),
    }
}

fn is_verified_actor(actor: &str) -> bool {
    let mut segments = actor.split(':');
    let prefix = segments.next();
    let first = segments.next();
    let second = segments.next();
    if prefix != Some(API_TOKEN_ACTOR_PREFIX) || segments.next().is_some() {
        return false;
    }
    match (first, second) {
        (Some(fingerprint), None) => is_actor_fingerprint(fingerprint),
        (Some(label), Some(fingerprint)) => {
            is_valid_actor_label(label) && is_actor_fingerprint(fingerprint)
        }
        _ => false,
    }
}

fn is_actor_fingerprint(value: &str) -> bool {
    value.len() == API_TOKEN_FINGERPRINT_HEX_LEN
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_valid_actor_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn detail_with_request_id(detail: serde_json::Value) -> serde_json::Value {
    let request_id = common::request_id::current();
    match detail {
        serde_json::Value::Object(mut object) => {
            object.insert("requestId".to_owned(), serde_json::json!(request_id));
            serde_json::Value::Object(object)
        }
        value => serde_json::json!({
            "requestId": request_id,
            "detail": value,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuditSinkHealthSnapshot {
    pub(crate) initialized: bool,
    pub(crate) configured: bool,
    pub(crate) opened: bool,
    pub(crate) path: Option<String>,
    pub(crate) queue_capacity: u64,
    pub(crate) pending_writes: u64,
    pub(crate) writer_alive: bool,
    pub(crate) write_attempts: u64,
    pub(crate) write_successes: u64,
    pub(crate) write_failures: u64,
    pub(crate) last_write_at_ms: Option<i64>,
    pub(crate) last_error_at_ms: Option<i64>,
    pub(crate) last_error: Option<String>,
    pub(crate) observed_at_ms: i64,
}

impl AuditSinkHealthSnapshot {
    fn not_initialized(now_ms: i64) -> Self {
        Self {
            initialized: false,
            configured: false,
            opened: false,
            path: None,
            queue_capacity: 0,
            pending_writes: 0,
            writer_alive: false,
            write_attempts: 0,
            write_successes: 0,
            write_failures: 0,
            last_write_at_ms: None,
            last_error_at_ms: None,
            last_error: Some("audit logger not initialized".to_owned()),
            observed_at_ms: now_ms,
        }
    }
}

/// 判断在 `Live` 订单 mutation 前是否应因审计链路不可用而 fail-closed 拒绝。
///
/// 契约：审计未配置（dev/test noop）→ 不拦截；一旦配置了审计落盘，但 sink 打不开或
/// 最近一次写入失败，则实盘下单/撤单不得继续——实盘订单 mutation 必须有可写的不可变审计记录，
/// 否则"被拒绝/已成交"会缺失合规留痕。
pub(crate) fn live_order_mutation_block_reason(
    snapshot: &AuditSinkHealthSnapshot,
) -> Option<String> {
    if !snapshot.configured {
        return None;
    }
    if !snapshot.opened {
        return Some(format!(
            "audit log configured but sink is not open: {}",
            snapshot.last_error.as_deref().unwrap_or("sink unavailable")
        ));
    }
    if let Some(error) = snapshot.last_error.as_deref() {
        return Some(format!("audit log last write failed: {error}"));
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn writes_jsonl_lines_to_file() {
        let dir = must_tempdir();
        let path = dir.path().join("audit.jsonl");
        let sink = AuditSink::new(path.to_str());

        let event = AuditEvent::now(
            "test-actor",
            "venue_credentials.update",
            "binance",
            "success",
            json!({"fields": ["api_key"]}),
        );
        sink.write(&event);
        sink.wait_until_idle_for_test();

        let contents = must_read_to_string(&path);
        let line = first_line(&contents);
        let parsed = must_parse_json(line);
        assert_eq!(parsed["actor"], "test-actor");
        assert_eq!(parsed["action"], "venue_credentials.update");
        assert_eq!(parsed["resource"], "binance");
        assert_eq!(parsed["outcome"], "success");
    }

    #[test]
    fn event_serializes_correlation_as_top_level_fields() {
        let event = AuditEvent::now(
            "actor",
            "trading.order.submit",
            "client-order-1",
            "success",
            json!({"safe": true}),
        )
        .with_correlation(
            AuditCorrelation::request(Some("req-1".to_owned()))
                .with_action_run_id("action-1".to_owned())
                .with_idempotency_key(Some("idem-1".to_owned()))
                .with_order_ids(["internal-1".to_owned(), "exchange-1".to_owned()])
                .with_run_ids(["execution-1".to_owned()]),
        );

        let value = serde_json::to_value(event)
            .unwrap_or_else(|error| panic!("audit event encode failed: {error}"));

        assert_eq!(value["requestId"], "req-1");
        assert_eq!(value["actionRunId"], "action-1");
        assert_eq!(value["idempotencyKey"], "idem-1");
        assert_eq!(value["orderIds"][1], "exchange-1");
        assert_eq!(value["runIds"][0], "execution-1");
        assert!(value["detail"]["safe"].as_bool().unwrap_or(false));
    }

    #[test]
    fn noop_when_path_missing() {
        let sink = AuditSink::new(None);
        let event = AuditEvent::now("a", "b", "c", "success", json!({}));
        sink.write(&event); // 不应 panic

        let health = sink.health_snapshot(10);
        assert!(health.initialized);
        assert!(!health.configured);
        assert!(!health.opened);
        assert_eq!(health.write_attempts, 0);
    }

    #[test]
    fn appends_multiple_lines() {
        let dir = must_tempdir();
        let path = dir.path().join("nested").join("audit.jsonl");
        let sink = AuditSink::new(path.to_str());
        for i in 0..3 {
            let event = AuditEvent::now("actor", "action", "res", "success", json!({"i": i}));
            sink.write(&event);
        }
        sink.wait_until_idle_for_test();
        let contents = must_read_to_string(&path);
        assert_eq!(contents.lines().count(), 3);

        let health = sink.health_snapshot(10);
        assert!(health.configured);
        assert!(health.opened);
        assert_eq!(health.write_attempts, 3);
        assert_eq!(health.write_successes, 3);
        assert_eq!(health.write_failures, 0);
        assert!(health.last_write_at_ms.is_some());
        assert!(health.last_error.is_none());
    }

    #[test]
    fn configured_sink_records_async_writer_health() {
        let dir = must_tempdir();
        let path = dir.path().join("async-audit.jsonl");
        let sink = AuditSink::new(path.to_str());
        let event = AuditEvent::now("actor", "action", "res", "accepted", json!({}));

        sink.write(&event);
        let queued = sink.health_snapshot(10);
        assert_eq!(queued.write_attempts, 1);
        assert_eq!(queued.queue_capacity, AUDIT_WRITER_QUEUE_CAPACITY as u64);
        assert!(queued.writer_alive);

        sink.wait_until_idle_for_test();
        let flushed = sink.health_snapshot(20);
        assert_eq!(flushed.pending_writes, 0);
        assert_eq!(flushed.write_successes, 1);
        assert_eq!(flushed.write_failures, 0);
        assert!(flushed.last_error.is_none());
        assert!(must_read_to_string(&path).contains("\"outcome\":\"accepted\""));
    }

    #[test]
    fn durable_ack_syncs_each_action_event_and_shutdown_drains_before_exit() {
        let dir = must_tempdir();
        let path = dir.path().join("durable-audit.jsonl");
        let sink = AuditSink::new(path.to_str());
        let event = AuditEvent::now(
            "actor",
            "trading.order.submit",
            "client-1",
            "accepted",
            json!({}),
        );

        sink.write_durable(&event)
            .unwrap_or_else(|error| panic!("durable audit write failed: {error}"));
        let flushed = sink.health_snapshot(10);
        assert_eq!(flushed.pending_writes, 0);
        assert_eq!(flushed.write_successes, 1);
        assert!(must_read_to_string(&path).contains("trading.order.submit"));

        sink.shutdown(Duration::from_secs(1))
            .unwrap_or_else(|error| panic!("audit shutdown failed: {error}"));
        let stopped = sink.health_snapshot(20);
        assert!(!stopped.writer_alive);
        assert_eq!(stopped.queue_capacity, 0);
        assert!(sink.write_durable(&event).is_err());
    }

    #[test]
    fn audit_sink_health_reports_open_failure() {
        let dir = must_tempdir();
        let sink = AuditSink::new(dir.path().to_str());

        let health = sink.health_snapshot(10);

        assert!(health.configured);
        assert!(!health.opened);
        assert_eq!(health.write_successes, 0);
        assert!(health
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("open_failed")));
    }

    #[test]
    fn audit_sink_health_reports_uninitialized_global() {
        let health = AuditSinkHealthSnapshot::not_initialized(10);

        assert!(!health.initialized);
        assert!(!health.configured);
        assert!(!health.opened);
        assert_eq!(health.observed_at_ms, 10);
        assert!(health.last_error.is_some());
    }

    fn block_reason_snapshot(
        configured: bool,
        opened: bool,
        last_error: Option<&str>,
    ) -> AuditSinkHealthSnapshot {
        AuditSinkHealthSnapshot {
            initialized: true,
            configured,
            opened,
            path: configured.then(|| "/tmp/audit.jsonl".to_owned()),
            queue_capacity: if configured && opened {
                AUDIT_WRITER_QUEUE_CAPACITY as u64
            } else {
                0
            },
            pending_writes: 0,
            writer_alive: configured && opened,
            write_attempts: 0,
            write_successes: 0,
            write_failures: 0,
            last_write_at_ms: None,
            last_error_at_ms: last_error.map(|_| 1),
            last_error: last_error.map(ToOwned::to_owned),
            observed_at_ms: 10,
        }
    }

    #[test]
    fn live_order_mutation_allowed_when_audit_unconfigured() {
        let snapshot = block_reason_snapshot(false, false, Some("audit logger not initialized"));
        assert_eq!(live_order_mutation_block_reason(&snapshot), None);
    }

    #[test]
    fn live_order_mutation_blocked_when_configured_sink_not_open() {
        let snapshot = block_reason_snapshot(true, false, Some("open_failed: denied"));
        let Some(reason) = live_order_mutation_block_reason(&snapshot) else {
            panic!("expected block reason");
        };
        assert!(reason.contains("not open"), "reason: {reason}");
        assert!(reason.contains("open_failed"), "reason: {reason}");
    }

    #[test]
    fn live_order_mutation_blocked_when_last_write_failed() {
        let snapshot = block_reason_snapshot(true, true, Some("write_failed: disk full"));
        let Some(reason) = live_order_mutation_block_reason(&snapshot) else {
            panic!("expected block reason");
        };
        assert!(reason.contains("last write failed"), "reason: {reason}");
        assert!(reason.contains("disk full"), "reason: {reason}");
    }

    #[test]
    fn live_order_mutation_allowed_when_audit_healthy() {
        let snapshot = block_reason_snapshot(true, true, None);
        assert_eq!(live_order_mutation_block_reason(&snapshot), None);
    }

    #[test]
    fn extract_actor_ignores_untrusted_forwarded_headers() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.7, 10.0.0.1"),
        );
        headers.insert("x-real-ip", HeaderValue::from_static("10.0.0.2"));
        assert_eq!(extract_actor(&headers), "unknown");
    }

    #[test]
    fn extract_actor_uses_verified_bearer_token_fingerprint() {
        let mut headers = axum::http::HeaderMap::new();
        insert_verified_bearer_actor(&mut headers, "configured-token", None);
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
        let actor = extract_actor(&headers);
        assert_eq!(actor, "api-token:4c9de24ee7366007");
        assert!(!actor.contains("configured-token"));
    }

    #[test]
    fn verified_actor_distinguishes_bearer_tokens_without_exposing_secret() {
        let mut first = axum::http::HeaderMap::new();
        let mut second = axum::http::HeaderMap::new();
        insert_verified_bearer_actor(&mut first, "configured-token", None);
        insert_verified_bearer_actor(&mut second, "rotated-token", None);

        let first_actor = extract_actor(&first);
        let second_actor = extract_actor(&second);

        assert_ne!(first_actor, second_actor);
        assert!(!first_actor.contains("configured-token"));
        assert!(!second_actor.contains("rotated-token"));
    }

    #[test]
    fn verified_actor_keeps_configured_operator_label_and_secret_fingerprint_separate() {
        let mut headers = axum::http::HeaderMap::new();
        insert_verified_bearer_actor(&mut headers, "configured-token", Some("operator-primary"));

        let actor = extract_actor(&headers);

        assert_eq!(actor, "api-token:operator-primary:4c9de24ee7366007");
        assert!(!actor.contains("configured-token"));
    }

    #[test]
    fn extract_actor_returns_unknown_when_absent() {
        let headers = axum::http::HeaderMap::new();
        assert_eq!(extract_actor(&headers), "unknown");
    }

    #[test]
    fn extract_actor_rejects_unverified_authorization_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer configured-token"),
        );
        assert_eq!(extract_actor(&headers), "unknown");
    }

    #[test]
    fn clear_verified_actor_removes_client_supplied_internal_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            VERIFIED_ACTOR_HEADER,
            HeaderValue::from_static("api-token:1234567890abcdef"),
        );
        assert_eq!(extract_actor(&headers), "api-token:1234567890abcdef");

        clear_verified_actor(&mut headers);

        assert_eq!(extract_actor(&headers), "unknown");
    }

    fn must_tempdir() -> tempfile::TempDir {
        match tempdir() {
            Ok(dir) => dir,
            Err(error) => panic!("tempdir failed: {error}"),
        }
    }

    fn must_read_to_string(path: &std::path::Path) -> String {
        match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) => panic!("read failed: {error}"),
        }
    }

    fn first_line(contents: &str) -> &str {
        let Some(line) = contents.lines().next() else {
            panic!("missing first line");
        };
        line
    }

    fn must_parse_json(line: &str) -> serde_json::Value {
        match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => panic!("json parse failed: {error}"),
        }
    }
}
