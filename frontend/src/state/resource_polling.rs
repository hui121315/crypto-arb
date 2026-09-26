use super::load_state::LoadState;
use shared_types::{ApiProblem, ResourceEnvelope, ResourceStatus};

pub(crate) fn apply_resource_envelope<T>(
    state: &mut LoadState<T>,
    mut envelope: ResourceEnvelope<T>,
) {
    let problem = resource_problem(&envelope);
    match envelope.data.take() {
        Some(value) if envelope.status.has_usable_data() => match problem {
            Some(problem) => *state = LoadState::Stale { value, problem },
            None => *state = LoadState::Ready(value),
        },
        Some(_) | None => state.apply_result(Err(
            problem.unwrap_or_else(|| unavailable_problem(envelope.status, &envelope.source))
        )),
    }
}

pub(crate) fn resource_problem<T>(envelope: &ResourceEnvelope<T>) -> Option<ApiProblem> {
    envelope.problems.first().cloned().or_else(|| {
        (envelope.data.is_none() || envelope.status != ResourceStatus::Ready)
            .then(|| unavailable_problem(envelope.status, &envelope.source))
    })
}

fn unavailable_problem(status: ResourceStatus, source: &str) -> ApiProblem {
    let (code, message) = match status {
        ResourceStatus::Ready => (
            "RESOURCE_DATA_UNAVAILABLE",
            "resource is ready but contains no data",
        ),
        ResourceStatus::Warming => ("RESOURCE_WARMING", "resource is still warming"),
        ResourceStatus::Degraded => (
            "RESOURCE_DEGRADED_WITHOUT_PROBLEM",
            "resource is degraded without typed problem evidence",
        ),
        ResourceStatus::Partial => (
            "RESOURCE_PARTIAL_WITHOUT_PROBLEM",
            "resource is partial without typed problem evidence",
        ),
        ResourceStatus::Error => (
            "RESOURCE_ERROR_WITHOUT_PROBLEM",
            "resource failed without typed problem evidence",
        ),
    };
    ApiProblem::new(code, message).with_source(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_resource_publishes_data_as_stale_with_typed_problem() {
        let problem = ApiProblem::new("GATE_PARTIAL", "gate positions failed")
            .with_status(502)
            .with_request_id(Some("req-pr-ay-gate".to_owned()))
            .with_retry_after_ms(Some(3_000))
            .with_source("gate.positions");
        let envelope = ResourceEnvelope::with_data(
            7_u8,
            ResourceStatus::Degraded,
            "system-health-snapshot",
            1_000,
            vec![problem],
        );
        let mut state = LoadState::Loading;

        apply_resource_envelope(&mut state, envelope);

        assert_eq!(state.value(), Some(&7));
        assert_eq!(
            state
                .problem()
                .and_then(|problem| problem.request_id.as_deref()),
            Some("req-pr-ay-gate")
        );
        assert_eq!(
            state.problem().and_then(|problem| problem.retry_after_ms),
            Some(3_000)
        );
    }

    #[test]
    fn unusable_resource_preserves_previous_value() {
        let envelope = ResourceEnvelope::<u8>::unavailable(
            ResourceStatus::Error,
            "system-health-snapshot",
            1_000,
            vec![ApiProblem::new("SYSTEM_DOWN", "system health failed")],
        );
        let mut state = LoadState::Ready(9_u8);

        apply_resource_envelope(&mut state, envelope);

        assert_eq!(state.value(), Some(&9));
        assert_eq!(
            state.problem().map(|problem| problem.code.as_str()),
            Some("SYSTEM_DOWN")
        );
    }

    #[test]
    fn degraded_resource_without_problem_fails_closed() {
        let envelope = ResourceEnvelope::with_data(
            3_u8,
            ResourceStatus::Degraded,
            "system-health-snapshot",
            1_000,
            Vec::new(),
        );
        let mut state = LoadState::Loading;

        apply_resource_envelope(&mut state, envelope);

        assert_eq!(state.value(), Some(&3));
        assert_eq!(
            state.problem().map(|problem| problem.code.as_str()),
            Some("RESOURCE_DEGRADED_WITHOUT_PROBLEM")
        );
    }
}
