use crate::api::rest::ApiClient;
use crate::state::load_state::LoadState;
use crate::state::section::problem_message;
use leptos::prelude::*;
use shared_types::{ApiProblem, StrategyKindInfo};

#[derive(Clone, Copy)]
pub(crate) struct StrategyKindsStore {
    pub(crate) state: RwSignal<LoadState<Vec<StrategyKindInfo>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StrategyKindsView {
    pub(crate) rows: Vec<StrategyKindInfo>,
    pub(crate) note: Option<StrategyKindsNote>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StrategyKindsNote {
    pub(crate) text: String,
    pub(crate) is_error: bool,
}

pub(crate) fn provide_strategy_kinds(client: &ApiClient) {
    let state = RwSignal::new(LoadState::Loading);
    let client = client.clone();
    let resource = LocalResource::new(move || {
        let client = client.clone();
        async move {
            client
                .main_strategy_kinds()
                .await
                .map_err(|error| error.problem)
        }
    });
    Effect::new(move |_| {
        let Some(result) = resource.get() else {
            return;
        };
        state.update(|state| apply_strategy_kinds_result(state, &result));
    });
    provide_context(StrategyKindsStore { state });
}

pub(crate) fn use_strategy_kinds() -> StrategyKindsStore {
    expect_context::<StrategyKindsStore>()
}

pub(crate) fn strategy_kinds_view(state: &LoadState<Vec<StrategyKindInfo>>) -> StrategyKindsView {
    match state {
        LoadState::Loading => StrategyKindsView {
            rows: Vec::new(),
            note: Some(StrategyKindsNote::info("策略范围确认中")),
        },
        LoadState::Ready(rows) => StrategyKindsView {
            rows: rows.clone(),
            note: rows
                .is_empty()
                .then(|| StrategyKindsNote::info("当前没有开放策略")),
        },
        LoadState::Stale { value, problem } => StrategyKindsView {
            rows: value.clone(),
            note: Some(StrategyKindsNote::error(format!(
                "策略种类刷新失败 · {}",
                problem_message(problem)
            ))),
        },
        LoadState::Error(problem) => StrategyKindsView {
            rows: Vec::new(),
            note: Some(StrategyKindsNote::error(format!(
                "策略种类获取失败 · {}",
                problem_message(problem)
            ))),
        },
    }
}

impl StrategyKindsNote {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

fn apply_strategy_kinds_result(
    state: &mut LoadState<Vec<StrategyKindInfo>>,
    result: &Result<Vec<StrategyKindInfo>, ApiProblem>,
) {
    match result {
        Ok(kinds) => state.apply_result(Ok(kinds.clone())),
        Err(problem) => state.apply_result(Err(problem.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::StrategyKind;

    #[test]
    fn strategy_kind_loading_has_visible_note() {
        let state = LoadState::Loading;

        let view = strategy_kinds_view(&state);

        assert!(view.rows.is_empty());
        assert_eq!(
            view.note.as_ref().map(|note| note.text.as_str()),
            Some("策略范围确认中")
        );
        assert_eq!(view.note.as_ref().map(|note| note.is_error), Some(false));
    }

    #[test]
    fn strategy_kind_error_without_cache_is_cold_error() {
        let problem = ApiProblem::new("STRATEGY_KINDS_DOWN", "strategy kinds unavailable")
            .with_request_id(Some("req-strategy-1".into()))
            .with_retry_after_ms(Some(5_000));
        let mut state = LoadState::Loading;

        apply_strategy_kinds_result(&mut state, &Err(problem));

        let view = strategy_kinds_view(&state);
        assert!(view.rows.is_empty());
        let note = view.note.as_ref().map(|note| note.text.as_str());
        assert!(note.is_some_and(|text| text.contains("strategy kinds unavailable")));
        assert!(note.is_some_and(|text| text.contains("request_id req-strategy-1")));
        assert!(note.is_some_and(|text| text.contains("retry 5000ms")));
        assert_eq!(view.note.as_ref().map(|note| note.is_error), Some(true));
    }

    #[test]
    fn strategy_kind_error_with_cache_keeps_stale_rows_and_problem() {
        let ready = vec![StrategyKindInfo::from_kind(StrategyKind::PerpCross, true)];
        let problem = ApiProblem::new("STRATEGY_KINDS_RATE_LIMITED", "rate limited")
            .with_request_id(Some("req-strategy-2".into()))
            .with_retry_after_ms(Some(15_000));
        let mut state = LoadState::Ready(ready.clone());

        apply_strategy_kinds_result(&mut state, &Err(problem));

        let view = strategy_kinds_view(&state);
        assert_eq!(view.rows, ready);
        let note = view.note.as_ref().map(|note| note.text.as_str());
        assert!(note.is_some_and(|text| text.contains("rate limited")));
        assert!(note.is_some_and(|text| text.contains("request_id req-strategy-2")));
        assert!(note.is_some_and(|text| text.contains("retry 15000ms")));
        assert_eq!(view.note.as_ref().map(|note| note.is_error), Some(true));
    }

    #[test]
    fn strategy_kind_success_clears_stale_problem() {
        let ready = vec![StrategyKindInfo::from_kind(StrategyKind::PerpCross, true)];
        let problem = ApiProblem::new("STRATEGY_KINDS_RATE_LIMITED", "rate limited");
        let mut state = LoadState::Stale {
            value: ready.clone(),
            problem,
        };

        apply_strategy_kinds_result(&mut state, &Ok(ready.clone()));

        let view = strategy_kinds_view(&state);
        assert_eq!(view.rows, ready);
        assert!(view.note.is_none());
    }
}
