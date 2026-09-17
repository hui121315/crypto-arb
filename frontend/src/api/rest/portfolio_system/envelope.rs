//! portfolio/review REST 的快照信封收敛、降级 problem 派生与分页/查询路径构造。
//! 客户端方法见父模块 `portfolio_system.rs`。

use super::super::*;

const REVIEW_PAGE_LIMIT: usize = 50;

pub(super) fn portfolio_snapshot_from_envelope(
    mut envelope: shared_types::PortfolioSnapshotEnvelope,
) -> Result<shared_types::PortfolioSnapshot, ApiError> {
    envelope
        .snapshot
        .take()
        .ok_or_else(|| ApiError::from_problem(portfolio_envelope_problem(&envelope)))
}

pub(crate) fn portfolio_envelope_degraded_problem(
    envelope: &shared_types::PortfolioSnapshotEnvelope,
) -> Option<shared_types::ApiProblem> {
    if envelope.status == shared_types::PortfolioSnapshotStatus::Fresh {
        None
    } else {
        Some(portfolio_envelope_problem(envelope))
    }
}

pub(crate) fn portfolio_envelope_problem(
    envelope: &shared_types::PortfolioSnapshotEnvelope,
) -> shared_types::ApiProblem {
    envelope
        .problem
        .clone()
        .or_else(|| envelope.problems.first().cloned())
        .unwrap_or_else(|| {
            let (code, message) = portfolio_envelope_fallback(&envelope.status);
            shared_types::ApiProblem::new(code, message)
                .with_retry_after_ms(envelope.retry_after_ms)
                .with_source(envelope.source.clone())
        })
}

fn portfolio_envelope_fallback(
    status: &shared_types::PortfolioSnapshotStatus,
) -> (&'static str, &'static str) {
    match status {
        shared_types::PortfolioSnapshotStatus::Degraded => (
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_DEGRADED,
            "portfolio snapshot degraded",
        ),
        shared_types::PortfolioSnapshotStatus::Stale => (
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_STALE,
            "portfolio snapshot stale",
        ),
        shared_types::PortfolioSnapshotStatus::Fresh => (
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_DEGRADED,
            "portfolio snapshot has runtime problems",
        ),
        shared_types::PortfolioSnapshotStatus::Error => (
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE,
            "portfolio snapshot unavailable",
        ),
    }
}

pub(super) fn review_page_path(path: &str, days: u32, cursor: Option<&str>) -> String {
    let cursor = cursor
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("&cursor={}", encode_query_component(value)))
        .unwrap_or_default();
    format!("{path}?days={days}&limit={REVIEW_PAGE_LIMIT}{cursor}")
}

pub(super) fn venue_operation_health_path(venue: &str) -> String {
    let venue = encode_query_component(venue);
    format!("/api/system/venue-operation-health?venue={venue}")
}

pub(super) fn spot_ticks_path(query: &shared_types::SpotTicksQuery) -> String {
    let mut params = [
        ("symbol", query.symbol.as_deref()),
        ("base", query.base.as_deref()),
        ("quote", query.quote.as_deref()),
        ("venue", query.venue.as_deref()),
        ("cursor", query.cursor.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("{name}={}", encode_query_component(value)))
    })
    .collect::<Vec<_>>();
    if let Some(limit) = query.limit {
        params.push(format!("limit={limit}"));
    }
    if query.fresh_only {
        params.push("fresh_only=true".into());
    }
    if params.is_empty() {
        "/api/v1/spot/ticks".into()
    } else {
        format!("/api/v1/spot/ticks?{}", params.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_operation_health_path_encodes_venue_query() {
        assert_eq!(
            venue_operation_health_path("hyperliquid:xyz"),
            "/api/system/venue-operation-health?venue=hyperliquid%3Axyz"
        );
        assert_eq!(
            venue_operation_health_path("Gate USDT"),
            "/api/system/venue-operation-health?venue=Gate%20USDT"
        );
    }

    #[test]
    fn spot_ticks_path_preserves_the_shared_filter_contract() {
        let query = shared_types::SpotTicksQuery {
            symbol: Some("BTCUSDT".into()),
            base: Some("BTC".into()),
            quote: Some("USDT".into()),
            venue: Some("binance".into()),
            limit: Some(64),
            cursor: Some("32".into()),
            fresh_only: true,
        };

        let path = spot_ticks_path(&query);

        assert_eq!(
            path,
            "/api/v1/spot/ticks?symbol=BTCUSDT&base=BTC&quote=USDT&venue=binance&cursor=32&limit=64&fresh_only=true"
        );
    }

    #[test]
    fn portfolio_envelope_without_snapshot_becomes_api_error() {
        let problem = shared_types::ApiProblem::new(
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE,
            "portfolio snapshot unavailable",
        )
        .with_retry_after_ms(Some(2_000))
        .with_source("portfolio_lifecycle");
        let envelope = shared_types::PortfolioSnapshotEnvelope {
            status: shared_types::PortfolioSnapshotStatus::Error,
            source: "portfolio_lifecycle".to_owned(),
            observed_at_ms: 1,
            snapshot: None,
            problem: Some(problem),
            problems: Vec::new(),
            operation_health: Vec::new(),
            retry_after_ms: Some(2_000),
        };

        let result = portfolio_snapshot_from_envelope(envelope);
        assert!(result.is_err(), "missing snapshot should be an api error");
        let Err(error) = result else {
            return;
        };

        assert_eq!(
            error.problem.code,
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE
        );
        assert_eq!(error.problem.retry_after_ms, Some(2_000));
        assert_eq!(error.problem.source.as_deref(), Some("portfolio_lifecycle"));
    }

    #[test]
    fn portfolio_degraded_envelope_with_snapshot_keeps_stale_problem() {
        let envelope = shared_types::PortfolioSnapshotEnvelope {
            status: shared_types::PortfolioSnapshotStatus::Degraded,
            source: "portfolio_lifecycle".to_owned(),
            observed_at_ms: 1,
            snapshot: None,
            problem: None,
            problems: Vec::new(),
            operation_health: Vec::new(),
            retry_after_ms: Some(2_000),
        };

        let problem = portfolio_envelope_degraded_problem(&envelope);
        assert!(
            problem.is_some(),
            "degraded envelope should produce a visible problem"
        );
        let problem =
            problem.unwrap_or_else(|| shared_types::ApiProblem::new("TEST_MISSING", "missing"));

        assert_eq!(
            problem.code,
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_DEGRADED
        );
        assert_eq!(problem.retry_after_ms, Some(2_000));
        assert_eq!(problem.source.as_deref(), Some("portfolio_lifecycle"));
    }

    #[test]
    fn fresh_portfolio_envelope_keeps_advisory_problems_out_of_load_state() {
        let advisory = shared_types::ApiProblem::new(
            shared_types::problem::codes::NAV_STORAGE_UNAVAILABLE,
            "NAV sample skipped: account-level equity coverage incomplete",
        )
        .with_source("portfolio_nav_store");
        let envelope = shared_types::PortfolioSnapshotEnvelope {
            status: shared_types::PortfolioSnapshotStatus::Fresh,
            source: "portfolio_lifecycle".to_owned(),
            observed_at_ms: 1,
            snapshot: None,
            problem: Some(advisory.clone()),
            problems: vec![advisory],
            operation_health: Vec::new(),
            retry_after_ms: None,
        };

        assert!(portfolio_envelope_degraded_problem(&envelope).is_none());
    }
}
