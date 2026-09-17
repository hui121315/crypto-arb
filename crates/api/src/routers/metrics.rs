//! `GET /metrics`：Prometheus 文本格式（v0.0.4）输出。
//!
//! 不依赖 `prometheus` crate，手工拼接最小指标集，便于在 Grafana / Prometheus 中抓取
//! 后端运行健康度。约定的指标命名前缀：`crypto_arb_`。

use crate::services::market_data::{MarketCacheAccessMetric, MarketDataStats, MarketRuntimeHealth};
use crate::services::runtime_state::{self, RuntimeStateInventory};
use crate::state::AppState;
use crate::task_registry::TaskSnapshot;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use exchange::{
    HostGateSnapshot, HttpOutcomeMetricSnapshot, HttpRequestMetricSnapshot, RateLimiterSnapshot,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/metrics", get(metrics))
}

async fn metrics(State(state): State<AppState>) -> Response {
    let metrics = state.metrics();
    let snap = metrics.snapshot();
    let funding_by_exchange = metrics.funding_exchange_counts();
    let market_data = state.market_data().stats_snapshot();
    let market_cache_access = state.market_data().cache_access_metrics_snapshot();
    let market_health = state.market_data().runtime_health_snapshot();
    let http_requests = exchange::http_request_metrics_snapshot();
    let http_outcomes = exchange::http_outcome_metrics_snapshot();
    let now_ms = common::time::now_ms();
    let host_gates = exchange::host_gate_snapshots(now_ms);
    let rate_limiters = exchange::rate_limiter_snapshots(now_ms);
    let registry = state.task_registry();
    let task_snapshots = registry.task_snapshots(now_ms);
    let runtime_state = runtime_state::inventory(&state).await;
    let body = render_metrics_body(&MetricsRenderInput {
        snap,
        market_data,
        market_cache_access: &market_cache_access,
        market_health: &market_health,
        hub: state.ws_hub(),
        funding_by_exchange: &funding_by_exchange,
        http_requests: &http_requests,
        http_outcomes: &http_outcomes,
        host_gates: &host_gates,
        rate_limiters: &rate_limiters,
        runtime_state: &runtime_state,
        now_ms,
        task_snapshots: &task_snapshots,
    });

    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

struct MetricsRenderInput<'a> {
    snap: crate::metrics::MetricsSnapshot,
    market_data: MarketDataStats,
    market_cache_access: &'a [MarketCacheAccessMetric],
    market_health: &'a [MarketRuntimeHealth],
    hub: &'a realtime::WsHub,
    funding_by_exchange: &'a BTreeMap<String, u64>,
    http_requests: &'a [HttpRequestMetricSnapshot],
    http_outcomes: &'a [HttpOutcomeMetricSnapshot],
    host_gates: &'a [HostGateSnapshot],
    rate_limiters: &'a [RateLimiterSnapshot],
    runtime_state: &'a RuntimeStateInventory,
    now_ms: i64,
    task_snapshots: &'a [TaskSnapshot],
}

fn render_metrics_body(input: &MetricsRenderInput<'_>) -> String {
    let channels = input.hub.channels();
    let mut body = String::with_capacity(2048);
    write_scan_metrics(&mut body, input.snap);
    write_funding_metrics(&mut body, input.snap, input.funding_by_exchange);
    write_market_cache_metrics(&mut body, input.market_data);
    write_market_cache_access_metrics(&mut body, input.market_cache_access);
    write_market_health_metrics(&mut body, input.market_health);
    write_http_request_metrics(&mut body, input.http_requests);
    write_http_outcome_metrics(&mut body, input.http_outcomes);
    write_host_gate_metrics(&mut body, input.host_gates);
    write_rate_limiter_metrics(&mut body, input.rate_limiters);
    write_ws_metrics(&mut body, input.snap, input.hub, &channels);
    write_opportunity_rest_metrics(&mut body, input.snap);
    write_runtime_state_metrics(&mut body, input.runtime_state);
    write_task_health_metrics(&mut body, input.now_ms, input.task_snapshots);
    body
}

fn write_scan_metrics(body: &mut String, snap: crate::metrics::MetricsSnapshot) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_scan_total Total number of arbitrage snapshot scans completed since process start."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_scan_total counter");
    let _ = writeln!(body, "crypto_arb_scan_total {}", snap.arb_scan_total);

    let _ = writeln!(
        body,
        "# HELP crypto_arb_last_scan_ms Wall-clock duration of the most recent arbitrage scan, in milliseconds."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_last_scan_ms gauge");
    let _ = writeln!(body, "crypto_arb_last_scan_ms {}", snap.arb_last_scan_ms);

    let _ = writeln!(
        body,
        "# HELP crypto_arb_last_count Number of arbitrage opportunities in the most recent snapshot."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_last_count gauge");
    let _ = writeln!(body, "crypto_arb_last_count {}", snap.arb_last_count);

    let _ = writeln!(
        body,
        "# HELP crypto_arb_last_at_ms Unix epoch milliseconds when the most recent arbitrage snapshot was produced."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_last_at_ms gauge");
    let _ = writeln!(body, "crypto_arb_last_at_ms {}", snap.arb_last_at_ms);
}

fn write_funding_metrics(
    body: &mut String,
    snap: crate::metrics::MetricsSnapshot,
    funding_by_exchange: &BTreeMap<String, u64>,
) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_funding_fetch_total Total number of funding-rates aggregations completed since process start."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_funding_fetch_total counter");
    let _ = writeln!(
        body,
        "crypto_arb_funding_fetch_total {}",
        snap.funding_fetch_total
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_funding_last_scan_ms Wall-clock duration of the most recent funding-rates fetch, in milliseconds."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_funding_last_scan_ms gauge");
    let _ = writeln!(
        body,
        "crypto_arb_funding_last_scan_ms {}",
        snap.funding_last_scan_ms
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_funding_last_count Total number of (symbol, exchange) funding-rate cells in the most recent snapshot."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_funding_last_count gauge");
    let _ = writeln!(
        body,
        "crypto_arb_funding_last_count {}",
        snap.funding_last_count
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_funding_last_at_ms Unix epoch milliseconds when the most recent funding-rates snapshot was produced."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_funding_last_at_ms gauge");
    let _ = writeln!(
        body,
        "crypto_arb_funding_last_at_ms {}",
        snap.funding_last_at_ms
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_funding_exchange_last_count Number of funding-rate cells in the most recent snapshot per exchange."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_funding_exchange_last_count gauge");
    for (exchange, count) in funding_by_exchange {
        let label = prometheus_label_value(exchange);
        let _ = writeln!(
            body,
            "crypto_arb_funding_exchange_last_count{{exchange=\"{label}\"}} {count}"
        );
    }

    let _ = writeln!(
        body,
        "# HELP crypto_arb_alerts_fired_total Total number of alert events fired since process start."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_alerts_fired_total counter");
    let _ = writeln!(
        body,
        "crypto_arb_alerts_fired_total {}",
        snap.alerts_fired_total
    );
}

fn write_market_cache_metrics(body: &mut String, market_data: MarketDataStats) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_cache_hit_total Total market-data cache fresh hits."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_market_cache_hit_total counter");
    let _ = writeln!(
        body,
        "crypto_arb_market_cache_hit_total {}",
        market_data.cache_hit_total
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_cache_miss_total Total market-data cache misses that required upstream refresh."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_market_cache_miss_total counter");
    let _ = writeln!(
        body,
        "crypto_arb_market_cache_miss_total {}",
        market_data.cache_miss_total
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_cache_stale_total Total market-data bounded-stale reads."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_market_cache_stale_total counter");
    let _ = writeln!(
        body,
        "crypto_arb_market_cache_stale_total {}",
        market_data.cache_stale_total
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_cache_hit_ratio Fresh hit ratio for market-data cache reads."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_market_cache_hit_ratio gauge");
    let _ = writeln!(
        body,
        "crypto_arb_market_cache_hit_ratio {:.6}",
        market_data.cache_hit_ratio
    );

    write_market_snapshot_stale_metrics(body, market_data);
    write_market_rest_baseline_metrics(body, market_data);
}

