use crate::state::AppState;
use automation::CandidateSelection;
use shared_types::{
    venue_names_equal, ApiProblem, AutomatedArbitrageConfig, HedgePreviewRequest,
    HedgePreviewResponse,
};

pub(super) const MAX_PREVIEW_CANDIDATES: usize = 3;
const DECISION_SOURCE: &str = "automated_arbitrage";

pub(super) struct PreviewFailure {
    pub reason: String,
    pub problem: Option<ApiProblem>,
}

pub(super) async fn build_candidate_preview(
    state: &AppState,
    candidate: &CandidateSelection,
    snapshot_id: &str,
    config: &AutomatedArbitrageConfig,
) -> Result<HedgePreviewResponse, PreviewFailure> {
    let preview = crate::services::hedge_preview::build_preview(
        state,
        candidate.opportunity.id.clone(),
        HedgePreviewRequest {
            opportunity_id: candidate.opportunity.id.clone(),
            opportunity_snapshot_id: Some(snapshot_id.to_owned()),
            capital_usd: config.capital_usd,
            leverage: config.leverage,
            long_price: None,
            short_price: None,
            long_notional_usd: None,
            short_notional_usd: None,
            execution_params: None,
        },
    )
    .await
    .map_err(|error| {
        let problem = error.to_api_problem().with_source(DECISION_SOURCE);
        PreviewFailure {
            reason: format!("automatic hedge preview blocked: {}", problem.message),
            problem: Some(problem),
        }
    })?;

    if let Some(reason) = preview_submission_blocker(&preview, snapshot_id, config) {
        return Err(PreviewFailure {
            reason,
            problem: None,
        });
    }
    Ok(preview)
}

pub(super) fn venues_allowed(
    risk: &trading::RiskConfig,
    long_exchange: &str,
    short_exchange: &str,
) -> bool {
    // Venue-native instrument symbols exist only after the preview compiles both intents.
    trading::exchange_allowed(&risk.allowed_exchanges, long_exchange)
        && trading::exchange_allowed(&risk.allowed_exchanges, short_exchange)
}

pub(super) fn candidate_allowed(
    risk: &trading::RiskConfig,
    long_exchange: &str,
    short_exchange: &str,
    canonical_symbol: &str,
) -> bool {
    venues_allowed(risk, long_exchange, short_exchange)
        && !risk.protected_positions.iter().any(|protected| {
            protected
                .canonical_symbol
                .trim()
                .eq_ignore_ascii_case(canonical_symbol.trim())
                && (venue_names_equal(&protected.venue, long_exchange)
                    || venue_names_equal(&protected.venue, short_exchange))
        })
}

fn preview_submission_blocker(
    preview: &HedgePreviewResponse,
    expected_snapshot_id: &str,
    config: &AutomatedArbitrageConfig,
) -> Option<String> {
    preview_snapshot_blocker(&preview.opportunity_snapshot_id, expected_snapshot_id)
        .or_else(|| {
            preview_one_cycle_net_blocker(
                preview
                    .ticket
                    .cost
                    .as_ref()
                    .map(|cost| cost.one_cycle.net_bps),
                config.min_one_cycle_net_bps,
            )
        })
        .or_else(|| {
            preview_depth_blocker(
                preview.ticket.long_leg.depth_usd_5bps,
                preview.ticket.short_leg.depth_usd_5bps,
                config.min_depth_usd,
            )
        })
        .or_else(|| {
            (!crate::services::hedge_ticket::ticket_ready(&preview.ticket)).then(|| {
                let reasons = preview
                    .ticket
                    .blockers
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ");
                if reasons.is_empty() {
                    "automatic preview ticket guards did not pass".to_owned()
                } else {
                    format!("automatic preview ticket blocked: {reasons}")
                }
            })
        })
        .or_else(|| {
            (!preview.long_risk.allowed || !preview.short_risk.allowed).then(|| {
                format!(
                    "automatic preview risk blocked: long={:?}; short={:?}",
                    preview.long_risk.reasons, preview.short_risk.reasons
                )
            })
        })
}

fn preview_snapshot_blocker(actual: &str, expected: &str) -> Option<String> {
    (actual != expected)
        .then(|| format!("机会快照已从 {expected} 更新为 {actual}，本轮取消并重新选择候选"))
}

