//! 机会计数元数据：从 REST/流事件收敛后端 total/filtered/executable 与按策略计数，
//! 计数选择器在缺证据时回退可见行、在有证据时严格用后端口径（不伪装）。
//! 新鲜度/覆盖文案见 `freshness.rs`，空态文案见 `empty_label.rs`。

use crate::api::rest::OpportunityListResponse;
use shared_types::{
    ApiProblem, OpportunityEnvelopeScope, OpportunityEnvelopeStatus, OpportunityScanMeta,
    OpportunityStreamEvent, StrategyKind,
};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct OpportunityCountMeta {
    pub(crate) total_count: usize,
    pub(crate) filtered_count: usize,
    pub(crate) executable_count: usize,
    pub(crate) strategy_counts: HashMap<StrategyKind, usize>,
    pub(crate) executable_strategy_counts: HashMap<StrategyKind, usize>,
    pub(crate) scan: OpportunityScanMeta,
    pub(crate) status: OpportunityEnvelopeStatus,
    pub(crate) scope: OpportunityEnvelopeScope,
    pub(crate) query_key: String,
    pub(crate) filter_symbol: Option<String>,
    pub(crate) instrument_coverage_diagnostics: String,
    pub(crate) source: String,
    pub(crate) cached_at: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) observed_at_ms: i64,
    pub(crate) freshness_ms: Option<i64>,
    pub(crate) received_clock: Option<(i64, i64)>,
    pub(crate) rows_retained: bool,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) error: Option<ApiProblem>,
    pub(crate) partial_failures: Vec<ApiProblem>,
}

impl OpportunityCountMeta {
    pub(crate) fn from_list_response(response: &OpportunityListResponse) -> Self {
        Self {
            total_count: response.scope_meta.global_total_count,
            filtered_count: response.scope_meta.filtered_count,
            executable_count: response.main_p0_counts.executable_count,
            strategy_counts: response.main_p0_counts.strategy_counts.clone(),
            executable_strategy_counts: response.main_p0_counts.executable_strategy_counts.clone(),
            scan: response.meta.clone(),
            status: response.status,
            scope: response.scope,
            query_key: response.query_key.clone(),
            filter_symbol: response.request_meta.filter.symbol.clone(),
            instrument_coverage_diagnostics: response.instrument_coverage_diagnostics.clone(),
            source: response.source.clone(),
            cached_at: Some(response.cached_at),
            observed_at_ms: response.observed_at_ms,
            freshness_ms: response.freshness_ms,
            received_clock: Some(super::freshness::snapshot_clock()),
            rows_retained: false,
            retry_after_ms: response.retry_after_ms,
            error: response.error.clone(),
            partial_failures: response.partial_failures.clone(),
        }
    }

    pub(crate) fn from_stream_event(event: &OpportunityStreamEvent) -> Self {
        Self {
            total_count: event.scope_meta.global_total_count,
            filtered_count: event.scope_meta.filtered_count,
            executable_count: event.main_p0_counts.executable_count,
            strategy_counts: event.main_p0_counts.strategy_counts.clone(),
            executable_strategy_counts: event.main_p0_counts.executable_strategy_counts.clone(),
            scan: event.meta.clone(),
            status: event.status,
            scope: event.scope,
            query_key: event.query_key.clone(),
            filter_symbol: None,
            instrument_coverage_diagnostics: String::new(),
            source: event.source.clone(),
            cached_at: Some(event.cached_at),
            observed_at_ms: event.observed_at_ms,
            freshness_ms: event.freshness_ms,
            received_clock: Some(super::freshness::snapshot_clock()),
            rows_retained: false,
            retry_after_ms: event.retry_after_ms,
            error: event.error.clone(),
            partial_failures: event.partial_failures.clone(),
        }
    }

