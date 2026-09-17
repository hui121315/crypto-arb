//! Trading fee snapshots and execution cost breakdown DTOs.

use crate::{funding::FundingHistoryEvidence, hedge::HedgeLegRole, problem::ApiProblem};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeProduct {
    Spot,
    Perp,
    Margin,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeFeeSource {
    AccountApi,
    OfficialSchedule,
    Manual,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeScheduleEvidence {
    pub evidence_id: String,
    pub source_name: String,
    pub source_url: String,
    pub checked_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// Compatibility name retained for existing API consumers.
pub type TradeFeeEvidence = FeeScheduleEvidence;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeScheduleRegistryResponse {
    #[serde(default)]
    pub schema: FeeScheduleRegistrySchema,
    pub venues: Vec<FeeScheduleVenue>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeScheduleRegistrySchema {
    pub version: String,
    pub fingerprint: String,
}

impl FeeScheduleRegistrySchema {
    pub fn matches(&self, version: &str, fingerprint: &str) -> bool {
        self.version == version && self.fingerprint == fingerprint
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Serialize, Deserialize))]
#[cfg_attr(target_arch = "wasm32", derive(Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum YieldBasis {
    NativeSettlement,
    WindowNormalized,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Serialize, Deserialize))]
#[cfg_attr(target_arch = "wasm32", derive(Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct FundingWindowMismatchEvidence {
    pub yield_basis: YieldBasis,
    pub buffer_bps: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_next_settlement_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_next_settlement_ms: Option<i64>,
}

impl FundingWindowMismatchEvidence {
    pub fn is_traceable(&self) -> bool {
        if matches!(self.yield_basis, YieldBasis::Unknown)
            || !self.buffer_bps.is_finite()
            || self.buffer_bps < 0.0
        {
            return false;
        }
        if self.buffer_bps == 0.0 {
            return true;
        }
        matches!(
            (self.long_next_settlement_ms, self.short_next_settlement_ms),
            (Some(long), Some(short)) if long > 0 && short > 0 && long != short
        )
    }
}