fn preview_one_cycle_net_blocker(
    actual_net_bps: Option<f64>,
    minimum_net_bps: f64,
) -> Option<String> {
    if !minimum_net_bps.is_finite() || minimum_net_bps < 0.0 {
        return Some("自动化费后单周期净差门槛无效，本轮禁止提交".to_owned());
    }
    let Some(actual_net_bps) = actual_net_bps.filter(|value| value.is_finite()) else {
        return Some(format!(
            "预检未证明费后单周期净差；要求至少 {minimum_net_bps:.4} bps"
        ));
    };
    (actual_net_bps + f64::EPSILON < minimum_net_bps).then(|| {
        format!("预检费后单周期净差 {actual_net_bps:.4} bps 低于配置门槛 {minimum_net_bps:.4} bps")
    })
}

fn preview_depth_blocker(
    long_depth_usd_5bps: Option<f64>,
    short_depth_usd_5bps: Option<f64>,
    minimum_depth_usd: f64,
) -> Option<String> {
    let valid = |value: Option<f64>| value.filter(|depth| depth.is_finite() && *depth >= 0.0);
    let (Some(long), Some(short)) = (valid(long_depth_usd_5bps), valid(short_depth_usd_5bps))
    else {
        return Some(format!(
            "automatic preview did not prove both 5bps depths; required ${minimum_depth_usd:.2}"
        ));
    };
    let available = long.min(short);
    (available + f64::EPSILON < minimum_depth_usd).then(|| {
        format!(
            "automatic preview 5bps depth ${available:.2} is below required ${minimum_depth_usd:.2}"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        candidate_allowed, preview_depth_blocker, preview_one_cycle_net_blocker,
        preview_snapshot_blocker, venues_allowed,
    };
    use shared_types::ProtectedPositionFingerprint;
    use std::collections::BTreeSet;

    #[test]
    fn preview_depth_gate_requires_both_measurements() {
        assert!(preview_depth_blocker(None, Some(100.0), 10.0).is_some());
        assert!(preview_depth_blocker(Some(f64::NAN), Some(100.0), 10.0).is_some());
        assert!(preview_depth_blocker(Some(100.0), Some(9.99), 10.0).is_some());
        assert!(preview_depth_blocker(Some(100.0), Some(10.0), 10.0).is_none());
    }

    #[test]
    fn automatic_preview_must_remain_bound_to_the_selected_snapshot() -> Result<(), &'static str> {
        assert!(preview_snapshot_blocker("snapshot-a", "snapshot-a").is_none());
        let blocker = preview_snapshot_blocker("snapshot-b", "snapshot-a")
            .ok_or("snapshot drift must fail closed")?;
        assert!(blocker.contains("本轮取消并重新选择候选"));
        Ok(())
    }

    #[test]
    fn automatic_preview_rechecks_the_configured_one_cycle_net_edge() {
        assert!(preview_one_cycle_net_blocker(Some(10.0), 10.0).is_none());
        assert!(preview_one_cycle_net_blocker(Some(12.5), 10.0).is_none());
        assert!(preview_one_cycle_net_blocker(Some(9.99), 10.0).is_some());
        assert!(preview_one_cycle_net_blocker(None, 10.0).is_some());
        assert!(preview_one_cycle_net_blocker(Some(f64::NAN), 10.0).is_some());
        assert!(preview_one_cycle_net_blocker(Some(10.0), f64::NAN).is_some());
    }

    #[test]
    fn candidate_prefilter_uses_only_the_available_venue_scope() {
        let risk = trading::RiskConfig {
            allowed_exchanges: BTreeSet::from(["bitget".to_owned(), "bybit".to_owned()]),
            allowed_symbols: BTreeSet::from(["coti".to_owned()]),
            ..trading::RiskConfig::default()
        };

        assert!(venues_allowed(&risk, "bitget", "bybit"));
        assert!(!venues_allowed(&risk, "bitget", "gate"));
    }

    #[test]
    fn candidate_prefilter_excludes_only_the_protected_venue_symbol_leg() {
        let risk = trading::RiskConfig {
            protected_positions: vec![ProtectedPositionFingerprint {
                venue: "binance".to_owned(),
                canonical_symbol: "btc".to_owned(),
                native_symbol: "btcusdt".to_owned(),
                side: "long".to_owned(),
                quantity: 0.232,
                entry_price: 64_456.2,
                position_mode: Some("both".to_owned()),
                opening_identity: "preexisting-btc-long".to_owned(),
                source: "account_position_runtime".to_owned(),
                captured_at_ms: 1,
            }],
            ..trading::RiskConfig::default()
        };

        assert!(!candidate_allowed(&risk, "BINANCE", "bitget", "BTC"));
        assert!(candidate_allowed(&risk, "gate", "bitget", "BTC"));
        assert!(candidate_allowed(&risk, "binance", "bitget", "ETH"));
    }
}