    pub(crate) fn selected_count(&self, kind: Option<StrategyKind>, fallback: usize) -> usize {
        if let Some(kind) = kind {
            return self.kind_count(kind, fallback);
        }
        if self.filtered_count == 0 && !self.has_count_evidence() {
            fallback
        } else {
            self.filtered_count
        }
    }

    pub(crate) fn kind_count(&self, kind: StrategyKind, fallback: usize) -> usize {
        self.strategy_counts
            .get(&kind)
            .copied()
            .unwrap_or_else(|| self.missing_count_fallback(fallback))
    }

    pub(crate) fn selected_executable_count(
        &self,
        kind: Option<StrategyKind>,
        fallback: usize,
    ) -> usize {
        if let Some(kind) = kind {
            return self.executable_kind_count(kind, fallback);
        }
        if self.executable_count == 0 && !self.has_count_evidence() {
            fallback
        } else {
            self.executable_count
        }
    }

    pub(crate) fn executable_kind_count(&self, kind: StrategyKind, fallback: usize) -> usize {
        self.executable_strategy_counts
            .get(&kind)
            .copied()
            .unwrap_or_else(|| self.missing_count_fallback(fallback))
    }

    fn missing_count_fallback(&self, fallback: usize) -> usize {
        if self.has_count_evidence() {
            0
        } else {
            fallback
        }
    }

    pub(in crate::panels::modules::opportunity_counts) fn has_count_evidence(&self) -> bool {
        self.cached_at.is_some()
            || self.observed_at_ms > 0
            || self.freshness_ms.is_some()
            || self.retry_after_ms.is_some()
            || !self.source.is_empty()
            || !self.query_key.is_empty()
            || self.error.is_some()
            || !self.partial_failures.is_empty()
            || self.status != OpportunityEnvelopeStatus::Fresh
            || self.scan.scan_started_at.is_some()
            || self.scan.candidate_count > 0
            || self.scan.emitted_count > 0
            || self.scan.scan_ms > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_when_backend_meta_is_absent() {
        let meta = OpportunityCountMeta::default();

        assert_eq!(meta.selected_count(None, 17), 17);
        assert_eq!(meta.kind_count(StrategyKind::PerpCross, 9), 9);
    }

    #[test]
    fn keeps_backend_counts_separate_from_visible_rows() {
        let mut meta = OpportunityCountMeta {
            total_count: 900,
            filtered_count: 700,
            executable_count: 300,
            strategy_counts: HashMap::new(),
            executable_strategy_counts: HashMap::new(),
            scan: OpportunityScanMeta::default(),
            source: String::new(),
            cached_at: None,
            ..Default::default()
        };
        meta.strategy_counts.insert(StrategyKind::PerpCross, 360);
        meta.executable_strategy_counts
            .insert(StrategyKind::PerpCross, 120);

        assert_eq!(meta.selected_count(None, 100), 700);
        assert_eq!(meta.selected_count(Some(StrategyKind::PerpCross), 100), 360);
        assert_eq!(meta.selected_executable_count(None, 50), 300);
        assert_eq!(
            meta.selected_executable_count(Some(StrategyKind::PerpCross), 50),
            120
        );
    }

    #[test]
    fn explicit_backend_zero_counts_do_not_fall_back_to_visible_rows() {
        let meta = OpportunityCountMeta {
            total_count: 0,
            filtered_count: 0,
            executable_count: 0,
            strategy_counts: HashMap::new(),
            executable_strategy_counts: HashMap::new(),
            source: "snapshot".to_owned(),
            cached_at: Some(chrono::Utc::now()),
            observed_at_ms: 1,
            ..Default::default()
        };

        assert_eq!(meta.selected_count(None, 17), 0);
        assert_eq!(meta.kind_count(StrategyKind::PerpCross, 9), 0);
        assert_eq!(meta.selected_executable_count(None, 5), 0);
        assert_eq!(meta.executable_kind_count(StrategyKind::PerpCross, 3), 0);
    }
}