fn write_market_cache_access_metrics(body: &mut String, rows: &[MarketCacheAccessMetric]) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_cache_access_total Total market-data cache accesses by feed, outcome, source, and quality."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_market_cache_access_total counter");
    for row in rows {
        let source = row.key.source.as_str();
        let quality = row.key.quality.as_str();
        let _ = writeln!(
            body,
            "crypto_arb_market_cache_access_total{{feed=\"{}\",outcome=\"{}\",source=\"{source}\",quality=\"{quality}\"}} {}",
            row.key.feed, row.key.outcome, row.count
        );
    }
}

fn write_market_snapshot_stale_metrics(body: &mut String, market_data: MarketDataStats) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_snapshot_served_stale_total Total market snapshots served from bounded-stale rows while a refresh was already running."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_snapshot_served_stale_total counter"
    );
    write_market_feed_metric(
        body,
        "crypto_arb_market_snapshot_served_stale_total",
        "perp_tickers",
        market_data.perp_ticker_snapshot_served_stale_total,
    );
    write_market_feed_metric(
        body,
        "crypto_arb_market_snapshot_served_stale_total",
        "spot_ticks",
        market_data.spot_tick_snapshot_served_stale_total,
    );
}

fn write_market_rest_baseline_metrics(body: &mut String, market_data: MarketDataStats) {
    write_market_rest_baseline_guard_metrics(body, market_data);
    write_market_rest_baseline_wait_metrics(body, market_data);
    write_market_rest_baseline_lifecycle_metrics(body, market_data);
}

fn write_market_rest_baseline_guard_metrics(body: &mut String, market_data: MarketDataStats) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_rest_baseline_guard_keys Current REST baseline singleflight guard key count by scope."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_rest_baseline_guard_keys gauge"
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_guard_keys",
        "orderbook",
        market_data.rest_baseline_orderbook_guard_keys,
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_guard_keys",
        "snapshot",
        market_data.rest_baseline_snapshot_feed_keys,
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_rest_baseline_in_flight Current REST baseline singleflight guards held by scope."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_rest_baseline_in_flight gauge"
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_in_flight",
        "orderbook",
        market_data.rest_baseline_orderbook_in_flight,
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_in_flight",
        "snapshot",
        market_data.rest_baseline_snapshot_feed_in_flight,
    );
}

fn write_market_rest_baseline_wait_metrics(body: &mut String, market_data: MarketDataStats) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_rest_baseline_wait_count_total Total REST baseline singleflight waits by scope."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_rest_baseline_wait_count_total counter"
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_wait_count_total",
        "orderbook",
        market_data.rest_baseline_orderbook_wait_count_total,
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_wait_count_total",
        "snapshot",
        market_data.rest_baseline_snapshot_wait_count_total,
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_rest_baseline_wait_ms_total Total REST baseline singleflight wait time in milliseconds by scope."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_rest_baseline_wait_ms_total counter"
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_wait_ms_total",
        "orderbook",
        market_data.rest_baseline_orderbook_wait_ms_total,
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_wait_ms_total",
        "snapshot",
        market_data.rest_baseline_snapshot_wait_ms_total,
    );
}

fn write_market_rest_baseline_lifecycle_metrics(body: &mut String, market_data: MarketDataStats) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_rest_baseline_guard_evicted_total Total REST baseline singleflight guard keys evicted by scope."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_rest_baseline_guard_evicted_total counter"
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_guard_evicted_total",
        "orderbook",
        market_data.rest_baseline_orderbook_guard_evicted_total,
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_rest_baseline_guard_oldest_idle_ms Oldest current REST baseline singleflight guard idle time in milliseconds by scope."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_rest_baseline_guard_oldest_idle_ms gauge"
    );
    write_market_rest_baseline_scope_metric(
        body,
        "crypto_arb_market_rest_baseline_guard_oldest_idle_ms",
        "orderbook",
        market_data.rest_baseline_orderbook_guard_oldest_idle_ms,
    );
}

fn write_market_rest_baseline_scope_metric(
    body: &mut String,
    metric: &str,
    scope: &str,
    value: impl std::fmt::Display,
) {
    let _ = writeln!(body, "{metric}{{scope=\"{scope}\"}} {value}");
}

fn write_market_feed_metric(
    body: &mut String,
    metric: &str,
    feed: &str,
    value: impl std::fmt::Display,
) {
    let _ = writeln!(body, "{metric}{{feed=\"{feed}\"}} {value}");
}

fn write_market_health_metrics(body: &mut String, rows: &[MarketRuntimeHealth]) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_data_runtime_status Latest market-data runtime health by venue, operation, source, and quality (value is always 1)."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_market_data_runtime_status gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_data_runtime_requested Latest requested market-data items by venue and operation."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_data_runtime_requested gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_data_runtime_rows Latest returned market-data rows by venue and operation."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_market_data_runtime_rows gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_market_data_runtime_retry_after_ms Latest market-data retry-after delay by venue and operation."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_market_data_runtime_retry_after_ms gauge"
    );
    for row in rows {
        write_market_health_metric_row(body, row);
    }
}

fn write_market_health_metric_row(body: &mut String, row: &MarketRuntimeHealth) {
    let labels = market_health_labels(row);
    let _ = writeln!(body, "crypto_arb_market_data_runtime_status{{{labels}}} 1");
    let _ = writeln!(
        body,
        "crypto_arb_market_data_runtime_requested{{{labels}}} {}",
        row.requested
    );
    let _ = writeln!(
        body,
        "crypto_arb_market_data_runtime_rows{{{labels}}} {}",
        row.rows
    );
    if let Some(retry_after_ms) = row.retry_after_ms {
        let _ = writeln!(
            body,
            "crypto_arb_market_data_runtime_retry_after_ms{{{labels}}} {retry_after_ms}"
        );
    }
}

fn market_health_labels(row: &MarketRuntimeHealth) -> String {
    let venue = prometheus_label_value(&row.venue);
    let operation = prometheus_label_value(row.operation);
    let source = row.source.as_str();
    let quality = row.quality.as_str();
    format!("venue=\"{venue}\",operation=\"{operation}\",source=\"{source}\",quality=\"{quality}\"")
}

fn write_http_request_metrics(body: &mut String, rows: &[HttpRequestMetricSnapshot]) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_requests_total Total outbound exchange REST attempts by endpoint."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_http_requests_total counter");
    for row in rows {
        write_http_metric_row(
            body,
            "crypto_arb_http_requests_total",
            row,
            row.request_total,
        );
    }

    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_weight_total Total outbound exchange REST request weight by endpoint."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_http_request_weight_total counter");
    for row in rows {
        write_http_metric_row(
            body,
            "crypto_arb_http_request_weight_total",
            row,
            row.weight_total,
        );
    }
}

fn write_http_outcome_metrics(body: &mut String, rows: &[HttpOutcomeMetricSnapshot]) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_outcome_total Total outbound exchange REST outcomes by endpoint, outcome, and status."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_http_request_outcome_total counter");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_latency_ms_total Total outbound exchange REST attempt latency in milliseconds by outcome."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_request_latency_ms_total counter"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_retry_after_ms_total Total exchange REST retry-after milliseconds by outcome."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_request_retry_after_ms_total counter"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_last_latency_ms Latest outbound exchange REST attempt latency in milliseconds by outcome."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_http_request_last_latency_ms gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_last_retry_after_ms Latest exchange REST retry-after delay in milliseconds by outcome."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_request_last_retry_after_ms gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_last_observed_at_ms Unix epoch milliseconds when the latest exchange REST outcome was observed."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_request_last_observed_at_ms gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_retry_total Total outbound exchange REST retries by endpoint, outcome, and status."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_http_request_retry_total counter");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_latency_ms_bucket Cumulative outbound exchange REST latency buckets in milliseconds by outcome."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_request_latency_ms_bucket counter"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_request_latency_ms_p95_bucket Upper-bound bucket for approximate outbound exchange REST p95 latency."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_request_latency_ms_p95_bucket gauge"
    );
    for row in rows {
        write_http_outcome_metric_rows(body, row);
    }
}

