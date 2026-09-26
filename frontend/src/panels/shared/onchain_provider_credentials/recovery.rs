use crate::api::rest::{ApiClient, ApiError, MutationRequestContext};
use futures::future::{select, Either};
use gloo_timers::future::TimeoutFuture;
use serde::{Deserialize, Serialize};
use shared_types::{
    ActionRun, ActionRunKind, ActionRunStatus, ApiProblem,
    OnchainProviderCredentialMutationResponse, ResourceStatus,
};
use std::future::Future;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CredentialOperation {
    Save,
    Clear,
}

impl CredentialOperation {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Save => "保存",
            Self::Clear => "清除",
        }
    }

    pub(super) fn kind(self) -> ActionRunKind {
        match self {
            Self::Save => ActionRunKind::OnchainProviderCredentialsUpdate,
            Self::Clear => ActionRunKind::OnchainProviderCredentialsClear,
        }
    }
}

// Recovery keeps correlation only. No credential values or request bodies survive timeout.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CredentialAttempt {
    pub provider: String,
    pub operation: CredentialOperation,
    pub context: MutationRequestContext,
    pub run_id: Option<String>,
}

impl CredentialAttempt {
    pub(super) fn new(provider: String, operation: CredentialOperation) -> Self {
        let context = MutationRequestContext::new_idempotent_attempt(format!(
            "onchain-provider-credentials-{}-{provider}",
            operation.kind().as_str(),
        ));
        Self {
            provider,
            operation,
            context,
            run_id: None,
        }
    }

    pub(super) fn validate_response(
        &self,
        response: &OnchainProviderCredentialMutationResponse,
    ) -> Result<(), ApiError> {
        if response.provider != self.provider
            || response.configured_count > response.field_count
            || response
                .request_id
                .as_deref()
                .is_some_and(|id| id != self.context.request_id())
            || self
                .run_id
                .as_ref()
                .is_some_and(|id| response.action_run_id.as_ref() != Some(id))
        {
            return Err(ApiError::client(
                "PROVIDER_RECEIPT_MISMATCH",
                "凭证处理结果与原操作不匹配，结果仍未确认",
            ));
        }
        Ok(())
    }
}

pub(super) enum CredentialRecovery {
    Waiting(CredentialAttempt),
    Succeeded(OnchainProviderCredentialMutationResponse),
    Failed(ApiProblem),
}

pub(super) async fn read_with_timeout<T>(
    operation: &'static str,
    request: impl Future<Output = Result<T, ApiError>>,
) -> Result<T, ApiError> {
    let timeout = TimeoutFuture::new(10_000);
    futures::pin_mut!(request, timeout);
    match select(request, timeout).await {
        Either::Left((result, _)) => result,
        Either::Right((_, _)) => Err(ApiError::client(
            "TIMEOUT",
            format!("{operation}超过 10 秒未响应，请稍后重试"),
        )),
    }
}

pub(super) fn outcome_unknown(error: &ApiError) -> bool {
    // A local/gateway failure cannot prove that the server never applied the write.
    !matches!(error.problem.status, Some(400..=499))
        || error.problem.status == Some(408)
        || error.problem.code == shared_types::problem::codes::ACTION_RUN_IN_FLIGHT
}

pub(super) async fn recover_attempt(
    client: &ApiClient,
    mut attempt: CredentialAttempt,
) -> Result<CredentialRecovery, ApiError> {
    let run = if let Some(id) = &attempt.run_id {
        client.action_run(id).await?
    } else {
        let envelope = client.action_runs_envelope().await?;
        if !matches!(
            envelope.status,
            ResourceStatus::Ready | ResourceStatus::Partial
        ) {
            return Err(ApiError::client(
                "PROVIDER_RECEIPT_UNAVAILABLE",
                "动作账本尚不可核对，保留原操作",
            ));
        }
        let rows = envelope.data.unwrap_or_default();
        let mut matches = rows
            .into_iter()
            .filter(|run| matches_attempt(run, &attempt));
        let run = matches.next().ok_or_else(|| {
            ApiError::client(
                "PROVIDER_RECEIPT_NOT_FOUND",
                "保留的动作账本中未查到原操作，不能据此认定未执行；请稍后核对，不要重复提交",
            )
        })?;
        if matches.next().is_some() {
            return Err(ApiError::client(
                "PROVIDER_RECEIPT_AMBIGUOUS",
                "原操作存在多个处理结果，暂不能确认结果",
            ));
        }
        run
    };
    if run.id.is_empty()
        || !matches_attempt(&run, &attempt)
        || attempt.run_id.as_ref().is_some_and(|id| id != &run.id)
    {
        return Err(ApiError::client(
            "PROVIDER_RECEIPT_MISMATCH",
            "动作处理结果身份不匹配，保留原操作",
        ));
    }
    attempt.run_id = Some(run.id.clone());
    match run.status {
        ActionRunStatus::Accepted => Ok(CredentialRecovery::Waiting(attempt)),
        ActionRunStatus::Failed => Ok(CredentialRecovery::Failed(
            run.problem
                .unwrap_or_else(|| ApiProblem::new("PROVIDER_OPERATION_FAILED", run.message)),
        )),
        ActionRunStatus::Succeeded => {
            if run.problem.is_some() {
                return Err(ApiError::client(
                    "PROVIDER_RECEIPT_MISMATCH",
                    "成功处理结果仍包含失败信息，结果待核对",
                ));
            }
            let response: OnchainProviderCredentialMutationResponse = run
                .result
                .and_then(|value| serde_json::from_value(value).ok())
                .ok_or_else(|| {
                    ApiError::client(
                        "PROVIDER_RECEIPT_MISSING",
                        "成功动作缺少凭证结果，不能视为配置已确认",
                    )
                })?;
            attempt.validate_response(&response)?;
            Ok(CredentialRecovery::Succeeded(response))
        }
    }
}

fn matches_attempt(run: &ActionRun, attempt: &CredentialAttempt) -> bool {
    run.kind == attempt.operation.kind()
        && run.target.as_deref() == Some(attempt.provider.as_str())
        && run.idempotency_key.as_deref() == attempt.context.idempotency_key()
        && run.request_id.as_deref() == Some(attempt.context.request_id())
}
