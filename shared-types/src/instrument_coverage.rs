//! Instrument coverage registry DTOs (PR-FP).
//!
//! `InstrumentCoverageEntry` 汇集“某 canonical 标的在每家交易所的挂牌覆盖证据”，
//! 用于搜索诊断、listing blocker 与跨交易所建腿的事实源。
//!
//! 核心 fail-closed 契约：一条腿只有在 `Listed` + 官方 endpoint 核验 + 完整
//! `InstrumentSpec` + 未过期 + 无 problem 时才算 `is_executable_leg`。跨交易所
//! 套利还要求至少两条可执行腿。

use crate::instruments::InstrumentMetadataSource;
use crate::problem::ApiProblem;
use serde::{Deserialize, Serialize};

/// Browser-facing projection of the registry evidence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentCoverageDiagnostic {
    pub canonical_symbol: String,
    pub executable_count: usize,
    pub venue_count: usize,
    pub constructible: bool,
    pub diagnostics_text: String,
}

/// 单家交易所对某标的的挂牌覆盖状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueListingState {
    /// 官方挂牌且在交易中。
    Listed,
    /// 官方明确未挂牌该标的。
    Unlisted,
    /// 覆盖探测失败（解析/网络/鉴权错误）。
    Failed,
    /// 曾挂牌但证据已过期，需要重新核验。
    Stale,
    /// 该交易所不支持此资产类别/产品。
    Unsupported,
    /// 尚未探测，状态未知。
    Unknown,
}

/// 整体覆盖状态推导结果（fail-closed）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentCoverageStatus {
    /// 无任何 venue 记录。
    Empty,
    /// 至少一家可执行腿。
    Executable,
    /// 有可观测挂牌数据但当前无可执行腿。
    ObservedOnly,
    /// 全部 unlisted/failed/unsupported/unknown，无可用证据。
    Unavailable,
}

/// 单家交易所单个标的的覆盖条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCoverageEntry {
    pub venue: String,
    /// 交易所原生 ticker。
    pub native_symbol: String,
    pub state: VenueListingState,
    pub source: InstrumentMetadataSource,
    /// The exact registry row also passed the shared execution specification gate.
    #[serde(default)]
    pub execution_ready: bool,
    /// 该覆盖证据的核验时间戳（毫秒）。
    pub checked_at_ms: i64,
    /// 过期截止时间戳（毫秒）；到达后证据视为过期。`None` 表示不设过期。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_after_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl VenueCoverageEntry {
    /// 结构校验：venue/native symbol 非空，时间戳为正，过期截止（若给）须为正。
    pub fn is_structurally_valid(&self) -> bool {
        !self.venue.trim().is_empty()
            && !self.native_symbol.trim().is_empty()
            && self.checked_at_ms > 0
            && self.stale_after_ms.map(|v| v > 0).unwrap_or(true)
    }

    /// 证据是否已过期。
    pub fn is_stale(&self, now_ms: i64) -> bool {
        match self.stale_after_ms {
            Some(deadline) => now_ms >= deadline,
            None => false,
        }
    }

    /// fail-closed：该腿当前是否可执行。
    pub fn is_executable_leg(&self, now_ms: i64) -> bool {
        self.is_structurally_valid()
            && self.problem.is_none()
            && self.state == VenueListingState::Listed
            && self.source == InstrumentMetadataSource::OfficialEndpoint
            && self.execution_ready
            && !self.is_stale(now_ms)
    }

    /// 是否仍是可观测的挂牌数据（用于 `ObservedOnly` 判定）。
    fn is_observable(&self) -> bool {
        self.problem.is_none()
            && matches!(
                self.state,
                VenueListingState::Listed | VenueListingState::Stale
            )
    }
}

/// 某 canonical 标的的跨交易所覆盖汇总。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentCoverageEntry {
    pub canonical_symbol: String,
    #[serde(default)]
    pub venues: Vec<VenueCoverageEntry>,
}

impl InstrumentCoverageEntry {
    /// 结构校验：canonical symbol 非空，且每条 venue 条目结构合法。
    pub fn is_structurally_valid(&self) -> bool {
        !self.canonical_symbol.trim().is_empty()
            && self
                .venues
                .iter()
                .all(VenueCoverageEntry::is_structurally_valid)
    }

    /// 当前可执行的腿。
    pub fn executable_venues(&self, now_ms: i64) -> Vec<&VenueCoverageEntry> {
        self.venues
            .iter()
            .filter(|v| v.is_executable_leg(now_ms))
            .collect()
    }

    /// fail-closed 整体覆盖状态推导。
    pub fn coverage_status(&self, now_ms: i64) -> InstrumentCoverageStatus {
        if self.venues.is_empty() {
            return InstrumentCoverageStatus::Empty;
        }
        if self.venues.iter().any(|v| v.is_executable_leg(now_ms)) {
            return InstrumentCoverageStatus::Executable;
        }
        if self.venues.iter().any(VenueCoverageEntry::is_observable) {
            return InstrumentCoverageStatus::ObservedOnly;
        }
        InstrumentCoverageStatus::Unavailable
    }