fn write_http_outcome_metric_rows(body: &mut String, row: &HttpOutcomeMetricSnapshot) {
    let labels = http_outcome_labels(row);
    let _ = writeln!(
        body,
        "crypto_arb_http_request_outcome_total{{{labels}}} {}",
        row.request_total
    );
    let _ = writeln!(
        body,
        "crypto_arb_http_request_latency_ms_total{{{labels}}} {}",
        row.latency_ms_total
    );
    let _ = writeln!(
        body,
        "crypto_arb_http_request_last_latency_ms{{{labels}}} {}",
        row.last_latency_ms
    );
    if row.retry_after_ms_total > 0 {
        let _ = writeln!(
            body,
            "crypto_arb_http_request_retry_after_ms_total{{{labels}}} {}",
            row.retry_after_ms_total
        );
    }
    if let Some(last_retry_after_ms) = row.last_retry_after_ms {
        let _ = writeln!(
            body,
            "crypto_arb_http_request_last_retry_after_ms{{{labels}}} {last_retry_after_ms}"
        );
    }
    let _ = writeln!(
        body,
        "crypto_arb_http_request_last_observed_at_ms{{{labels}}} {}",
        row.last_observed_at_ms
    );
    if row.retry_total > 0 {
        let _ = writeln!(
            body,
            "crypto_arb_http_request_retry_total{{{labels}}} {}",
            row.retry_total
        );
    }
    for bucket in &row.latency_buckets {
        let _ = writeln!(
            body,
            "crypto_arb_http_request_latency_ms_bucket{{{labels},le=\"{}\"}} {}",
            bucket.le_ms, bucket.count
        );
    }
    let _ = writeln!(
        body,
        "crypto_arb_http_request_latency_ms_bucket{{{labels},le=\"+Inf\"}} {}",
        row.request_total
    );
    if let Some(p95_ms) = row.latency_p95_ms {
        let _ = writeln!(
            body,
            "crypto_arb_http_request_latency_ms_p95_bucket{{{labels}}} {p95_ms}"
        );
    }
}

fn http_outcome_labels(row: &HttpOutcomeMetricSnapshot) -> String {
    let exchange = prometheus_label_value(&row.exchange);
    let method = prometheus_label_value(&row.method);
    let path = prometheus_label_value(&row.path);
    let outcome = prometheus_label_value(&row.outcome);
    let status = row
        .status_code
        .map(|status| status.to_string())
        .unwrap_or_else(|| "none".to_owned());
    format!(
        "exchange=\"{exchange}\",method=\"{method}\",path=\"{path}\",outcome=\"{outcome}\",status=\"{status}\""
    )
}

fn write_host_gate_metrics(body: &mut String, rows: &[HostGateSnapshot]) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_host_gate_state Current outbound HTTP HostGate state by exchange and host (value is always 1)."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_http_host_gate_state gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_host_gate_retry_after_ms Current HostGate retry-after delay in milliseconds by exchange and host."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_host_gate_retry_after_ms gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_host_gate_consecutive_failures Current HostGate consecutive failure count by exchange and host."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_host_gate_consecutive_failures gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_host_gate_inflight_keys Current retained HostGate singleflight key count by exchange and host."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_http_host_gate_inflight_keys gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_host_gate_inflight_active_keys Current active HostGate singleflight key count by exchange and host."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_host_gate_inflight_active_keys gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_host_gate_inflight_pruned_total Total HostGate singleflight keys pruned by exchange and host."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_host_gate_inflight_pruned_total counter"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_http_host_gate_inflight_oldest_idle_ms Oldest idle HostGate singleflight key age in milliseconds by exchange and host."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_http_host_gate_inflight_oldest_idle_ms gauge"
    );
    for row in rows {
        write_host_gate_metric_rows(body, row);
    }
}

fn write_host_gate_metric_rows(body: &mut String, row: &HostGateSnapshot) {
    let labels = host_gate_labels(row);
    let state = host_gate_state(row);
    let _ = writeln!(
        body,
        "crypto_arb_http_host_gate_state{{{labels},state=\"{state}\"}} 1"
    );
    if let Some(retry_after_ms) = host_gate_retry_after_ms(row) {
        let _ = writeln!(
            body,
            "crypto_arb_http_host_gate_retry_after_ms{{{labels},state=\"{state}\"}} {retry_after_ms}"
        );
    }
    let _ = writeln!(
        body,
        "crypto_arb_http_host_gate_consecutive_failures{{{labels}}} {}",
        row.consecutive_failures
    );
    let _ = writeln!(
        body,
        "crypto_arb_http_host_gate_inflight_keys{{{labels}}} {}",
        row.inflight_keys
    );
    let _ = writeln!(
        body,
        "crypto_arb_http_host_gate_inflight_active_keys{{{labels}}} {}",
        row.inflight_active_keys
    );
    let _ = writeln!(
        body,
        "crypto_arb_http_host_gate_inflight_pruned_total{{{labels}}} {}",
        row.inflight_pruned_total
    );
    if let Some(oldest_idle_ms) = row.inflight_oldest_idle_ms {
        let _ = writeln!(
            body,
            "crypto_arb_http_host_gate_inflight_oldest_idle_ms{{{labels}}} {oldest_idle_ms}"
        );
    }
}

fn host_gate_labels(row: &HostGateSnapshot) -> String {
    let exchange = prometheus_label_value(&row.exchange);
    let host = prometheus_label_value(&row.host);
    format!("exchange=\"{exchange}\",host=\"{host}\"")
}

fn host_gate_state(row: &HostGateSnapshot) -> &'static str {
    if row.rate_limit_retry_after_ms.is_some() {
        "rate_limited"
    } else if row.circuit_retry_after_ms.is_some() {
        "circuit_open"
    } else {
        "ok"
    }
}

fn host_gate_retry_after_ms(row: &HostGateSnapshot) -> Option<u64> {
    row.rate_limit_retry_after_ms.or(row.circuit_retry_after_ms)
}

fn write_rate_limiter_metrics(body: &mut String, rows: &[RateLimiterSnapshot]) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_rate_limiter_qps Configured rate limiter QPS by limiter."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_rate_limiter_qps gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_rate_limiter_wait_total Total rate limiter token waits by limiter."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_rate_limiter_wait_total counter");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_rate_limiter_wait_ms_total Total rate limiter wait time in milliseconds by limiter."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_rate_limiter_wait_ms_total counter");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_rate_limiter_try_acquire_total Total immediate token acquisition attempts by limiter."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_rate_limiter_try_acquire_total counter"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_rate_limiter_try_acquire_rejected_total Total immediate token acquisition rejections by limiter."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_rate_limiter_try_acquire_rejected_total counter"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_rate_limiter_last_wait_ms Last observed rate limiter wait in milliseconds by limiter."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_rate_limiter_last_wait_ms gauge");
    for row in rows {
        write_rate_limiter_metric_rows(body, row);
    }
}

fn write_rate_limiter_metric_rows(body: &mut String, row: &RateLimiterSnapshot) {
    let labels = rate_limiter_labels(row);
    let _ = writeln!(body, "crypto_arb_rate_limiter_qps{{{labels}}} {}", row.qps);
    let _ = writeln!(
        body,
        "crypto_arb_rate_limiter_wait_total{{{labels}}} {}",
        row.wait_total
    );
    let _ = writeln!(
        body,
        "crypto_arb_rate_limiter_wait_ms_total{{{labels}}} {}",
        row.wait_ms_total
    );
    let _ = writeln!(
        body,
        "crypto_arb_rate_limiter_try_acquire_total{{{labels}}} {}",
        row.try_acquire_total
    );
    let _ = writeln!(
        body,
        "crypto_arb_rate_limiter_try_acquire_rejected_total{{{labels}}} {}",
        row.try_acquire_rejected_total
    );
    if let Some(last_wait_ms) = row.last_wait_ms {
        let _ = writeln!(
            body,
            "crypto_arb_rate_limiter_last_wait_ms{{{labels}}} {last_wait_ms}"
        );
    }
}

fn rate_limiter_labels(row: &RateLimiterSnapshot) -> String {
    let limiter = prometheus_label_value(&row.name);
    let parent = row
        .parent
        .as_deref()
        .map(prometheus_label_value)
        .unwrap_or_else(|| "none".to_owned());
    format!("limiter=\"{limiter}\",parent=\"{parent}\"")
}

fn write_http_metric_row(
    body: &mut String,
    metric: &str,
    row: &HttpRequestMetricSnapshot,
    value: u64,
) {
    let exchange = prometheus_label_value(&row.exchange);
    let method = prometheus_label_value(&row.method);
    let path = prometheus_label_value(&row.path);
    let _ = writeln!(
        body,
        "{metric}{{exchange=\"{exchange}\",method=\"{method}\",path=\"{path}\"}} {value}"
    );
}

