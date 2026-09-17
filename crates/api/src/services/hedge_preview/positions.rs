use super::pricing::PreviewCosts;
use super::*;

mod evidence;

pub(crate) use evidence::positions_evidence_guard;

pub(crate) struct PreviewMetrics {
    pub(crate) current_account_liq_distance_pct: Option<f64>,
    pub(crate) after_hedge_liq_distance_pct: Option<f64>,
    pub(crate) positions_evidence: HedgePreviewPositionsEvidence,
    pub(crate) position_rows: Vec<PositionInfo>,
    pub(crate) used_capital_usd: f64,
    pub(crate) max_loss_usd: f64,
}

pub(super) async fn append_positions_evidence_guard(
    state: &AppState,
    ticket: &mut HedgeTicket,
    mode: ExecutionMode,
    funding_usd: f64,
    costs: &PreviewCosts,
    intents: &[&shared_types::OrderIntent],
) -> PreviewMetrics {
    let venues = intents
        .iter()
        .map(|intent| intent.exchange.clone())
        .collect::<Vec<_>>();
    let envelope = crate::services::account_positions::envelope_for_venues(state, &venues).await;
    let metrics = preview_metrics_from_positions(&envelope, funding_usd, costs);
    crate::services::hedge_ticket::append_guard(
        ticket,
        positions_evidence_guard(
            mode,
            &metrics.positions_evidence,
            &metrics.position_rows,
            intents,
        ),
    );
    metrics
}

fn preview_metrics_from_positions(
    envelope: &VenuePositionEnvelope,
    funding_usd: f64,
    costs: &PreviewCosts,
) -> PreviewMetrics {
    let used_capital_usd = used_capital_usd(&envelope.rows);
    let current_account_liq_distance_pct = current_liquidation_distance_pct(&envelope.rows);
    let max_loss_usd = costs.total_usd() + funding_usd.min(0.0).abs();
    PreviewMetrics {
        current_account_liq_distance_pct,
        // No venue-verified hypothetical liquidation model is wired here yet.
        after_hedge_liq_distance_pct: None,
        positions_evidence: preview_positions_evidence(envelope, current_account_liq_distance_pct),
        position_rows: envelope.rows.clone(),
        used_capital_usd,
        max_loss_usd,
    }
}

#[cfg(test)]
pub(crate) fn preview_metrics_from_position_envelope(
    envelope: &VenuePositionEnvelope,
    funding_usd: f64,
    costs: &PreviewCosts,
) -> PreviewMetrics {
    preview_metrics_from_positions(envelope, funding_usd, costs)
}

pub(crate) fn used_capital_usd(positions: &[PositionInfo]) -> f64 {
    positions.iter().map(|row| row.margin.max(0.0)).sum()
}

pub(crate) fn current_liquidation_distance_pct(positions: &[PositionInfo]) -> Option<f64> {
    positions
        .iter()
        .filter_map(|row| row.liquidation_distance_pct)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .reduce(f64::min)
}

fn preview_positions_evidence(
    envelope: &VenuePositionEnvelope,
    current_account_liq_distance_pct: Option<f64>,
) -> HedgePreviewPositionsEvidence {
    HedgePreviewPositionsEvidence {
        status: envelope.status,
        source: envelope.source.clone(),
        observed_at_ms: envelope.observed_at_ms,
        row_count: envelope.row_count,
        current_account_liq_distance_pct,
        problems: envelope.problems.clone(),
        operation_health: envelope.operation_health.clone(),
        field_quality: envelope.field_quality.clone(),
        row_health: envelope.row_health.clone(),
        account_bindings: envelope.account_bindings.clone(),
        request_id: positions_request_id(envelope),
        retry_after_ms: positions_retry_after_ms(envelope),
    }
}

fn positions_retry_after_ms(evidence: &VenuePositionEnvelope) -> Option<u64> {
    evidence
        .problems
        .iter()
        .filter_map(|problem| problem.retry_after_ms)
        .chain(
            evidence
                .operation_health
                .iter()
                .filter_map(|row| row.retry_after_ms),
        )
        .max()
}

fn positions_request_id(evidence: &VenuePositionEnvelope) -> Option<String> {
    evidence
        .problems
        .iter()
        .find_map(|problem| problem.request_id.clone())
        .or_else(common::request_id::current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        AccountBindingEvidence, AccountBindingStatus, AccountDataHealth, AccountFieldSubject,
        ListStatus, VenuePositionEnvelope,
    };

    #[test]
    fn preview_and_ticket_evidence_keep_position_row_health_and_account_bindings(
    ) -> Result<(), &'static str> {
        let row_health = AccountDataHealth::new(
            AccountFieldSubject::position("binance", "BTCUSDT", "long"),
            "account_position_runtime",
            42,
        );
        let binding = AccountBindingEvidence {
            venue: "binance".into(),
            account_scope: Some("usds_m_futures".into()),
            status: AccountBindingStatus::Verified,
            source: "credential_probe:account_mode_read".into(),
            checked_at_ms: Some(41),
            freshness_ms: Some(1),
            credential_fingerprint: Some("hmac-sha256:test".into()),
            problem: None,
        };
        let envelope = VenuePositionEnvelope::new(
            Vec::new(),
            ListStatus::Fresh,
            "account_position_runtime",
            42,
            Vec::new(),
            Vec::new(),
        )
        .with_row_health(vec![row_health.clone()])
        .with_account_bindings(vec![binding.clone()]);

        let metrics =
            preview_metrics_from_position_envelope(&envelope, 0.0, &PreviewCosts::default());
        let guard =
            positions_evidence_guard(ExecutionMode::Live, &metrics.positions_evidence, &[], &[]);
        let outcome = guard
            .preflight_outcome
            .ok_or("positions guard must retain preflight evidence")?;

        assert_eq!(metrics.positions_evidence.row_health, vec![row_health]);
        assert_eq!(metrics.positions_evidence.account_bindings, vec![binding]);
        assert_eq!(outcome.row_health, metrics.positions_evidence.row_health);
        assert_eq!(outcome.scope.venues, vec!["binance"]);
        assert_eq!(outcome.scope.symbols, vec!["BTCUSDT"]);
        Ok(())
    }
}
