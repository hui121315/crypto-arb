use gloo_storage::Storage;
use leptos::prelude::*;
use shared_types::{ActionState, ApiProblem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModuleRuntimeStatus {
    Loading,
    Ready,
    SetupRequired,
    Stale,
    Error,
    Pending,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModuleRuntimeState {
    pub status: ModuleRuntimeStatus,
    pub problem: Option<ApiProblem>,
    pub pending_label: Option<String>,
}

impl ModuleRuntimeState {
    pub(crate) const fn ready() -> Self {
        Self {
            status: ModuleRuntimeStatus::Ready,
            problem: None,
            pending_label: None,
        }
    }

    pub(crate) const fn setup_required() -> Self {
        Self {
            status: ModuleRuntimeStatus::SetupRequired,
            problem: None,
            pending_label: None,
        }
    }

    pub(crate) fn from_load_state<T>(state: &crate::state::load_state::LoadState<T>) -> Self {
        use crate::state::load_state::LoadState;
        match state {
            LoadState::Loading => Self::with_status(ModuleRuntimeStatus::Loading),
            LoadState::Ready(_) => Self::ready(),
            LoadState::Stale { problem, .. } => Self {
                status: ModuleRuntimeStatus::Stale,
                problem: Some(problem.clone()),
                pending_label: None,
            },
            LoadState::Error(problem) => Self {
                status: ModuleRuntimeStatus::Error,
                problem: Some(problem.clone()),
                pending_label: None,
            },
        }
    }

    pub(crate) fn from_action_state(state: &ActionState) -> Self {
        match state {
            ActionState::Pending { label, .. } | ActionState::Accepted { label, .. } => Self {
                status: ModuleRuntimeStatus::Pending,
                problem: None,
                pending_label: Some(label.clone()),
            },
            ActionState::Failed { problem, .. } => Self {
                status: ModuleRuntimeStatus::Error,
                problem: Some(problem.clone()),
                pending_label: None,
            },
            ActionState::Idle | ActionState::Succeeded { .. } => Self::ready(),
        }
    }

    pub(crate) fn from_problem(problem: Option<ApiProblem>) -> Self {
        problem.map_or_else(Self::ready, |problem| Self {
            status: ModuleRuntimeStatus::Error,
            problem: Some(problem),
            pending_label: None,
        })
    }

    pub(crate) fn combine(states: impl IntoIterator<Item = Self>) -> Self {
        states
            .into_iter()
            .max_by_key(|state| state.priority())
            .unwrap_or_else(Self::ready)
    }

    pub(crate) const fn slug(&self) -> &'static str {
        match self.status {
            ModuleRuntimeStatus::Loading => "loading",
            ModuleRuntimeStatus::Ready => "ready",
            ModuleRuntimeStatus::SetupRequired => "setup-required",
            ModuleRuntimeStatus::Stale => "stale",
            ModuleRuntimeStatus::Error => "error",
            ModuleRuntimeStatus::Pending => "pending",
        }
    }

    pub(crate) const fn label(&self) -> &'static str {
        match self.status {
            ModuleRuntimeStatus::Loading => "加载中",
            ModuleRuntimeStatus::Ready => "正常",
            ModuleRuntimeStatus::SetupRequired => "待配置",
            ModuleRuntimeStatus::Stale => "数据已过期",
            ModuleRuntimeStatus::Error => "读取失败",
            ModuleRuntimeStatus::Pending => "操作进行中",
        }
    }

    const fn with_status(status: ModuleRuntimeStatus) -> Self {
        Self {
            status,
            problem: None,
            pending_label: None,
        }
    }

    const fn priority(&self) -> u8 {
        match self.status {
            ModuleRuntimeStatus::Ready => 0,
            ModuleRuntimeStatus::SetupRequired => 1,
            ModuleRuntimeStatus::Loading => 1,
            ModuleRuntimeStatus::Stale => 2,
            ModuleRuntimeStatus::Pending => 3,
            ModuleRuntimeStatus::Error => 4,
        }
    }
}

pub(crate) fn stored_choice<T>(key: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<T> {
    let raw: String = gloo_storage::LocalStorage::get(key).ok()?;
    parse(&raw)
}

pub(crate) fn store_choice(key: &str, value: &str) {
    let _ = gloo_storage::LocalStorage::set(key, value);
}

pub(crate) fn clear_choice(key: &str) {
    gloo_storage::LocalStorage::delete(key);
}

pub(crate) fn stored_page(key: &str) -> Option<usize> {
    stored_choice(key, |value| {
        normalize_choice(value)
            .parse::<usize>()
            .ok()
            .filter(|page| *page > 0)
    })
}

pub(crate) fn store_page(key: &str, page: usize) {
    store_choice(key, &page.max(1).to_string());
}

pub(crate) fn persisted_choice_signal<T>(
    key: &'static str,
    fallback: T,
    parse: impl Fn(&str) -> Option<T> + 'static,
    serialize: impl Fn(&T) -> String + 'static,
) -> RwSignal<T>
where
    T: Send + Sync + 'static,
{
    let signal = RwSignal::new(stored_choice(key, parse).unwrap_or(fallback));
    Effect::new(move |_| {
        signal.with(|value| store_choice(key, &serialize(value)));
    });
    signal
}

pub(crate) fn normalize_choice(value: &str) -> &str {
    value.trim().trim_start_matches('#').trim_start_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::load_state::LoadState;

    #[test]
    fn normalize_choice_accepts_plain_hash_and_slash_values() {
        assert_eq!(normalize_choice(" risk "), "risk");
        assert_eq!(normalize_choice("#diagnostics"), "diagnostics");
        assert_eq!(normalize_choice("/executed"), "executed");
    }

    #[test]
    fn stored_page_parser_rejects_zero_and_non_numbers() {
        let parse = |value: &str| {
            normalize_choice(value)
                .parse::<usize>()
                .ok()
                .filter(|page| *page > 0)
        };
        assert_eq!(parse("3"), Some(3));
        assert_eq!(parse("#2"), Some(2));
        assert_eq!(parse("0"), None);
        assert_eq!(parse("abc"), None);
    }

    #[test]
    fn module_runtime_state_keeps_problem_and_pending_semantics() {
        let stale = ModuleRuntimeState::from_load_state(&LoadState::Stale {
            value: 7,
            problem: ApiProblem::new("STALE", "old snapshot"),
        });
        let pending = ModuleRuntimeState::from_action_state(&ActionState::pending("提交中"));
        let error = ModuleRuntimeState::from_load_state(&LoadState::<u8>::Error(ApiProblem::new(
            "NETWORK", "offline",
        )));

        assert_eq!(stale.slug(), "stale");
        assert_eq!(pending.pending_label.as_deref(), Some("提交中"));
        assert_eq!(
            error.problem.as_ref().map(|problem| problem.code.as_str()),
            Some("NETWORK")
        );
        assert_eq!(
            ModuleRuntimeState::combine([stale, pending, error]).slug(),
            "error"
        );
    }
}
