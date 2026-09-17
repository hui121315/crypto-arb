//! Per-venue partial-failure envelope DTOs.
//!
//! 运行态 fanout（system health、portfolio snapshot、account state、market
//! data）需要 per-venue 信封：任一 venue parse/读取失败只标记**该 venue**
//! degraded，而不是整体 502；但同时必须 fail-closed——当**所有** venue 都失败
//! 时，整体状态必须显式回到 `Failed`，绝不把“全军覆没”伪装成 `Ok`。

use crate::problem::ApiProblem;
use serde::{Deserialize, Serialize};

/// 单个 venue 的 fanout 结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueOutcomeStatus {
    /// 该 venue 正常返回。
    Ok,
    /// 该 venue 部分可用/降级（带 problem 说明）。
    Degraded,
    /// 该 venue 完全失败（带 problem 说明）。
    Failed,
}

/// 单个 venue 的 fanout 结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueOutcome {
    pub venue: String,
    pub status: VenueOutcomeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl VenueOutcome {
    pub fn ok(venue: impl Into<String>) -> Self {
        Self {
            venue: venue.into(),
            status: VenueOutcomeStatus::Ok,
            problem: None,
        }
    }

    pub fn degraded(venue: impl Into<String>, problem: ApiProblem) -> Self {
        Self {
            venue: venue.into(),
            status: VenueOutcomeStatus::Degraded,
            problem: Some(problem),
        }
    }

    pub fn failed(venue: impl Into<String>, problem: ApiProblem) -> Self {
        Self {
            venue: venue.into(),
            status: VenueOutcomeStatus::Failed,
            problem: Some(problem),
        }
    }
}

/// 聚合后的整体信封状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialEnvelopeStatus {
    /// 无任何 venue（空请求）——不可当作可用数据。
    Empty,
    /// 全部 venue Ok。
    Ok,
    /// 部分 venue 降级/失败，但仍有可用数据。
    Degraded,
    /// 所有 venue 均失败——fail-closed，绝不当作 Ok。
    Failed,
}

/// per-venue 部分失败信封。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerVenuePartialEnvelope {
    pub outcomes: Vec<VenueOutcome>,
}

impl PerVenuePartialEnvelope {
    pub fn new(outcomes: Vec<VenueOutcome>) -> Self {
        Self { outcomes }
    }

    fn count(&self, status: VenueOutcomeStatus) -> usize {
        self.outcomes.iter().filter(|o| o.status == status).count()
    }

    /// fail-closed 整体状态推导：
    /// 空集合→`Empty`；全部 Ok→`Ok`；全部 Failed→`Failed`；其余→`Degraded`。
    /// 注意：只要存在至少一个非 Ok 结果就不会是 `Ok`，且全失败绝不降级为
    /// `Degraded`/`Ok`。
    pub fn derive_status(&self) -> PartialEnvelopeStatus {
        if self.outcomes.is_empty() {
            return PartialEnvelopeStatus::Empty;
        }
        let total = self.outcomes.len();
        let ok = self.count(VenueOutcomeStatus::Ok);
        let failed = self.count(VenueOutcomeStatus::Failed);
        if ok == total {
            PartialEnvelopeStatus::Ok
        } else if failed == total {
            PartialEnvelopeStatus::Failed
        } else {
            PartialEnvelopeStatus::Degraded
        }
    }

    /// 是否存在任何可用（Ok 或 Degraded）数据；全失败或空时为 false。
    pub fn has_usable_data(&self) -> bool {
        matches!(
            self.derive_status(),
            PartialEnvelopeStatus::Ok | PartialEnvelopeStatus::Degraded
        )
    }

    fn venues_with(&self, status: VenueOutcomeStatus) -> Vec<String> {
        self.outcomes
            .iter()
            .filter(|o| o.status == status)
            .map(|o| o.venue.clone())
            .collect()
    }

    pub fn degraded_venues(&self) -> Vec<String> {
        self.venues_with(VenueOutcomeStatus::Degraded)
    }

    pub fn failed_venues(&self) -> Vec<String> {
        self.venues_with(VenueOutcomeStatus::Failed)
    }

    pub fn ok_venues(&self) -> Vec<String> {
        self.venues_with(VenueOutcomeStatus::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem() -> ApiProblem {
        ApiProblem::new("VENUE_FANOUT_DEGRADED", "parse error")
    }

    #[test]
    fn empty_envelope_is_empty_and_unusable() {
        let env = PerVenuePartialEnvelope::default();
        assert_eq!(env.derive_status(), PartialEnvelopeStatus::Empty);
        assert!(!env.has_usable_data());
    }

    #[test]
    fn all_ok_is_ok() {
        let env = PerVenuePartialEnvelope::new(vec![
            VenueOutcome::ok("binance"),
            VenueOutcome::ok("okx"),
        ]);
        assert_eq!(env.derive_status(), PartialEnvelopeStatus::Ok);
        assert!(env.has_usable_data());
        assert_eq!(env.ok_venues(), vec!["binance", "okx"]);
    }

    #[test]
    fn one_venue_failure_is_degraded_not_total() {
        let env = PerVenuePartialEnvelope::new(vec![
            VenueOutcome::ok("binance"),
            VenueOutcome::failed("okx", problem()),
        ]);
        assert_eq!(env.derive_status(), PartialEnvelopeStatus::Degraded);
        assert!(env.has_usable_data());
        assert_eq!(env.failed_venues(), vec!["okx"]);
        assert_eq!(env.ok_venues(), vec!["binance"]);
    }

    #[test]
    fn one_venue_degraded_is_degraded() {
        let env = PerVenuePartialEnvelope::new(vec![
            VenueOutcome::ok("binance"),
            VenueOutcome::degraded("gate", problem()),
        ]);
        assert_eq!(env.derive_status(), PartialEnvelopeStatus::Degraded);
        assert_eq!(env.degraded_venues(), vec!["gate"]);
    }

    #[test]
    fn all_failed_is_failed_never_ok() {
        let env = PerVenuePartialEnvelope::new(vec![
            VenueOutcome::failed("binance", problem()),
            VenueOutcome::failed("okx", problem()),
        ]);
        assert_eq!(env.derive_status(), PartialEnvelopeStatus::Failed);
        assert!(!env.has_usable_data());
        assert_eq!(env.failed_venues(), vec!["binance", "okx"]);
    }

    #[test]
    fn mixed_degraded_and_failed_without_ok_is_degraded() {
        // 仍有降级（部分可用）数据，不是全军覆没——保持 Degraded 而非 Failed。
        let env = PerVenuePartialEnvelope::new(vec![
            VenueOutcome::degraded("binance", problem()),
            VenueOutcome::failed("okx", problem()),
        ]);
        assert_eq!(env.derive_status(), PartialEnvelopeStatus::Degraded);
        assert!(env.has_usable_data());
    }
}
