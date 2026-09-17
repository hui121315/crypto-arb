use super::executed_projection::{
    executed_envelope_from_materialization, materialize_executed_at, ExecutedEnvelopeContext,
};
use super::*;
use shared_types::{ReviewRuntimeSnapshot, StrategyPerformance};

const REVIEW_RUNTIME_SOURCE: &str = "review_projection";
const REVIEW_RUNTIME_REFRESH_MS: u64 = 30_000;

pub(crate) async fn runtime_snapshot_from_trading(
    service: &TradingService,
    hot_close_runs: &[CloseRun],
) -> ReviewRuntimeSnapshot {
    let now_ms = common::time::now_ms();
    let from_ms = min_window_ms(now_ms, 30);
    let realized = realized_ledger_from_trading(service, from_ms, now_ms.saturating_add(1)).await;
    let close_runs = close_runs_for_review(hot_close_runs, &realized.close_runs);
    let snapshot =
        runtime_snapshot_from_parts(&realized.orders, &realized.ledger, &close_runs, now_ms);
    ReviewRuntimeSnapshot {
        executed: with_trading_ledger_storage_health(snapshot.executed, service, now_ms)
            .with_funding_payment_ingest_if_present(service),
        strategy_performance: with_trading_ledger_storage_health(
            snapshot.strategy_performance,
            service,
            now_ms,
        )
        .with_funding_payment_ingest_if_present(service),
        generated_at_ms: now_ms,
    }
}

pub(super) fn runtime_snapshot_from_parts(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    now_ms: i64,
) -> ReviewRuntimeSnapshot {
    let materialized = materialize_executed_at(orders, ledger, close_runs, 30, now_ms);
    let executed = executed_envelope_from_materialization(
        &materialized,
        ExecutedEnvelopeContext {
            ledger,
            days: 30,
            offset: 0,
            limit: REVIEW_DEFAULT_LIMIT,
            now_ms,
            from_ms: min_window_ms(now_ms, 30),
            page_status: ListStatus::Fresh,
            page_problems: Vec::new(),
        },
    );
    let strategy_performance =
        strategy_performance_envelope_from_trades(&materialized.trades, ledger, now_ms);
    ReviewRuntimeSnapshot {
        executed,
        strategy_performance,
        generated_at_ms: now_ms,
    }
}

pub(crate) fn warming_runtime_snapshot(now_ms: i64) -> ReviewRuntimeSnapshot {
    let problem = ApiProblem::new(
        codes::REVIEW_RUNTIME_SNAPSHOT_WARMING,
        "review lifecycle has not published its first runtime snapshot",
    )
    .with_source(REVIEW_RUNTIME_SOURCE)
    .with_retry_after_ms(Some(REVIEW_RUNTIME_REFRESH_MS));
    let mut executed = ReviewEnvelope::new(
        Vec::<ExecutedTrade>::new(),
        now_ms,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::NoLedgerEvents),
        Vec::new(),
    );
    executed.status = ListStatus::Degraded;
    executed.problems.push(problem.clone());
    let mut strategy_performance = ReviewEnvelope::new(
        Vec::<StrategyPerformance>::new(),
        now_ms,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::NoLedgerEvents),
        Vec::new(),
    );
    strategy_performance.status = ListStatus::Degraded;
    strategy_performance.problems.push(problem);
    ReviewRuntimeSnapshot {
        executed,
        strategy_performance,
        generated_at_ms: now_ms,
    }
}