fn write_ws_metrics(
    body: &mut String,
    snap: crate::metrics::MetricsSnapshot,
    hub: &realtime::WsHub,
    channels: &[String],
) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_ws_arbitrage_payload_bytes Serialized bytes of the latest arbitrage WebSocket payload."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_ws_arbitrage_payload_bytes gauge");
    let _ = writeln!(
        body,
        "crypto_arb_ws_arbitrage_payload_bytes {}",
        snap.ws_arbitrage_payload_bytes
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_ws_arbitrage_top_ids Top opportunity ids carried by the latest arbitrage WebSocket payload."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_ws_arbitrage_top_ids gauge");
    let _ = writeln!(
        body,
        "crypto_arb_ws_arbitrage_top_ids {}",
        snap.ws_arbitrage_top_ids
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_ws_arbitrage_changed_ids Changed opportunity ids carried by the latest arbitrage WebSocket payload."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_ws_arbitrage_changed_ids gauge");
    let _ = writeln!(
        body,
        "crypto_arb_ws_arbitrage_changed_ids {}",
        snap.ws_arbitrage_changed_ids
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_ws_arbitrage_changed_rows Changed lightweight opportunity rows carried by the latest arbitrage WebSocket payload."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_ws_arbitrage_changed_rows gauge");
    let _ = writeln!(
        body,
        "crypto_arb_ws_arbitrage_changed_rows {}",
        snap.ws_arbitrage_changed_rows
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_ws_arbitrage_removed_ids Removed opportunity ids carried by the latest arbitrage WebSocket payload."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_ws_arbitrage_removed_ids gauge");
    let _ = writeln!(
        body,
        "crypto_arb_ws_arbitrage_removed_ids {}",
        snap.ws_arbitrage_removed_ids
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_ws_subscribers Number of active WebSocket subscribers per channel."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_ws_subscribers gauge");
    for ch in channels {
        let subs = hub.subscriber_count(ch);
        let label = prometheus_label_value(ch);
        let _ = writeln!(
            body,
            "crypto_arb_ws_subscribers{{channel=\"{label}\"}} {subs}"
        );
    }

    let _ = writeln!(
        body,
        "# HELP crypto_arb_ws_channels Number of registered WebSocket channels currently broadcasting."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_ws_channels gauge");
    let _ = writeln!(body, "crypto_arb_ws_channels {}", channels.len());
}

fn write_opportunity_rest_metrics(body: &mut String, snap: crate::metrics::MetricsSnapshot) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_rest_opportunity_list_payload_bytes Serialized bytes of the latest opportunity list REST payload."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_rest_opportunity_list_payload_bytes gauge"
    );
    let _ = writeln!(
        body,
        "crypto_arb_rest_opportunity_list_payload_bytes {}",
        snap.rest_opportunity_list_payload_bytes
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_rest_opportunity_list_serde_ms Wall-clock milliseconds spent measuring the latest opportunity list REST payload serialization."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_rest_opportunity_list_serde_ms gauge"
    );
    let _ = writeln!(
        body,
        "crypto_arb_rest_opportunity_list_serde_ms {}",
        snap.rest_opportunity_list_serde_ms
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_rest_opportunity_list_rows Lightweight rows carried by the latest opportunity list REST payload."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_rest_opportunity_list_rows gauge");
    let _ = writeln!(
        body,
        "crypto_arb_rest_opportunity_list_rows {}",
        snap.rest_opportunity_list_rows
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_rest_opportunity_detail_seed_payload_bytes Serialized bytes of the latest legacy opportunity detail seed REST payload."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_rest_opportunity_detail_seed_payload_bytes gauge"
    );
    let _ = writeln!(
        body,
        "crypto_arb_rest_opportunity_detail_seed_payload_bytes {}",
        snap.rest_opportunity_detail_seed_payload_bytes
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_rest_opportunity_detail_seed_serde_ms Wall-clock milliseconds spent measuring the latest legacy opportunity detail seed REST payload serialization."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_rest_opportunity_detail_seed_serde_ms gauge"
    );
    let _ = writeln!(
        body,
        "crypto_arb_rest_opportunity_detail_seed_serde_ms {}",
        snap.rest_opportunity_detail_seed_serde_ms
    );
}

fn write_task_health_metrics(body: &mut String, now_ms: i64, snapshots: &[TaskSnapshot]) {
    let total = snapshots.len();
    let enabled_count = snapshots.iter().filter(|snapshot| snapshot.enabled).count();
    let disabled_count = total.saturating_sub(enabled_count);
    let unhealthy_count = snapshots
        .iter()
        .filter(|snapshot| snapshot.issue.is_some())
        .count();
    let healthy = enabled_count.saturating_sub(unhealthy_count);

    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_tasks_total Number of registered background tasks (including exited)."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_tasks_total gauge");
    let _ = writeln!(body, "crypto_arb_background_tasks_total {total}");

    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_tasks_enabled Number of configured background tasks."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_tasks_enabled gauge");
    let _ = writeln!(body, "crypto_arb_background_tasks_enabled {enabled_count}");

    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_tasks_disabled Number of explicitly disabled background tasks."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_tasks_disabled gauge");
    let _ = writeln!(
        body,
        "crypto_arb_background_tasks_disabled {disabled_count}"
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_tasks_healthy Number of background tasks currently healthy."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_tasks_healthy gauge");
    let _ = writeln!(body, "crypto_arb_background_tasks_healthy {healthy}");

    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_tasks_unhealthy Number of background tasks currently dead, stale, or failing."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_tasks_unhealthy gauge");
    let _ = writeln!(
        body,
        "crypto_arb_background_tasks_unhealthy {unhealthy_count}"
    );

    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_unhealthy Unhealthy background tasks labeled by task and issue code (value is always 1)."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_task_unhealthy gauge");
    write_task_detail_metric_headers(body);
    for snapshot in snapshots {
        write_task_detail_metrics(body, now_ms, snapshot);
    }
}

fn write_task_detail_metric_headers(body: &mut String) {
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_enabled Whether a background task is configured (1 enabled, 0 disabled)."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_task_enabled gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_status Background task current status by task and status label (value is always 1)."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_task_status gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_last_tick_age_ms Milliseconds since a background task last reported progress."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_last_tick_age_ms gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_lag_ms Runtime task progress lag in milliseconds."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_task_lag_ms gauge");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_retry_after_ms Remaining supervisor backoff in milliseconds."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_retry_after_ms gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_last_success_at_ms Unix epoch milliseconds when a background task last reported success."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_last_success_at_ms gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_consecutive_failures Current consecutive background task failures."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_consecutive_failures gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_exit_total Background task exits observed since process start."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_background_task_exit_total counter");
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_restart_total Background task restarts performed by the bounded supervisor since process start."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_restart_total counter"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_max_silence_ms Background task stale threshold in milliseconds."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_max_silence_ms gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_last_duration_ms Wall-clock duration of the latest completed background task iteration."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_last_duration_ms gauge"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_slow_tick_total Total background task iterations that exceeded the slow threshold."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_slow_tick_total counter"
    );
    let _ = writeln!(
        body,
        "# HELP crypto_arb_background_task_slow_threshold_ms Background task slow-iteration threshold in milliseconds."
    );
    let _ = writeln!(
        body,
        "# TYPE crypto_arb_background_task_slow_threshold_ms gauge"
    );
}

