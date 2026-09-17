use crate::state::section::prefixed_problem_message;
use shared_types::ApiProblem;

pub(in crate::panels::modules::execution) fn execution_problem_text(
    prefix: &str,
    problem: &ApiProblem,
) -> String {
    prefixed_problem_message(prefix, problem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_preview_problem_context_formatter() {
        let problem = ApiProblem::new("PREVIEW_LIMITED", "preview slow")
            .with_source("preview-rest")
            .with_status(429)
            .with_request_id(Some("req-preview-1".into()))
            .with_retry_after_ms(Some(2_000))
            .with_recovery_action(shared_types::ApiRecoveryAction::RetryAfterDelay);

        let text = execution_problem_text("预览失败", &problem);

        assert!(text.contains("code PREVIEW_LIMITED"));
        assert!(text.contains("source preview-rest"));
        assert!(text.contains("HTTP 429"));
        assert!(text.contains("request_id req-preview-1"));
        assert!(text.contains("retry 2000ms"));
        assert!(text.contains("下一步 等待后重试"));
    }
}
