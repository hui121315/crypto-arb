use crate::state::action_state::ActionState;
use crate::state::section::prefixed_problem_message;
use leptos::prelude::*;
use shared_types::ApiProblem;

pub(in crate::panels::modules::settings) fn problem_cell(
    label: &'static str,
    problem: &ApiProblem,
) -> AnyView {
    let message = problem_message(label, problem);
    let title = problem_title(problem);
    view! {
        <div class="empty-cell" title=title>
            {message}
        </div>
    }
    .into_any()
}

pub(in crate::panels::modules::settings) fn problem_message(
    prefix: &str,
    problem: &ApiProblem,
) -> String {
    prefixed_problem_message(prefix, problem)
}

pub(in crate::panels::modules::settings) fn action_message(
    default: &str,
    state: &ActionState,
) -> String {
    state.message(default)
}

fn problem_title(problem: &ApiProblem) -> String {
    problem_message(&problem.code, problem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_message_keeps_request_context() {
        let problem = ApiProblem::new("RATE_LIMITED", "slow down")
            .with_source("settings.test")
            .with_status(429)
            .with_request_id(Some("req-1".into()))
            .with_retry_after_ms(Some(2_000))
            .with_recovery_action(shared_types::ApiRecoveryAction::RetryAfterDelay);

        let message = problem_message("失败", &problem);

        assert!(message.contains("HTTP 429"));
        assert!(message.contains("code RATE_LIMITED"));
        assert!(message.contains("source settings.test"));
        assert!(message.contains("request_id req-1"));
        assert!(message.contains("retry 2000ms"));
        assert!(message.contains("下一步 等待后重试"));
    }

    #[test]
    fn action_message_uses_typed_problem() {
        let problem = ApiProblem::new("TIMEOUT", "保存超时").with_status(408);
        let state = ActionState::failed("保存失败", problem);

        let message = action_message("默认", &state);

        assert!(message.contains("保存失败：保存超时"));
        assert!(message.contains("code TIMEOUT"));
        assert!(message.contains("HTTP 408"));
    }
}