fn write_task_detail_metrics(body: &mut String, now_ms: i64, snapshot: &TaskSnapshot) {
    let task = prometheus_label_value(snapshot.name);
    let status = task_status_label(snapshot);
    let status_label = prometheus_label_value(status);
    let _ = writeln!(
        body,
        "crypto_arb_background_task_status{{task=\"{task}\",status=\"{status_label}\"}} 1"
    );
    let _ = writeln!(
        body,
        "crypto_arb_background_task_enabled{{task=\"{task}\"}} {}",
        u8::from(snapshot.enabled)
    );
    let _ = writeln!(
        body,
        "crypto_arb_background_task_last_tick_age_ms{{task=\"{task}\"}} {}",
        freshness_since(snapshot.last_tick_ms, now_ms)
    );
    let _ = writeln!(
        body,
        "crypto_arb_background_task_lag_ms{{task=\"{task}\"}} {}",
        snapshot.lag_ms
    );
    if let Some(retry_after_ms) = snapshot.retry_after_ms {
        let _ = writeln!(
            body,
            "crypto_arb_background_task_retry_after_ms{{task=\"{task}\"}} {retry_after_ms}"
        );
    }
    if let Some(last_success_ms) = snapshot.last_success_ms {
        let _ = writeln!(
            body,
            "crypto_arb_background_task_last_success_at_ms{{task=\"{task}\"}} {last_success_ms}"
        );
    }
    let _ = writeln!(
        body,
        "crypto_arb_background_task_consecutive_failures{{task=\"{task}\"}} {}",
        snapshot.consecutive_failures
    );
    let _ = writeln!(
        body,
        "crypto_arb_background_task_exit_total{{task=\"{task}\"}} {}",
        snapshot.exit_count
    );
    let _ = writeln!(
        body,
        "crypto_arb_background_task_restart_total{{task=\"{task}\"}} {}",
        snapshot.restart_count
    );
    let _ = writeln!(
        body,
        "crypto_arb_background_task_max_silence_ms{{task=\"{task}\"}} {}",
        snapshot.max_silence_ms
    );
    if let Some(last_duration_ms) = snapshot.last_duration_ms {
        let _ = writeln!(
            body,
            "crypto_arb_background_task_last_duration_ms{{task=\"{task}\"}} {last_duration_ms}"
        );
    }
    let _ = writeln!(
        body,
        "crypto_arb_background_task_slow_tick_total{{task=\"{task}\"}} {}",
        snapshot.slow_tick_count
    );
    let _ = writeln!(
        body,
        "crypto_arb_background_task_slow_threshold_ms{{task=\"{task}\"}} {}",
        snapshot.slow_threshold_ms
    );
    if let Some(issue) = &snapshot.issue {
        let code = issue.kind.code();
        let _ = writeln!(
            body,
            "crypto_arb_background_task_unhealthy{{task=\"{task}\",code=\"{code}\"}} 1"
        );
    }
}

fn task_status_label(snapshot: &TaskSnapshot) -> &'static str {
    if !snapshot.enabled {
        return "disabled";
    }
    snapshot
        .issue
        .as_ref()
        .map(|issue| issue.kind.code())
        .unwrap_or("ok")
}

fn freshness_since(observed_at_ms: i64, now_ms: i64) -> i64 {
    now_ms.saturating_sub(observed_at_ms).max(0)
}

