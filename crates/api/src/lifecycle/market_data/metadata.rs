use super::*;

/// PR-DP-04 follow-up: 冷启动 metadata prewarm。
///
/// 通过 `Aggregator::refresh_all_metadata` 并发触发每家 venue 的
/// [`exchange::adapter::ExchangeAdapter::refresh_metadata`]，让 24h TTL 的
/// 元数据缓存（Binance `fundingInfo` / Gate `contracts` 等）在第一轮
/// `run_once` baseline 之前完成首次 REST 拉取，避免首次 hot path 等待
/// metadata REST。失败 / 超时不阻塞行情基线；lifecycle 之后按低频周期重试，
/// 让失败恢复不再依赖首个用户请求触发 hot-path lazy refresh。
pub(super) async fn prewarm_metadata(
    runtime: &MarketDataRuntime,
    source: MarketSource,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let outcomes = runtime.aggregator.refresh_all_metadata().await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let (refreshed, skipped, failed) = record_metadata_outcomes(runtime, outcomes, source);
    log_metadata_summary(refreshed, skipped, failed, elapsed_ms);
    if failed == 0 {
        Ok(())
    } else {
        Err(format!("metadata prewarm partial failure: {failed} failed"))
    }
}

fn log_metadata_summary(refreshed: usize, skipped: usize, failed: usize, elapsed_ms: u64) {
    if failed == 0 {
        log_metadata_success(refreshed, skipped, elapsed_ms);
    } else {
        log_metadata_partial_failure(refreshed, skipped, failed, elapsed_ms);
    }
}

fn log_metadata_success(refreshed: usize, skipped: usize, elapsed_ms: u64) {
    debug!(refreshed, skipped, elapsed_ms, "metadata refresh completed");
}

fn log_metadata_partial_failure(refreshed: usize, skipped: usize, failed: usize, elapsed_ms: u64) {
    info!(
        refreshed,
        skipped, failed, elapsed_ms, "metadata refresh completed with partial failures"
    );
}

pub(super) fn record_metadata_outcomes(
    runtime: &MarketDataRuntime,
    outcomes: Vec<(
        String,
        exchange::error::ExchangeResult<exchange::MetadataRefreshOutcome>,
    )>,
    source: MarketSource,
) -> (usize, usize, usize) {
    let mut refreshed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for (venue, result) in outcomes {
        match record_metadata_outcome(runtime, &venue, result, source) {
            MetadataRecord::Refreshed => refreshed += 1,
            MetadataRecord::Skipped => skipped += 1,
            MetadataRecord::Failed => failed += 1,
        }
    }
    (refreshed, skipped, failed)
}

enum MetadataRecord {
    Refreshed,
    Skipped,
    Failed,
}

fn record_metadata_outcome(
    runtime: &MarketDataRuntime,
    venue: &str,
    result: exchange::error::ExchangeResult<exchange::MetadataRefreshOutcome>,
    source: MarketSource,
) -> MetadataRecord {
    match result {
        Ok(exchange::MetadataRefreshOutcome::Refreshed) => {
            runtime.market_data.record_runtime_success(
                venue,
                MARKET_OP_REST_METADATA,
                source,
                1,
                1,
            );
            MetadataRecord::Refreshed
        }
        Ok(exchange::MetadataRefreshOutcome::NotRequired) => {
            runtime.market_data.record_runtime_unsupported(
                venue,
                MARKET_OP_REST_METADATA,
                source,
                1,
                "adapter has no separate metadata cache to refresh",
            );
            MetadataRecord::Skipped
        }
        Err(error) => {
            runtime.market_data.record_runtime_error(
                venue,
                MARKET_OP_REST_METADATA,
                source,
                1,
                &error,
            );
            warn!(
                venue,
                %error,
                "metadata prewarm failed; venue will lazy-refresh on first hot-path call"
            );
            MetadataRecord::Failed
        }
    }
}
