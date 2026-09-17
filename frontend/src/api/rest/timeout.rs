use std::future::Future;

use futures::future::{select, Either};
use gloo_timers::future::TimeoutFuture;
use serde_json::json;
use shared_types::ApiProblem;

use super::ApiError;

pub(crate) const MUTATION_TIMEOUT_MS: u32 = 20_000;

pub(crate) async fn with_mutation_timeout<T>(
    operation: &'static str,
    request: impl Future<Output = Result<T, ApiError>>,
) -> Result<T, ApiError> {
    with_mutation_timeout_ms(operation, request, MUTATION_TIMEOUT_MS).await
}

pub(crate) async fn with_mutation_timeout_ms<T>(
    operation: &'static str,
    request: impl Future<Output = Result<T, ApiError>>,
    timeout_ms: u32,
) -> Result<T, ApiError> {
    let timeout = TimeoutFuture::new(timeout_ms);
    futures::pin_mut!(request, timeout);
    match select(request, timeout).await {
        Either::Left((result, _)) => result,
        Either::Right((_, _)) => Err(mutation_timeout_error(operation, timeout_ms)),
    }
}

fn mutation_timeout_error(operation: &str, timeout_ms: u32) -> ApiError {
    let mut problem = ApiProblem::new(
        "MUTATION_TIMEOUT",
        format!("{operation}超时：请检查 API Base、后端运行态或网络连接"),
    )
    .with_source("frontend.mutation_timeout");
    problem.details = Some(json!({
        "operation": operation,
        "timeoutMs": timeout_ms,
    }));
    ApiError::from_problem(problem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_timeout_is_typed_and_operation_scoped() {
        let error = mutation_timeout_error("对冲提交", MUTATION_TIMEOUT_MS);

        assert_eq!(error.problem.code, "MUTATION_TIMEOUT");
        assert_eq!(
            error.problem.source.as_deref(),
            Some("frontend.mutation_timeout")
        );
        assert!(error.problem.message.contains("对冲提交超时"));
        assert_eq!(
            error
                .problem
                .details
                .as_ref()
                .and_then(|details| details.get("timeoutMs"))
                .and_then(serde_json::Value::as_u64),
            Some(u64::from(MUTATION_TIMEOUT_MS))
        );
    }

    #[test]
    fn stock_pair_recheck_timeout_preserves_its_request_budget() {
        let error = mutation_timeout_error("核对股票两腿回执", 40_000);
        assert_eq!(error.problem.details.unwrap()["timeoutMs"], 40_000);
        assert_eq!(MUTATION_TIMEOUT_MS, 20_000);
    }
}