fn write_runtime_state_metrics(body: &mut String, inventory: &RuntimeStateInventory) {
    let backend = prometheus_label_value(inventory.history_backend);
    let _ = writeln!(
        body,
        "# HELP crypto_arb_history_store_info Active history store backend (value is always 1)."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_history_store_info gauge");
    let _ = writeln!(
        body,
        "crypto_arb_history_store_info{{backend=\"{backend}\"}} 1"
    );

    if let Some(rows) = inventory.history_rows {
        let _ = writeln!(
            body,
            "# HELP crypto_arb_history_store_rows Approximate rows in the active history store when cheaply available."
        );
        let _ = writeln!(body, "# TYPE crypto_arb_history_store_rows gauge");
        let _ = writeln!(
            body,
            "crypto_arb_history_store_rows{{backend=\"{backend}\"}} {rows}"
        );
    }

    let _ = writeln!(
        body,
        "# HELP crypto_arb_runtime_state_entries Records held by process-local runtime state stores."
    );
    let _ = writeln!(body, "# TYPE crypto_arb_runtime_state_entries gauge");
    for store in &inventory.stores {
        let store_name = prometheus_label_value(store.name);
        let persistence = prometheus_label_value(store.persistence);
        let _ = writeln!(
            body,
            "crypto_arb_runtime_state_entries{{store=\"{store_name}\",persistence=\"{persistence}\"}} {}",
            store.count
        );
    }
}

fn prometheus_label_value(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\n' => escaped.push_str("\\n"),
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MetricsSnapshot;
    use crate::task_registry::TaskIssue;

    #[test]
    fn prometheus_label_value_escapes_reserved_chars() {
        assert_eq!(prometheus_label_value("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn render_metrics_body_includes_snapshot_and_ws_channels() {
        let hub = realtime::WsHub::new(16);
        let _rx = hub.subscribe("weird\"channel\\name\nx");
        let funding_by_exchange = BTreeMap::from([
            ("binance".to_owned(), 691),
            ("okx".to_owned(), 318),
            ("weird\"exchange\\name\nx".to_owned(), 1),
        ]);
        let snap = sample_metrics_snapshot();
        let market_data = sample_market_data_stats();
        let market_health = sample_market_health();
        let market_cache_access = sample_market_cache_access();
        let http_requests = sample_http_requests();
        let http_outcomes = sample_http_outcomes();
        let host_gates = sample_host_gates();
        let rate_limiters = sample_rate_limiters();
        let now_ms = 1_700_000_001_000;
        let task_snapshots = sample_task_snapshots();
        let runtime_state = sample_runtime_state();

        let body = render_metrics_body(&MetricsRenderInput {
            snap,
            market_data,
            market_cache_access: &market_cache_access,
            market_health: &market_health,
            hub: &hub,
            funding_by_exchange: &funding_by_exchange,
            http_requests: &http_requests,
            http_outcomes: &http_outcomes,
            host_gates: &host_gates,
            rate_limiters: &rate_limiters,
            runtime_state: &runtime_state,
            now_ms,
            task_snapshots: &task_snapshots,
        });

        assert_snapshot_metrics(&body);
        assert_market_metrics(&body);
        assert_market_health_metrics(&body);
        assert_http_metrics(&body);
        assert_host_gate_metrics(&body);
        assert_rate_limiter_metrics(&body);
        assert_label_metrics(&body);
        assert_runtime_state_metrics(&body);
        assert_task_health_metrics(&body);
    }

    fn sample_metrics_snapshot() -> MetricsSnapshot {
        MetricsSnapshot {
            arb_scan_total: 7,
            arb_last_scan_ms: 1234,
            arb_last_count: 56,
            arb_last_at_ms: 1_700_000_001_000,
            ws_arbitrage_payload_bytes: 321,
            ws_arbitrage_top_ids: 20,
            ws_arbitrage_changed_ids: 3,
            ws_arbitrage_changed_rows: 2,
            ws_arbitrage_removed_ids: 2,
            rest_opportunity_list_payload_bytes: 45_678,
            rest_opportunity_list_serde_ms: 6,
            rest_opportunity_list_rows: 120,
            rest_opportunity_detail_seed_payload_bytes: 9_876,
            rest_opportunity_detail_seed_serde_ms: 2,
            funding_fetch_total: 8,
            funding_last_scan_ms: 2345,
            funding_last_count: 3818,
            funding_last_at_ms: 1_700_000_002_000,
            alerts_fired_total: 2,
        }
    }

    fn sample_market_data_stats() -> MarketDataStats {
        MarketDataStats {
            cache_hit_total: 95,
            cache_miss_total: 5,
            cache_stale_total: 3,
            cache_hit_ratio: 0.95,
            rest_baseline_orderbook_guard_keys: 7,
            rest_baseline_orderbook_in_flight: 2,
            rest_baseline_orderbook_wait_count_total: 11,
            rest_baseline_orderbook_wait_ms_total: 44,
            rest_baseline_orderbook_guard_evicted_total: 5,
            rest_baseline_orderbook_guard_oldest_idle_ms: 1_500,
            rest_baseline_snapshot_feed_keys: 2,
            rest_baseline_snapshot_feed_in_flight: 1,
            rest_baseline_snapshot_wait_count_total: 3,
            rest_baseline_snapshot_wait_ms_total: 12,
            perp_ticker_snapshot_served_stale_total: 4,
            spot_tick_snapshot_served_stale_total: 6,
        }
    }

    fn sample_market_health() -> Vec<MarketRuntimeHealth> {
        vec![MarketRuntimeHealth {
            venue: "bybit".to_owned(),
            operation: "ws_ticker",
            quality: crate::services::market_data::MarketQuality::RateLimited,
            source: crate::services::market_data::MarketSource::WsPush,
            requested: 2,
            rows: 0,
            retry_after_ms: Some(3_000),
            last_error: Some("rate limited".to_owned()),
            problem: None,
            observed_at_ms: 1_700_000_000_000,
        }]
    }

    fn sample_market_cache_access() -> Vec<MarketCacheAccessMetric> {
        vec![MarketCacheAccessMetric {
            key: crate::services::market_data::cache::MarketCacheAccessKey {
                feed: "orderbook",
                outcome: "hit",
                source: crate::services::market_data::MarketSource::RestBaseline,
                quality: crate::services::market_data::MarketQuality::Fresh,
            },
            count: 9,
        }]
    }

    fn sample_http_requests() -> Vec<HttpRequestMetricSnapshot> {
        vec![HttpRequestMetricSnapshot {
            exchange: "binance".to_owned(),
            method: "GET".to_owned(),
            path: "/fapi/v1/depth".to_owned(),
            request_total: 3,
            weight_total: 6,
        }]
    }

    fn sample_http_outcomes() -> Vec<HttpOutcomeMetricSnapshot> {
        vec![
            HttpOutcomeMetricSnapshot {
                exchange: "binance".to_owned(),
                method: "GET".to_owned(),
                path: "/fapi/v1/depth".to_owned(),
                endpoint_evidence: exchange::endpoint_evidence(
                    "binance",
                    exchange::HttpMethod::Get,
                    "/fapi/v1/depth",
                ),
                outcome: "success".to_owned(),
                status_code: Some(200),
                request_total: 2,
                retry_total: 0,
                latency_ms_total: 15,
                retry_after_ms_total: 0,
                latency_buckets: sample_latency_buckets(2),
                latency_p95_ms: Some(25),
                last_latency_ms: 9,
                last_retry_after_ms: None,
                last_request_id: Some("req-metrics-binance".to_owned()),
                last_request_context: vec!["instId=BTC-USDT-SWAP".to_owned()],
                last_observed_at_ms: 1_700_000_000_000,
            },
            HttpOutcomeMetricSnapshot {
                exchange: "bybit".to_owned(),
                method: "GET".to_owned(),
                path: "/v5/market/tickers".to_owned(),
                endpoint_evidence: exchange::endpoint_evidence(
                    "bybit",
                    exchange::HttpMethod::Get,
                    "/v5/market/tickers",
                ),
                outcome: "rate_limited".to_owned(),
                status_code: Some(429),
                request_total: 1,
                retry_total: 1,
                latency_ms_total: 4,
                retry_after_ms_total: 3_000,
                latency_buckets: sample_latency_buckets(1),
                latency_p95_ms: Some(10),
                last_latency_ms: 4,
                last_retry_after_ms: Some(3_000),
                last_request_id: Some("req-metrics-bybit".to_owned()),
                last_request_context: vec!["contract_code=BTC-USDT".to_owned()],
                last_observed_at_ms: 1_700_000_000_000,
            },
            HttpOutcomeMetricSnapshot {
                exchange: "okx".to_owned(),
                method: "GET".to_owned(),
                path: "/api/v5/market/tickers".to_owned(),
                endpoint_evidence: exchange::endpoint_evidence(
                    "okx",
                    exchange::HttpMethod::Get,
                    "/api/v5/market/tickers",
                ),
                outcome: "circuit_open".to_owned(),
                status_code: None,
                request_total: 1,
                retry_total: 1,
                latency_ms_total: 0,
                retry_after_ms_total: 0,
                latency_buckets: sample_latency_buckets(1),
                latency_p95_ms: Some(10),
                last_latency_ms: 0,
                last_retry_after_ms: None,
                last_request_id: Some("req-metrics-okx".to_owned()),
                last_request_context: vec!["instId=BTC-USDT-SWAP".to_owned()],
                last_observed_at_ms: 1_700_000_000_000,
            },
        ]
    }

    fn sample_latency_buckets(count: u64) -> Vec<exchange::HttpLatencyBucketSnapshot> {
        vec![
            exchange::HttpLatencyBucketSnapshot { le_ms: 10, count },
            exchange::HttpLatencyBucketSnapshot { le_ms: 25, count },
        ]
    }

    fn sample_host_gates() -> Vec<HostGateSnapshot> {
        vec![
            HostGateSnapshot {
                exchange: "bybit".to_owned(),
                host: "api.bybit.com".to_owned(),
                consecutive_failures: 0,
                rate_limit_retry_after_ms: Some(2_000),
                circuit_retry_after_ms: None,
                inflight_keys: 3,
                inflight_active_keys: 1,
                inflight_pruned_total: 4,
                inflight_oldest_idle_ms: Some(700),
                observed_at_ms: 1_700_000_000_000,
            },
            HostGateSnapshot {
                exchange: "okx".to_owned(),
                host: "www.okx.com".to_owned(),
                consecutive_failures: 2,
                rate_limit_retry_after_ms: None,
                circuit_retry_after_ms: Some(5_000),
                inflight_keys: 1,
                inflight_active_keys: 0,
                inflight_pruned_total: 2,
                inflight_oldest_idle_ms: Some(300),
                observed_at_ms: 1_700_000_000_000,
            },
        ]
    }

    fn sample_rate_limiters() -> Vec<RateLimiterSnapshot> {
        vec![RateLimiterSnapshot {
            name: "binance".to_owned(),
            qps: 20,
            parent: Some("global-binance".to_owned()),
            wait_total: 3,
            wait_ms_total: 250,
            try_acquire_total: 4,
            try_acquire_rejected_total: 1,
            last_wait_ms: Some(80),
            last_wait_observed_at_ms: Some(1_700_000_000_000),
            last_rejected_observed_at_ms: Some(1_700_000_000_100),
            observed_at_ms: 1_700_000_000_200,
        }]
    }

    fn sample_task_snapshots() -> Vec<TaskSnapshot> {
        let mut snapshots = vec![
            sample_task_snapshot(
                "portfolio",
                None,
                1_700_000_000_900,
                Some(1_700_000_000_800),
                0,
                0,
            ),
            sample_task_snapshot(
                "system_health",
                None,
                1_700_000_000_850,
                Some(1_700_000_000_850),
                0,
                0,
            ),
            sample_slow_task_snapshot("market_prewarm", 1_700_000_000_700),
            sample_task_snapshot("private_ws_supervisor", None, 1_700_000_000_650, None, 0, 0),
            sample_task_snapshot(
                "reconciliation",
                None,
                1_700_000_000_500,
                Some(1_700_000_000_500),
                0,
                0,
            ),
            sample_task_snapshot("ws_housekeeping", None, 1_700_000_000_400, None, 0, 0),
            sample_disabled_task_snapshot("ledger_projection_jobs"),
            sample_task_snapshot(
                "snapshot",
                Some(TaskIssue {
                    name: "snapshot",
                    kind: crate::task_registry::TaskIssueKind::Dead,
                    detail: "panicked: boom".to_owned(),
                    since_ms: Some(1_700_000_000_000),
                }),
                1_700_000_000_000,
                None,
                0,
                1,
            ),
            sample_task_snapshot(
                "funding",
                Some(TaskIssue {
                    name: "funding",
                    kind: crate::task_registry::TaskIssueKind::Failing,
                    detail: "3 consecutive failures".to_owned(),
                    since_ms: None,
                }),
                1_700_000_000_200,
                Some(1_699_999_990_000),
                3,
                0,
            ),
        ];
        if let Some(snapshot) = snapshots
            .iter_mut()
            .find(|snapshot| snapshot.name == "snapshot")
        {
            snapshot.retry_after_ms = Some(5_000);
        }
        snapshots
    }

    fn sample_task_snapshot(
        name: &'static str,
        issue: Option<TaskIssue>,
        last_tick_ms: i64,
        last_success_ms: Option<i64>,
        consecutive_failures: u32,
        exit_count: u32,
    ) -> TaskSnapshot {
        TaskSnapshot {
            name,
            enabled: true,
            running: issue
                .as_ref()
                .is_none_or(|issue| issue.kind != crate::task_registry::TaskIssueKind::Dead),
            started_at_ms: 1_699_999_000_000,
            last_tick_ms,
            last_success_ms,
            consecutive_failures,
            last_error: (consecutive_failures > 0).then(|| "task failed".to_owned()),
            last_exit_reason: (exit_count > 0).then(|| "panicked: boom".to_owned()),
            last_exit_at_ms: (exit_count > 0).then_some(1_700_000_000_000),
            exit_count,
            restart_count: exit_count,
            max_silence_ms: 60_000,
            slow_threshold_ms: 60_000,
            last_duration_ms: Some(25),
            slow_tick_count: 0,
            lag_ms: 1_700_000_001_000_i64.saturating_sub(last_tick_ms),
            retry_after_ms: None,
            issue,
        }
    }

    fn sample_disabled_task_snapshot(name: &'static str) -> TaskSnapshot {
        let mut snapshot = sample_task_snapshot(name, None, 1_700_000_001_000, None, 0, 0);
        snapshot.enabled = false;
        snapshot.running = false;
        snapshot.last_duration_ms = None;
        snapshot
    }

    fn sample_slow_task_snapshot(name: &'static str, last_tick_ms: i64) -> TaskSnapshot {
        let mut snapshot = sample_task_snapshot(name, None, last_tick_ms, None, 0, 0);
        snapshot.last_duration_ms = Some(70_000);
        snapshot.slow_tick_count = 2;
        snapshot
    }

    fn sample_runtime_state() -> RuntimeStateInventory {
        RuntimeStateInventory {
            history_backend: "memory",
            history_rows: Some(4),
            stores: vec![
                runtime_state::RuntimeStateStore {
                    name: "watchlist",
                    persistence: "memory",
                    count: 2,
                },
                runtime_state::RuntimeStateStore {
                    name: "execution_runs",
                    persistence: "jsonl_snapshot",
                    count: 1,
                },
                runtime_state::RuntimeStateStore {
                    name: "close_runs",
                    persistence: "jsonl_snapshot",
                    count: 3,
                },
            ],
        }
    }

    fn assert_runtime_state_metrics(body: &str) {
        assert!(body.contains("crypto_arb_history_store_info{backend=\"memory\"} 1"));
        assert!(body.contains("crypto_arb_history_store_rows{backend=\"memory\"} 4"));
        assert!(body.contains(
            "crypto_arb_runtime_state_entries{store=\"watchlist\",persistence=\"memory\"} 2"
        ));
        assert!(body.contains(
            "crypto_arb_runtime_state_entries{store=\"execution_runs\",persistence=\"jsonl_snapshot\"} 1"
        ));
        assert!(body.contains(
            "crypto_arb_runtime_state_entries{store=\"close_runs\",persistence=\"jsonl_snapshot\"} 3"
        ));
    }

    fn assert_task_health_metrics(body: &str) {
        assert!(body.contains("crypto_arb_background_tasks_total 9"));
        assert!(body.contains("crypto_arb_background_tasks_enabled 8"));
        assert!(body.contains("crypto_arb_background_tasks_disabled 1"));
        assert!(body.contains("crypto_arb_background_tasks_healthy 6"));
        assert!(body.contains("crypto_arb_background_tasks_unhealthy 2"));
        assert_task_status_metrics(body);
        assert_task_timing_metrics(body);
        assert_task_slow_metrics(body);
        assert_task_issue_metrics(body);
    }

    fn assert_task_status_metrics(body: &str) {
        assert!(
            body.contains("crypto_arb_background_task_status{task=\"portfolio\",status=\"ok\"} 1")
        );
        assert!(body.contains(
            "crypto_arb_background_task_status{task=\"snapshot\",status=\"TASK_DOWN\"} 1"
        ));
        assert!(body.contains(
            "crypto_arb_background_task_status{task=\"ledger_projection_jobs\",status=\"disabled\"} 1"
        ));
        assert!(
            body.contains("crypto_arb_background_task_enabled{task=\"ledger_projection_jobs\"} 0")
        );
    }

    fn assert_task_timing_metrics(body: &str) {
        assert!(
            body.contains("crypto_arb_background_task_last_tick_age_ms{task=\"portfolio\"} 100")
        );
        assert!(body.contains(
            "crypto_arb_background_task_last_success_at_ms{task=\"portfolio\"} 1700000000800"
        ));
        assert!(body.contains("crypto_arb_background_task_lag_ms{task=\"portfolio\"} 100"));
        assert!(body.contains("crypto_arb_background_task_retry_after_ms{task=\"snapshot\"} 5000"));
        assert!(
            body.contains("crypto_arb_background_task_consecutive_failures{task=\"funding\"} 3")
        );
        assert!(body.contains("crypto_arb_background_task_exit_total{task=\"snapshot\"} 1"));
        assert!(body.contains("crypto_arb_background_task_restart_total{task=\"snapshot\"} 1"));
        assert!(body
            .contains("crypto_arb_background_task_max_silence_ms{task=\"market_prewarm\"} 60000"));
    }

    fn assert_task_slow_metrics(body: &str) {
        assert!(body.contains(
            "crypto_arb_background_task_last_duration_ms{task=\"market_prewarm\"} 70000"
        ));
        assert!(
            body.contains("crypto_arb_background_task_slow_tick_total{task=\"market_prewarm\"} 2")
        );
        assert!(body.contains(
            "crypto_arb_background_task_slow_threshold_ms{task=\"market_prewarm\"} 60000"
        ));
    }

    fn assert_task_issue_metrics(body: &str) {
        assert!(body.contains(
            "crypto_arb_background_task_unhealthy{task=\"snapshot\",code=\"TASK_DOWN\"} 1"
        ));
        assert!(body.contains(
            "crypto_arb_background_task_unhealthy{task=\"funding\",code=\"TASK_FAILING\"} 1"
        ));
    }

    fn assert_snapshot_metrics(body: &str) {
        assert_scan_snapshot_metrics(body);
        assert_ws_snapshot_metrics(body);
        assert_opportunity_rest_snapshot_metrics(body);
        assert_funding_snapshot_metrics(body);
    }

    fn assert_scan_snapshot_metrics(body: &str) {
        assert!(body.contains("crypto_arb_scan_total 7"));
        assert!(body.contains("crypto_arb_last_scan_ms 1234"));
        assert!(body.contains("crypto_arb_last_count 56"));
    }

    fn assert_ws_snapshot_metrics(body: &str) {
        assert!(body.contains("# TYPE crypto_arb_ws_arbitrage_payload_bytes gauge"));
        assert!(body.contains("crypto_arb_ws_arbitrage_payload_bytes 321"));
        assert!(body.contains("crypto_arb_ws_arbitrage_top_ids 20"));
        assert!(body.contains("crypto_arb_ws_arbitrage_changed_ids 3"));
        assert!(body.contains("crypto_arb_ws_arbitrage_changed_rows 2"));
        assert!(body.contains("crypto_arb_ws_arbitrage_removed_ids 2"));
    }

    fn assert_opportunity_rest_snapshot_metrics(body: &str) {
        assert!(body.contains("# TYPE crypto_arb_rest_opportunity_list_payload_bytes gauge"));
        assert!(body.contains("crypto_arb_rest_opportunity_list_payload_bytes 45678"));
        assert!(body.contains("crypto_arb_rest_opportunity_list_serde_ms 6"));
        assert!(body.contains("crypto_arb_rest_opportunity_list_rows 120"));
        assert!(body.contains("# TYPE crypto_arb_rest_opportunity_detail_seed_payload_bytes gauge"));
        assert!(body.contains("crypto_arb_rest_opportunity_detail_seed_payload_bytes 9876"));
        assert!(body.contains("crypto_arb_rest_opportunity_detail_seed_serde_ms 2"));
    }

    fn assert_funding_snapshot_metrics(body: &str) {
        assert!(body.contains("crypto_arb_funding_fetch_total 8"));
        assert!(body.contains("crypto_arb_funding_last_count 3818"));
        assert!(body.contains("crypto_arb_alerts_fired_total 2"));
    }

    fn assert_market_metrics(body: &str) {
        assert_market_cache_metrics(body);
        assert_market_cache_access_metrics(body);
        assert_market_snapshot_stale_metrics(body);
        assert_market_rest_baseline_metrics(body);
    }

    fn assert_market_cache_metrics(body: &str) {
        assert!(body.contains("crypto_arb_market_cache_hit_total 95"));
        assert!(body.contains("crypto_arb_market_cache_miss_total 5"));
        assert!(body.contains("crypto_arb_market_cache_stale_total 3"));
        assert!(body.contains("crypto_arb_market_cache_hit_ratio 0.950000"));
    }

    fn assert_market_cache_access_metrics(body: &str) {
        assert!(body.contains(
            "crypto_arb_market_cache_access_total{feed=\"orderbook\",outcome=\"hit\",source=\"rest_baseline\",quality=\"fresh\"} 9"
        ));
    }

    fn assert_market_snapshot_stale_metrics(body: &str) {
        assert!(
            body.contains("crypto_arb_market_snapshot_served_stale_total{feed=\"perp_tickers\"} 4")
        );
        assert!(
            body.contains("crypto_arb_market_snapshot_served_stale_total{feed=\"spot_ticks\"} 6")
        );
    }

    fn assert_market_rest_baseline_metrics(body: &str) {
        assert!(body.contains("crypto_arb_market_rest_baseline_guard_keys{scope=\"orderbook\"} 7"));
        assert!(body.contains("crypto_arb_market_rest_baseline_guard_keys{scope=\"snapshot\"} 2"));
        assert!(body.contains("crypto_arb_market_rest_baseline_in_flight{scope=\"orderbook\"} 2"));
        assert!(body.contains("crypto_arb_market_rest_baseline_in_flight{scope=\"snapshot\"} 1"));
        assert!(body
            .contains("crypto_arb_market_rest_baseline_wait_count_total{scope=\"orderbook\"} 11"));
        assert!(
            body.contains("crypto_arb_market_rest_baseline_wait_count_total{scope=\"snapshot\"} 3")
        );
        assert!(
            body.contains("crypto_arb_market_rest_baseline_wait_ms_total{scope=\"orderbook\"} 44")
        );
        assert!(
            body.contains("crypto_arb_market_rest_baseline_wait_ms_total{scope=\"snapshot\"} 12")
        );
        assert!(body.contains(
            "crypto_arb_market_rest_baseline_guard_evicted_total{scope=\"orderbook\"} 5"
        ));
        assert!(body.contains(
            "crypto_arb_market_rest_baseline_guard_oldest_idle_ms{scope=\"orderbook\"} 1500"
        ));
    }

    fn assert_market_health_metrics(body: &str) {
        assert!(body.contains(
            "crypto_arb_market_data_runtime_status{venue=\"bybit\",operation=\"ws_ticker\",source=\"ws_push\",quality=\"rate_limited\"} 1"
        ));
        assert!(body.contains(
            "crypto_arb_market_data_runtime_requested{venue=\"bybit\",operation=\"ws_ticker\",source=\"ws_push\",quality=\"rate_limited\"} 2"
        ));
        assert!(body.contains(
            "crypto_arb_market_data_runtime_rows{venue=\"bybit\",operation=\"ws_ticker\",source=\"ws_push\",quality=\"rate_limited\"} 0"
        ));
        assert!(body.contains(
            "crypto_arb_market_data_runtime_retry_after_ms{venue=\"bybit\",operation=\"ws_ticker\",source=\"ws_push\",quality=\"rate_limited\"} 3000"
        ));
    }

    fn assert_http_metrics(body: &str) {
        assert!(body.contains(
            "crypto_arb_http_requests_total{exchange=\"binance\",method=\"GET\",path=\"/fapi/v1/depth\"} 3"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_weight_total{exchange=\"binance\",method=\"GET\",path=\"/fapi/v1/depth\"} 6"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_outcome_total{exchange=\"binance\",method=\"GET\",path=\"/fapi/v1/depth\",outcome=\"success\",status=\"200\"} 2"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_latency_ms_total{exchange=\"binance\",method=\"GET\",path=\"/fapi/v1/depth\",outcome=\"success\",status=\"200\"} 15"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_last_latency_ms{exchange=\"binance\",method=\"GET\",path=\"/fapi/v1/depth\",outcome=\"success\",status=\"200\"} 9"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_retry_after_ms_total{exchange=\"bybit\",method=\"GET\",path=\"/v5/market/tickers\",outcome=\"rate_limited\",status=\"429\"} 3000"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_last_retry_after_ms{exchange=\"bybit\",method=\"GET\",path=\"/v5/market/tickers\",outcome=\"rate_limited\",status=\"429\"} 3000"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_last_observed_at_ms{exchange=\"bybit\",method=\"GET\",path=\"/v5/market/tickers\",outcome=\"rate_limited\",status=\"429\"} 1700000000000"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_retry_total{exchange=\"bybit\",method=\"GET\",path=\"/v5/market/tickers\",outcome=\"rate_limited\",status=\"429\"} 1"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_latency_ms_bucket{exchange=\"binance\",method=\"GET\",path=\"/fapi/v1/depth\",outcome=\"success\",status=\"200\",le=\"25\"} 2"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_latency_ms_bucket{exchange=\"binance\",method=\"GET\",path=\"/fapi/v1/depth\",outcome=\"success\",status=\"200\",le=\"+Inf\"} 2"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_latency_ms_p95_bucket{exchange=\"binance\",method=\"GET\",path=\"/fapi/v1/depth\",outcome=\"success\",status=\"200\"} 25"
        ));
        assert!(body.contains(
            "crypto_arb_http_request_outcome_total{exchange=\"okx\",method=\"GET\",path=\"/api/v5/market/tickers\",outcome=\"circuit_open\",status=\"none\"} 1"
        ));
    }

    fn assert_host_gate_metrics(body: &str) {
        assert!(body.contains(
            "crypto_arb_http_host_gate_state{exchange=\"bybit\",host=\"api.bybit.com\",state=\"rate_limited\"} 1"
        ));
        assert!(body.contains(
            "crypto_arb_http_host_gate_retry_after_ms{exchange=\"bybit\",host=\"api.bybit.com\",state=\"rate_limited\"} 2000"
        ));
        assert!(body.contains(
            "crypto_arb_http_host_gate_consecutive_failures{exchange=\"okx\",host=\"www.okx.com\"} 2"
        ));
        assert!(body.contains(
            "crypto_arb_http_host_gate_inflight_keys{exchange=\"bybit\",host=\"api.bybit.com\"} 3"
        ));
        assert!(body.contains(
            "crypto_arb_http_host_gate_inflight_active_keys{exchange=\"bybit\",host=\"api.bybit.com\"} 1"
        ));
        assert!(body.contains(
            "crypto_arb_http_host_gate_inflight_pruned_total{exchange=\"bybit\",host=\"api.bybit.com\"} 4"
        ));
        assert!(body.contains(
            "crypto_arb_http_host_gate_inflight_oldest_idle_ms{exchange=\"bybit\",host=\"api.bybit.com\"} 700"
        ));
    }

    fn assert_rate_limiter_metrics(body: &str) {
        assert!(body.contains(
            "crypto_arb_rate_limiter_qps{limiter=\"binance\",parent=\"global-binance\"} 20"
        ));
        assert!(body.contains(
            "crypto_arb_rate_limiter_wait_total{limiter=\"binance\",parent=\"global-binance\"} 3"
        ));
        assert!(body.contains(
            "crypto_arb_rate_limiter_wait_ms_total{limiter=\"binance\",parent=\"global-binance\"} 250"
        ));
        assert!(body.contains(
            "crypto_arb_rate_limiter_try_acquire_rejected_total{limiter=\"binance\",parent=\"global-binance\"} 1"
        ));
        assert!(body.contains(
            "crypto_arb_rate_limiter_last_wait_ms{limiter=\"binance\",parent=\"global-binance\"} 80"
        ));
    }

    fn assert_label_metrics(body: &str) {
        assert!(body.contains("crypto_arb_funding_exchange_last_count{exchange=\"binance\"} 691"));
        assert!(body.contains("crypto_arb_funding_exchange_last_count{exchange=\"okx\"} 318"));
        assert!(body.contains(
            "crypto_arb_funding_exchange_last_count{exchange=\"weird\\\"exchange\\\\name\\nx\"} 1"
        ));
        assert!(body.contains("crypto_arb_ws_channels 1"));
        assert!(
            body.contains("crypto_arb_ws_subscribers{channel=\"weird\\\"channel\\\\name\\nx\"} 1")
        );
    }
}