    /// 跨交易所套利至少需要两条可执行腿。
    pub fn is_arbitrage_constructible(&self, now_ms: i64) -> bool {
        self.is_structurally_valid() && self.executable_venues(now_ms).len() >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn entry(
        venue: &str,
        state: VenueListingState,
        source: InstrumentMetadataSource,
    ) -> VenueCoverageEntry {
        VenueCoverageEntry {
            venue: venue.to_owned(),
            native_symbol: "BTCUSDT".to_owned(),
            state,
            source,
            execution_ready: true,
            checked_at_ms: NOW - 1,
            stale_after_ms: Some(NOW + 1000),
            problem: None,
        }
    }

    #[test]
    fn listed_official_fresh_is_executable_leg() {
        let e = entry(
            "binance",
            VenueListingState::Listed,
            InstrumentMetadataSource::OfficialEndpoint,
        );
        assert!(e.is_executable_leg(NOW));
    }

    #[test]
    fn cached_source_not_executable() {
        let e = entry(
            "binance",
            VenueListingState::Listed,
            InstrumentMetadataSource::CachedSnapshot,
        );
        assert!(!e.is_executable_leg(NOW));
    }

    #[test]
    fn listed_official_without_execution_spec_is_observed_only() {
        let mut e = entry(
            "binance",
            VenueListingState::Listed,
            InstrumentMetadataSource::OfficialEndpoint,
        );
        e.execution_ready = false;
        assert!(!e.is_executable_leg(NOW));

        let coverage = InstrumentCoverageEntry {
            canonical_symbol: "BTC-PERP".to_owned(),
            venues: vec![e],
        };
        assert_eq!(
            coverage.coverage_status(NOW),
            InstrumentCoverageStatus::ObservedOnly
        );
    }

    #[test]
    fn stale_evidence_not_executable() {
        let mut e = entry(
            "binance",
            VenueListingState::Listed,
            InstrumentMetadataSource::OfficialEndpoint,
        );
        e.stale_after_ms = Some(NOW - 1);
        assert!(e.is_stale(NOW));
        assert!(!e.is_executable_leg(NOW));
    }

    #[test]
    fn problem_blocks_executable() {
        let mut e = entry(
            "binance",
            VenueListingState::Listed,
            InstrumentMetadataSource::OfficialEndpoint,
        );
        e.problem = Some(ApiProblem::new("COVERAGE_FAIL", "probe failed"));
        assert!(!e.is_executable_leg(NOW));
    }

    #[test]
    fn unlisted_failed_unsupported_not_executable() {
        for st in [
            VenueListingState::Unlisted,
            VenueListingState::Failed,
            VenueListingState::Unsupported,
            VenueListingState::Unknown,
        ] {
            let e = entry("binance", st, InstrumentMetadataSource::OfficialEndpoint);
            assert!(!e.is_executable_leg(NOW), "{st:?} must not be executable");
        }
    }

    #[test]
    fn empty_coverage_is_empty_and_not_arbitrage() {
        let c = InstrumentCoverageEntry {
            canonical_symbol: "BTC-PERP".to_owned(),
            venues: vec![],
        };
        assert_eq!(c.coverage_status(NOW), InstrumentCoverageStatus::Empty);
        assert!(!c.is_arbitrage_constructible(NOW));
    }

    #[test]
    fn single_executable_is_executable_status_but_not_arbitrage() {
        let c = InstrumentCoverageEntry {
            canonical_symbol: "BTC-PERP".to_owned(),
            venues: vec![entry(
                "binance",
                VenueListingState::Listed,
                InstrumentMetadataSource::OfficialEndpoint,
            )],
        };
        assert_eq!(c.coverage_status(NOW), InstrumentCoverageStatus::Executable);
        assert!(!c.is_arbitrage_constructible(NOW));
    }

    #[test]
    fn two_executable_legs_is_arbitrage_constructible() {
        let c = InstrumentCoverageEntry {
            canonical_symbol: "BTC-PERP".to_owned(),
            venues: vec![
                entry(
                    "binance",
                    VenueListingState::Listed,
                    InstrumentMetadataSource::OfficialEndpoint,
                ),
                entry(
                    "okx",
                    VenueListingState::Listed,
                    InstrumentMetadataSource::OfficialEndpoint,
                ),
            ],
        };
        assert_eq!(c.executable_venues(NOW).len(), 2);
        assert!(c.is_arbitrage_constructible(NOW));
    }

    #[test]
    fn listed_but_cached_is_observed_only() {
        let c = InstrumentCoverageEntry {
            canonical_symbol: "BTC-PERP".to_owned(),
            venues: vec![entry(
                "binance",
                VenueListingState::Listed,
                InstrumentMetadataSource::CachedSnapshot,
            )],
        };
        assert_eq!(
            c.coverage_status(NOW),
            InstrumentCoverageStatus::ObservedOnly
        );
        assert!(!c.is_arbitrage_constructible(NOW));
    }

    #[test]
    fn all_unlisted_failed_is_unavailable() {
        let c = InstrumentCoverageEntry {
            canonical_symbol: "BTC-PERP".to_owned(),
            venues: vec![
                entry(
                    "binance",
                    VenueListingState::Unlisted,
                    InstrumentMetadataSource::OfficialEndpoint,
                ),
                entry(
                    "okx",
                    VenueListingState::Failed,
                    InstrumentMetadataSource::OfficialEndpoint,
                ),
            ],
        };
        assert_eq!(
            c.coverage_status(NOW),
            InstrumentCoverageStatus::Unavailable
        );
    }

    #[test]
    fn empty_canonical_is_invalid() {
        let c = InstrumentCoverageEntry {
            canonical_symbol: "  ".to_owned(),
            venues: vec![entry(
                "binance",
                VenueListingState::Listed,
                InstrumentMetadataSource::OfficialEndpoint,
            )],
        };
        assert!(!c.is_structurally_valid());
        assert!(!c.is_arbitrage_constructible(NOW));
    }
}
