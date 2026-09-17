use crate::state::load_state::LoadState;
use crate::state::polling::{apply_resource_envelope_state, max_retry_after_ms, ResourceEnvelope};
use shared_types::{
    problem::codes, ApiProblem, OpportunityEnvelopeStatus, OpportunityListEnvelope,
};

pub(crate) fn opportunity_envelope_problem(
    envelope: &OpportunityListEnvelope,
) -> Option<ApiProblem> {
    envelope
        .error
        .clone()
        .or_else(|| envelope.partial_failures.first().cloned())
        .or_else(|| status_problem(envelope))
}

pub(crate) fn opportunity_envelope_should_publish_rows(envelope: &OpportunityListEnvelope) -> bool {
    !envelope.rows.is_empty() || opportunity_envelope_problem(envelope).is_none()
}

pub(crate) fn opportunity_envelope_retry_after_ms(
    envelope: &OpportunityListEnvelope,
) -> Option<u64> {
    max_retry_after_ms(
        envelope.retry_after_ms,
        envelope
            .error
            .iter()
            .chain(envelope.partial_failures.iter()),
    )
}

impl ResourceEnvelope for OpportunityListEnvelope {
    fn resource_problem(&self) -> Option<ApiProblem> {
        opportunity_envelope_problem(self)
    }

    fn resource_retry_after_ms(&self) -> Option<u64> {
        opportunity_envelope_retry_after_ms(self)
    }

    fn should_publish_value(&self) -> bool {
        opportunity_envelope_should_publish_rows(self)
    }
}

pub(crate) fn apply_opportunity_envelope_state(
    state: &mut LoadState<()>,
    envelope: &OpportunityListEnvelope,
    published_rows: bool,
) {
    apply_resource_envelope_state(state, envelope, (), published_rows);
}

fn status_problem(envelope: &OpportunityListEnvelope) -> Option<ApiProblem> {
    match envelope.status {
        OpportunityEnvelopeStatus::Fresh => None,
        OpportunityEnvelopeStatus::Warming => Some(status_api_problem(
            codes::OPPORTUNITY_SNAPSHOT_WARMING,
            "机会快照正在预热",
            envelope,
        )),
        OpportunityEnvelopeStatus::Stale => Some(status_api_problem(
            "OPPORTUNITY_ENVELOPE_STALE",
            "机会快照已陈旧",
            envelope,
        )),
        OpportunityEnvelopeStatus::Degraded => Some(status_api_problem(
            codes::OPPORTUNITY_MARKET_DATA_DEGRADED,
            "机会快照存在部分降级",
            envelope,
        )),
        OpportunityEnvelopeStatus::Error => Some(status_api_problem(
            "OPPORTUNITY_ENVELOPE_ERROR",
            "机会快照读取失败",
            envelope,
        )),
    }
}

fn status_api_problem(
    code: &'static str,
    message: &'static str,
    envelope: &OpportunityListEnvelope,
) -> ApiProblem {
    ApiProblem::new(code, format!("{message} · source {}", envelope.source))
        .with_retry_after_ms(envelope.retry_after_ms)
        .with_source("opportunity-envelope")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{OpportunityEnvelopeScope, OpportunityQueryScopeMeta, OpportunityScanMeta};

    #[test]
    fn degraded_envelope_keeps_state_stale_when_rows_publish() {
        let mut state = LoadState::Loading;
        let envelope = envelope(
            OpportunityEnvelopeStatus::Degraded,
            Some(problem("DEGRADED")),
        );

        apply_opportunity_envelope_state(&mut state, &envelope, true);

        assert!(matches!(state, LoadState::Stale { .. }));
        assert_eq!(
            state.problem().map(|problem| problem.code.as_str()),
            Some("DEGRADED")
        );
    }

    #[test]
    fn empty_error_envelope_without_prior_value_is_error() {
        let mut state = LoadState::Loading;
        let envelope = envelope(OpportunityEnvelopeStatus::Error, Some(problem("UPSTREAM")));

        apply_opportunity_envelope_state(&mut state, &envelope, false);

        assert!(matches!(state, LoadState::Error(_)));
        assert_eq!(
            state.problem().map(|problem| problem.code.as_str()),
            Some("UPSTREAM")
        );
    }

    #[test]
    fn empty_error_envelope_with_prior_value_is_stale() {
        let mut state = LoadState::Ready(());
        let envelope = envelope(OpportunityEnvelopeStatus::Error, Some(problem("UPSTREAM")));

        apply_opportunity_envelope_state(&mut state, &envelope, false);

        assert!(matches!(state, LoadState::Stale { .. }));
        assert_eq!(
            state.problem().map(|problem| problem.code.as_str()),
            Some("UPSTREAM")
        );
    }

    #[test]
    fn empty_fresh_envelope_can_publish_zero_rows() {
        let envelope = envelope(OpportunityEnvelopeStatus::Fresh, None);

        assert!(opportunity_envelope_should_publish_rows(&envelope));
    }

    #[test]
    fn empty_problem_envelope_preserves_prior_rows() {
        let envelope = envelope(OpportunityEnvelopeStatus::Warming, None);

        assert!(!opportunity_envelope_should_publish_rows(&envelope));
        assert_eq!(
            opportunity_envelope_problem(&envelope).map(|problem| problem.code),
            Some(codes::OPPORTUNITY_SNAPSHOT_WARMING.into())
        );
    }

    fn envelope(
        status: OpportunityEnvelopeStatus,
        error: Option<ApiProblem>,
    ) -> OpportunityListEnvelope {
        OpportunityListEnvelope {
            rows: Vec::new(),
            page: Default::default(),
            request_meta: Default::default(),
            scope_meta: OpportunityQueryScopeMeta::default(),
            main_p0_counts: Default::default(),
            registry_counts: Default::default(),
            meta: OpportunityScanMeta::default(),
            status,
            scope: OpportunityEnvelopeScope::MainP0,
            query_key: "test".into(),
            source: "test".into(),
            cached_at: chrono::Utc::now(),
            observed_at_ms: 1,
            freshness_ms: None,
            retry_after_ms: Some(5_000),
            error,
            partial_failures: Vec::new(),
            instrument_coverage_diagnostics: String::new(),
        }
    }

    fn problem(code: &str) -> ApiProblem {
        ApiProblem::new(code, "failed")
    }
}