impl YieldBasis {
    pub fn label(self) -> &'static str {
        match self {
            Self::NativeSettlement => "native_settlement",
            Self::WindowNormalized => "window_normalized",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeScheduleVenue {
    pub venue: String,
    pub schedules: Vec<FeeScheduleRegistryRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeScheduleRegistryRow {
    pub product: FeeProduct,
    pub maker_fee_bps: f64,
    pub taker_fee_bps: f64,
    pub evidence: TradeFeeEvidence,
    pub fixture_id: String,
    pub fixture_symbol: String,
    pub snapshot_ttl_ms: i64,
}

impl FeeScheduleEvidence {
    pub fn is_valid(&self) -> bool {
        non_empty(&self.evidence_id)
            && non_empty(&self.source_name)
            && self.source_url.starts_with("https://")
            && self.checked_at_ms > 0
            && self
                .problem
                .as_ref()
                .is_none_or(|problem| problem.trim().is_empty())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfitabilityEvidenceStatus {
    Verified,
    Partial,
    Stale,
    #[default]
    Missing,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfitabilityEvidence {
    pub status: ProfitabilityEvidenceStatus,
    pub source: String,
    pub observed_at_ms: i64,
    pub verified_fee_snapshot_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fee_sources: Vec<TradeFeeSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fee_evidence_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_history: Option<FundingHistoryEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl ProfitabilityEvidence {
    pub fn from_fee_snapshots(
        source: impl Into<String>,
        observed_at_ms: i64,
        snapshots: &[&TradeFeeSnapshot],
        funding_history: Option<FundingHistoryEvidence>,
    ) -> Self {
        let verified = snapshots
            .iter()
            .copied()
            .filter(|snapshot| snapshot.is_fresh_verified(observed_at_ms))
            .collect::<Vec<_>>();
        let fee_evidence_ids = verified
            .iter()
            .filter_map(|snapshot| snapshot.evidence.as_ref())
            .filter(|evidence| evidence.is_valid())
            .map(|evidence| evidence.evidence_id.clone())
            .collect();
        let fee_sources = verified.iter().map(|snapshot| snapshot.source).collect();
        let verified_fee_snapshot_count = verified.len();
        let (status, problem) =
            profitability_status(verified_fee_snapshot_count, funding_history.as_ref());
        Self {
            status,
            source: source.into(),
            observed_at_ms,
            verified_fee_snapshot_count,
            fee_sources,
            fee_evidence_ids,
            funding_history,
            problem,
        }
    }

    pub fn is_cost_verified(&self) -> bool {
        self.verified_fee_snapshot_count >= 2
            && self.fee_sources.len() >= 2
            && !matches!(self.status, ProfitabilityEvidenceStatus::Missing)
    }
}

fn profitability_status(
    verified_fee_snapshot_count: usize,
    funding_history: Option<&FundingHistoryEvidence>,
) -> (ProfitabilityEvidenceStatus, Option<ApiProblem>) {
    if verified_fee_snapshot_count < 2 {
        return (
            ProfitabilityEvidenceStatus::Missing,
            Some(ApiProblem::new(
                crate::problem::codes::PROFITABILITY_FEE_EVIDENCE_MISSING,
                "profitability requires two fresh verified fee snapshots",
            )),
        );
    }
    match funding_history {
        Some(history) if history.is_usable() => (ProfitabilityEvidenceStatus::Verified, None),
        Some(history) if history.sample_health == crate::FundingDiffSampleHealth::Stale => {
            (ProfitabilityEvidenceStatus::Stale, history.problem.clone())
        }
        Some(history) => (
            ProfitabilityEvidenceStatus::Partial,
            history.problem.clone(),
        ),
        None => (
            ProfitabilityEvidenceStatus::Partial,
            Some(ApiProblem::new(
                crate::problem::codes::PROFITABILITY_HISTORY_EVIDENCE_MISSING,
                "profitability funding history evidence is unavailable",
            )),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeFeeSnapshot {
    pub venue: String,
    pub symbol: String,
    pub product: FeeProduct,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub maker_fee_bps: f64,
    pub taker_fee_bps: f64,
    pub open_fee_bps: f64,
    pub close_fee_bps: f64,
    pub source: TradeFeeSource,
    pub fetched_at_ms: i64,
    pub valid_until_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<TradeFeeEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TradeFeeSnapshot {
    pub fn is_fresh_verified(&self, now_ms: i64) -> bool {
        if self.valid_until_ms <= now_ms
            || !finite(self.maker_fee_bps)
            || !finite(self.taker_fee_bps)
            || !non_negative(self.open_fee_bps)
            || !non_negative(self.close_fee_bps)
            || self
                .verification_problem
                .as_ref()
                .is_some_and(|problem| !problem.trim().is_empty())
        {
            return false;
        }
        match self.source {
            TradeFeeSource::AccountApi => self.fetched_at_ms > 0,
            TradeFeeSource::OfficialSchedule => self
                .evidence
                .as_ref()
                .is_some_and(TradeFeeEvidence::is_valid),
            TradeFeeSource::Manual | TradeFeeSource::Unverified => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegCostBreakdown {
    pub role: HedgeLegRole,
    pub venue: String,
    pub symbol: String,
    pub product: FeeProduct,
    pub open_fee_bps: f64,
    pub close_fee_bps: f64,
    pub open_slippage_bps: f64,
    pub close_slippage_bps: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_snapshot: Option<TradeFeeSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundTripCostBreakdown {
    pub long_leg: LegCostBreakdown,
    pub short_leg: LegCostBreakdown,
    pub open_fee_bps: f64,
    pub close_fee_bps: f64,
    pub open_slippage_bps: f64,
    pub close_slippage_bps: f64,
    pub borrow_or_financing_bps: f64,
    pub funding_window_mismatch_buffer_bps: f64,
    pub min_profit_buffer_bps: f64,
    pub total_cost_bps: f64,
    pub one_cycle_net_bps: f64,
    #[serde(default)]
    pub profitability_evidence: ProfitabilityEvidence,
}

fn non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn finite(value: f64) -> bool {
    value.is_finite()
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_snapshot_is_not_fresh_verified() {
        let snapshot = snapshot(TradeFeeSource::Manual, Some(valid_evidence()), None);

        assert!(!snapshot.is_fresh_verified(1_500));
    }

    #[test]
    fn official_schedule_requires_valid_evidence() {
        let without_evidence = snapshot(TradeFeeSource::OfficialSchedule, None, None);
        let with_problem = snapshot(
            TradeFeeSource::OfficialSchedule,
            Some(valid_evidence()),
            Some("fee tier mismatch".to_owned()),
        );
        let verified = snapshot(
            TradeFeeSource::OfficialSchedule,
            Some(valid_evidence()),
            None,
        );

        assert!(!without_evidence.is_fresh_verified(1_500));
        assert!(!with_problem.is_fresh_verified(1_500));
        assert!(verified.is_fresh_verified(1_500));
    }

    #[test]
    fn account_api_snapshot_is_verified_without_static_evidence() {
        let snapshot = snapshot(TradeFeeSource::AccountApi, None, None);

        assert!(snapshot.is_fresh_verified(1_500));
    }

    #[test]
    fn profitability_evidence_requires_two_fresh_fee_snapshots() {
        let verified = snapshot(
            TradeFeeSource::OfficialSchedule,
            Some(valid_evidence()),
            None,
        );
        let evidence = ProfitabilityEvidence::from_fee_snapshots(
            "test",
            1_500,
            &[&verified],
            Some(usable_history()),
        );

        assert_eq!(evidence.status, ProfitabilityEvidenceStatus::Missing);
        assert!(!evidence.is_cost_verified());
        assert_eq!(evidence.verified_fee_snapshot_count, 1);
        assert_eq!(
            evidence
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(crate::problem::codes::PROFITABILITY_FEE_EVIDENCE_MISSING)
        );
    }

    #[test]
    fn profitability_evidence_distinguishes_partial_and_verified_history() {
        let long = snapshot(
            TradeFeeSource::OfficialSchedule,
            Some(valid_evidence()),
            None,
        );
        let short = snapshot(TradeFeeSource::AccountApi, None, None);
        let partial =
            ProfitabilityEvidence::from_fee_snapshots("test", 1_500, &[&long, &short], None);
        let verified = ProfitabilityEvidence::from_fee_snapshots(
            "test",
            1_500,
            &[&long, &short],
            Some(usable_history()),
        );

        assert_eq!(partial.status, ProfitabilityEvidenceStatus::Partial);
        assert!(partial.is_cost_verified());
        assert_eq!(
            partial
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(crate::problem::codes::PROFITABILITY_HISTORY_EVIDENCE_MISSING)
        );
        assert_eq!(verified.status, ProfitabilityEvidenceStatus::Verified);
        assert!(verified.is_cost_verified());
        assert!(verified.problem.is_none());
        assert_eq!(verified.fee_sources.len(), 2);
    }

    #[test]
    fn fee_schedule_registry_response_uses_camel_case_contract() -> serde_json::Result<()> {
        let response = FeeScheduleRegistryResponse {
            schema: FeeScheduleRegistrySchema {
                version: "fee_schedule_registry_v1".into(),
                fingerprint: "464f05fbcd96fcbf".into(),
            },
            venues: vec![FeeScheduleVenue {
                venue: "binance".into(),
                schedules: vec![FeeScheduleRegistryRow {
                    product: FeeProduct::Perp,
                    maker_fee_bps: 2.0,
                    taker_fee_bps: 4.0,
                    evidence: valid_evidence(),
                    fixture_id: "standard-fee:binance:perp:test".into(),
                    fixture_symbol: "BTC-USDT".into(),
                    snapshot_ttl_ms: 86_400_000,
                }],
            }],
        };

        let value = serde_json::to_value(response)?;
        let row = &value["venues"][0]["schedules"][0];
        assert_eq!(row["makerFeeBps"], 2.0);
        assert_eq!(row["fixtureId"], "standard-fee:binance:perp:test");
        assert_eq!(row["snapshotTtlMs"], 86_400_000);
        assert_eq!(row["evidence"]["sourceUrl"], valid_evidence().source_url);
        Ok(())
    }

    fn snapshot(
        source: TradeFeeSource,
        evidence: Option<TradeFeeEvidence>,
        verification_problem: Option<String>,
    ) -> TradeFeeSnapshot {
        TradeFeeSnapshot {
            venue: "binance".into(),
            symbol: "BTCUSDT".into(),
            product: FeeProduct::Perp,
            account_id: None,
            maker_fee_bps: 2.0,
            taker_fee_bps: 4.0,
            open_fee_bps: 4.0,
            close_fee_bps: 4.0,
            source,
            fetched_at_ms: 1_000,
            valid_until_ms: 10_000,
            freshness_ms: Some(500),
            evidence,
            verification_problem,
            note: None,
        }
    }

    fn valid_evidence() -> TradeFeeEvidence {
        TradeFeeEvidence {
            evidence_id: "fee:binance:perp:vip0".into(),
            source_name: "Binance USD-M User Commission Rate".into(),
            source_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/account/rest-api/User-Commission-Rate".into(),
            checked_at_ms: 1_780_185_600_000,
            effective_at_ms: None,
            schedule_version: Some("2026-05-31".into()),
            tier: Some("VIP0".into()),
            scope: Some("USD-M futures standard fee example".into()),
            problem: None,
        }
    }

    fn usable_history() -> FundingHistoryEvidence {
        FundingHistoryEvidence {
            source: "history_store:funding_diff".into(),
            observed_at_ms: 1_500,
            latest_at_ms: 1_400,
            freshness_ms: Some(100),
            sample_count: 9,
            sample_health: crate::FundingDiffSampleHealth::Ok,
            problem: None,
            retry_after_ms: None,
        }
    }

    #[test]
    fn funding_window_mismatch_requires_explicit_basis_and_settlement_trace() {
        let traced = FundingWindowMismatchEvidence {
            yield_basis: YieldBasis::NativeSettlement,
            buffer_bps: 1.5,
            long_next_settlement_ms: Some(1_000),
            short_next_settlement_ms: Some(2_000),
        };
        let untraced = FundingWindowMismatchEvidence {
            short_next_settlement_ms: None,
            ..traced
        };

        assert!(traced.is_traceable());
        assert!(!untraced.is_traceable());
        assert!(!FundingWindowMismatchEvidence {
            yield_basis: YieldBasis::Unknown,
            buffer_bps: 0.0,
            long_next_settlement_ms: None,
            short_next_settlement_ms: None,
        }
        .is_traceable());
        assert!(!FundingWindowMismatchEvidence {
            yield_basis: YieldBasis::NativeSettlement,
            buffer_bps: -1.0,
            long_next_settlement_ms: None,
            short_next_settlement_ms: None,
        }
        .is_traceable());
    }
}
