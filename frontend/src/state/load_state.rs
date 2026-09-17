use shared_types::ApiProblem;

#[derive(Debug, Clone, PartialEq)]
pub enum LoadState<T> {
    Loading,
    Ready(T),
    Stale { value: T, problem: ApiProblem },
    Error(ApiProblem),
}

impl<T> LoadState<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            LoadState::Ready(value) | LoadState::Stale { value, .. } => Some(value),
            LoadState::Loading | LoadState::Error(_) => None,
        }
    }

    pub fn problem(&self) -> Option<&ApiProblem> {
        match self {
            LoadState::Stale { problem, .. } | LoadState::Error(problem) => Some(problem),
            LoadState::Loading | LoadState::Ready(_) => None,
        }
    }

    pub fn apply_result(&mut self, result: Result<T, ApiProblem>) {
        match result {
            Ok(value) => *self = LoadState::Ready(value),
            Err(problem) => self.apply_error(problem),
        }
    }

    fn apply_error(&mut self, problem: ApiProblem) {
        if let Some(value) = self.take_value() {
            *self = LoadState::Stale { value, problem };
        } else {
            *self = LoadState::Error(problem);
        }
    }

    fn take_value(&mut self) -> Option<T> {
        match std::mem::replace(self, LoadState::Loading) {
            LoadState::Ready(value) | LoadState::Stale { value, .. } => Some(value),
            LoadState::Loading | LoadState::Error(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_after_ready_preserves_stale_value() {
        let mut state = LoadState::Ready(7);

        state.apply_result(Err(problem("TIMEOUT")));

        assert_eq!(state.value(), Some(&7));
        assert_eq!(state.problem().map(|p| p.code.as_str()), Some("TIMEOUT"));
    }

    #[test]
    fn error_before_ready_has_no_value() {
        let mut state: LoadState<u32> = LoadState::Loading;

        state.apply_result(Err(problem("NETWORK")));

        assert!(state.value().is_none());
        assert_eq!(state.problem().map(|p| p.code.as_str()), Some("NETWORK"));
    }

    fn problem(code: &str) -> ApiProblem {
        ApiProblem::new(code, "failed")
    }
}
