use super::*;

pub(in crate::services::opportunity) fn p0_count_breakdown(
    rows: &[ArbitrageOpportunityDto],
) -> OpportunityCountBreakdown {
    count_breakdown(rows, CountMode::MainP0)
}

pub(super) fn registry_count_breakdown(
    rows: &[ArbitrageOpportunityDto],
) -> OpportunityCountBreakdown {
    count_breakdown(rows, CountMode::Registry)
}

fn count_breakdown(rows: &[ArbitrageOpportunityDto], mode: CountMode) -> OpportunityCountBreakdown {
    let mut out = OpportunityCountBreakdown::default();
    for row in rows {
        let Some(kind) = row.strategy_kind else {
            continue;
        };
        if mode == CountMode::MainP0 && !is_p0_executable_strategy(kind) {
            continue;
        }
        out.total_count += 1;
        *out.strategy_counts.entry(kind).or_insert(0) += 1;
        if is_hedge_preview_ready(row) {
            out.executable_count += 1;
            *out.executable_strategy_counts.entry(kind).or_insert(0) += 1;
        }
    }
    out
}

pub(in crate::services::opportunity) fn partial_failures(
    meta: &OpportunityScanMeta,
) -> Vec<ApiProblem> {
    if let Some(status) = meta.market_data_status.as_ref() {
        let failures = status
            .rows
            .iter()
            .filter(|row| is_opportunity_scan_market_problem(row))
            .map(status_row_problem)
            .collect::<Vec<_>>();
        if !failures.is_empty() {
            return failures;
        }
    }
    if meta.market_data_problem_count == 0 {
        return Vec::new();
    }
    let venues = if meta.degraded_venues.is_empty() {
        "unknown venues".to_owned()
    } else {
        meta.degraded_venues.join(",")
    };
    vec![ApiProblem::new(
        codes::OPPORTUNITY_MARKET_DATA_DEGRADED,
        format!(
            "{} market-data source(s) degraded: {venues}",
            meta.market_data_problem_count
        ),
    )
    .with_source("market-data-cache")]
}

pub(super) fn scope_market_data_status(
    meta: &mut OpportunityScanMeta,
    strategy_kinds: &[StrategyKind],
) {
    if strategy_kinds.is_empty() {
        return;
    }
    let Some(status) = meta.market_data_status.as_mut() else {
        return;
    };
    status.rows.retain(|row| {
        !scan_required_market_status(row)
            || strategy_kinds
                .iter()
                .any(|kind| strategy_requires_market_operation(*kind, row.operation))
    });
    let degraded = status
        .rows
        .iter()
        .filter(|row| scan_market_status_is_problem(row))
        .collect::<Vec<_>>();
    meta.market_data_problem_count = degraded.len();
    meta.degraded_venues = degraded
        .into_iter()
        .map(|row| format!("{}:{}", row.venue, row.operation.as_str()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
}

fn strategy_requires_market_operation(
    strategy: StrategyKind,
    operation: shared_types::MarketDataSnapshotOperation,
) -> bool {
    use shared_types::MarketDataSnapshotOperation::{FundingRates, PerpTickers, SpotTicks};

    match strategy {
        StrategyKind::PerpCross | StrategyKind::PerpPriceSpread => {
            matches!(operation, FundingRates | PerpTickers)
        }
        StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp => {
            matches!(operation, FundingRates | PerpTickers | SpotTicks)
        }
        StrategyKind::SpotCross => operation == SpotTicks,
        _ => matches!(operation, FundingRates | PerpTickers | SpotTicks),
    }
}

fn is_opportunity_scan_market_problem(row: &MarketDataSnapshotStatusRow) -> bool {
    scan_market_status_is_problem(row)
}

fn status_row_problem(row: &MarketDataSnapshotStatusRow) -> ApiProblem {
    row.health.problem.clone().unwrap_or_else(|| {
        ApiProblem::new(
            codes::OPPORTUNITY_MARKET_DATA_DEGRADED,
            format!(
                "{} {} market data degraded: {:?}",
                row.venue,
                row.operation.as_str(),
                row.health.quality
            ),
        )
        .with_retry_after_ms(row.health.retry_after_ms)
        .with_source("market-data-cache")
    })
}

pub(super) fn envelope_status(
    status: OpportunityEnvelopeStatus,
    no_partial_failures: bool,
) -> OpportunityEnvelopeStatus {
    if status == OpportunityEnvelopeStatus::Fresh && !no_partial_failures {
        OpportunityEnvelopeStatus::Degraded
    } else {
        status
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CountMode {
    MainP0,
    Registry,
}
