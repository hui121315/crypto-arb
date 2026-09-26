use crate::middleware::audit;
use crate::services::action_runs::{self, ActionRunStart};
use crate::services::onchain_comparison;
use crate::services::onchain_provider_credentials::{self, ProviderCredentialError};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use common::AppError;
use serde_json::json;
use shared_types::problem::codes;
use shared_types::{
    ActionRun, ActionRunKind, ActionRunStatus, OnchainBatchRemoveRequest, OnchainBatchSnapshot,
    OnchainComparisonConfigPatch, OnchainComparisonSnapshot, OnchainCrossChainAuthorizeRequest,
    OnchainCrossChainBuildRequest, OnchainCrossChainBuildResponse, OnchainCrossChainRecheckRequest, OnchainCrossChainRun,
    OnchainCrossChainRunsResponse, OnchainCrossChainSubmitRequest, OnchainExecutionBuildRequest,
    OnchainCrossChainRecoveryPreview, OnchainCrossChainRecoveryPreviewRequest,
    OnchainCrossChainRecoveryPlan, OnchainCrossChainRecoveryAuthorizeRequest, OnchainCrossChainRecoveryCancelRequest,
    OnchainExecutionBuildResponse, OnchainExecutionRunsResponse, OnchainExecutionSubmitRequest,
    OnchainExecutionSubmitResponse, OnchainProviderCredentialClearRequest,
    OnchainProviderCredentialMutationResponse, OnchainProviderCredentialUpdateRequest,
    OnchainProviderCredentialsResponse, OnchainReplenishmentAuthorizeRequest,
    OnchainReplenishmentBuildRequest, OnchainReplenishmentPlanResponse,
    OnchainReplenishmentPlansResponse, OnchainReplenishmentRun, OnchainReplenishmentRunsResponse,
    OnchainReplenishmentSubmitRequest, OnchainTokenApprovalBuildRequest,
    OnchainTokenApprovalBuildResponse, OnchainTokenApprovalRunsResponse,
    OnchainTokenApprovalSubmitRequest, OnchainTokenApprovalSubmitResponse,
};

mod cex_pairs;
mod token_identity;

const PROVIDER_CREDENTIAL_REPLAY_KEY: &[u8] = b"crossline-onchain-provider-credentials-v1";

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/onchain/comparison", get(snapshot))
        .route("/api/onchain/comparison/config", patch(update_config))
        .route("/api/onchain/comparison/refresh", post(refresh))
        .route(
            "/api/onchain/transfer-networks/refresh",
            post(refresh_transfer_networks),
        )
        .route("/api/onchain/cross-chain/build", post(build_cross_chain))
        .route(
            "/api/onchain/cross-chain/authorize",
            post(authorize_cross_chain),
        )
        .route("/api/onchain/cross-chain/submit", post(submit_cross_chain))
        .route("/api/onchain/cross-chain/recheck", post(recheck_cross_chain))
        .route("/api/onchain/cross-chain/recovery/preview", post(preview_cross_chain_recovery))
        .route("/api/onchain/cross-chain/recovery/reserve", post(reserve_cross_chain_recovery))
        .route("/api/onchain/cross-chain/recovery/cancel", post(cancel_cross_chain_recovery))
        .route("/api/onchain/cross-chain/runs", get(cross_chain_runs))
        .route("/api/onchain/execution/build", post(build_execution))
        .route("/api/onchain/execution/submit", post(submit_execution))
        .route("/api/onchain/execution/runs", get(execution_runs))
        .route(
            "/api/onchain/replenishment/build",
            post(build_replenishment),
        )
        .route(
            "/api/onchain/replenishment/authorize",
            post(authorize_replenishment),
        )
        .route(
            "/api/onchain/replenishment/submit",
            post(submit_replenishment),
        )
        .route("/api/onchain/replenishment/plans", get(replenishment_plans))
        .route("/api/onchain/replenishment/runs", get(replenishment_runs))
        .route("/api/onchain/replenishment/recheck", post(recheck_replenishment))
        .route(
            "/api/onchain/token-approval/build",
            post(build_token_approval),
        )
        .route(
            "/api/onchain/token-approval/submit",
            post(submit_token_approval),
        )
        .route("/api/onchain/token-approval/runs", get(token_approval_runs))
        .route(
            "/api/onchain/comparison/batch",
            get(batch_snapshot).post(add_to_batch),
        )
        .route(
            "/api/onchain/comparison/batch/remove",
            post(remove_batch_item),
        )
        .route("/api/onchain/cex-pairs", get(cex_pairs::list))
        .route("/api/onchain/token/resolve", post(token_identity::resolve))
        .route(
            "/api/onchain/credentials",
            get(provider_credentials).post(update_provider_credentials),
        )
        .route(
            "/api/onchain/credentials/clear",
            post(clear_provider_credentials),
        )
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionRunsQuery {
    limit: Option<usize>,
}

async fn execution_runs(
    State(state): State<AppState>,
    Query(query): Query<ExecutionRunsQuery>,
) -> Json<OnchainExecutionRunsResponse> {
    Json(onchain_comparison::execution_runs(
        &state,
        query.limit.unwrap_or(20),
    ))
}

async fn build_cross_chain(
    State(state): State<AppState>,
    Json(request): Json<OnchainCrossChainBuildRequest>,
) -> Result<Json<OnchainCrossChainBuildResponse>, AppError> {
    Ok(Json(
        onchain_comparison::build_cross_chain_preview(&state, &request).await?,
    ))
}

async fn build_execution(
    State(state): State<AppState>,
    Json(request): Json<OnchainExecutionBuildRequest>,
) -> Result<Json<OnchainExecutionBuildResponse>, AppError> {
    let current = onchain_comparison::snapshot(&state);
    if let Err(error) = crate::lifecycle::refresh_onchain_transfer_networks(&state, &current).await
    {
        tracing::warn!(%error, "on-chain build transfer evidence refresh failed");
    }
    onchain_comparison::project_latest(&state, common::time::now_ms(), true);
    Ok(Json(
        onchain_comparison::build_execution(&state, &request).await?,
    ))
}

async fn preview_cross_chain_recovery(
    State(state): State<AppState>, headers: HeaderMap,
    Json(request): Json<OnchainCrossChainRecoveryPreviewRequest>,
) -> Result<Json<OnchainCrossChainRecoveryPreview>, AppError> {
    let actor = audit::extract_actor(&headers);
    let result = onchain_comparison::preview_cross_chain_recovery(&state, &request, &actor).await;
    audit::record_http_event(&headers, "onchain_cross_chain.recovery_preview", &request.run_id,
        if result.is_ok() { "success" } else { "rejected" },
        json!({"assetIndex":request.asset_index,"readOnly":true,"broadcast":false}));
    result.map(Json)
}

async fn authorize_cross_chain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainCrossChainAuthorizeRequest>,
) -> Result<Json<OnchainCrossChainRun>, AppError> {
    let actor = audit::extract_actor(&headers);
    Ok(Json(onchain_comparison::authorize_cross_chain(
        &state, &request, &actor,
    )?))
}

async fn reserve_cross_chain_recovery(State(state): State<AppState>, headers: HeaderMap,
    Json(request): Json<OnchainCrossChainRecoveryAuthorizeRequest>) -> Result<Json<OnchainCrossChainRecoveryPlan>, AppError> {
    let actor = audit::extract_actor(&headers);
    let result = state.onchain_cross_chain_runs().reserve_recovery_plan(&request, &actor, common::time::now_ms());
    audit::record_http_event(&headers, "onchain_cross_chain.recovery_reserve", &request.plan_id,
        if result.is_ok() { "success" } else { "rejected" }, json!({"broadcast":false,"scope":"cross_chain_wallet"}));
    result.map(Json).map_err(|message| AppError::domain(StatusCode::CONFLICT, "ONCHAIN_RECOVERY_RESERVATION_REJECTED", message))
}

async fn cancel_cross_chain_recovery(State(state): State<AppState>, headers: HeaderMap,
    Json(request): Json<OnchainCrossChainRecoveryCancelRequest>) -> Result<Json<OnchainCrossChainRecoveryPlan>, AppError> {
    let actor = audit::extract_actor(&headers);
    let result = state.onchain_cross_chain_runs().cancel_recovery_plan(&request.plan_id, &actor, common::time::now_ms());
    audit::record_http_event(&headers, "onchain_cross_chain.recovery_cancel", &request.plan_id,
        if result.is_ok() { "success" } else { "rejected" }, json!({"broadcast":false}));
    result.map(Json).map_err(|message| AppError::domain(StatusCode::CONFLICT, "ONCHAIN_RECOVERY_CANCELLATION_REJECTED", message))
}

async fn submit_cross_chain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainCrossChainSubmitRequest>,
) -> Result<Json<OnchainCrossChainRun>, AppError> {
    let actor = audit::extract_actor(&headers);
    Ok(Json(
        onchain_comparison::submit_cross_chain(&state, &request, &actor).await?,
    ))
}

async fn cross_chain_runs(
    State(state): State<AppState>,
    Query(query): Query<ExecutionRunsQuery>,
) -> Json<OnchainCrossChainRunsResponse> {
    Json(onchain_comparison::cross_chain_runs(
        &state,
        query.limit.unwrap_or(20),
    ))
}

async fn recheck_cross_chain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainCrossChainRecheckRequest>,
) -> Result<Json<OnchainCrossChainRun>, AppError> {
    let actor = audit::extract_actor(&headers);
    let result = onchain_comparison::recheck_cross_chain(&state, &request, &actor);
    audit::record_http_event(
        &headers, "onchain_cross_chain.recheck", &request.run_id,
        if result.is_ok() { "success" } else { "rejected" },
        json!({"expectedPosition":request.expected_position,"readOnly":true,"broadcast":false}),
    );
    result.map(Json)
}

async fn build_replenishment(
    State(state): State<AppState>,
    Json(request): Json<OnchainReplenishmentBuildRequest>,
) -> Result<Json<OnchainReplenishmentPlanResponse>, AppError> {
    let current = onchain_comparison::snapshot(&state);
    if let Err(error) = crate::lifecycle::refresh_onchain_transfer_networks(&state, &current).await
    {
        tracing::warn!(%error, "on-chain replenishment transfer evidence refresh failed");
    }
    onchain_comparison::project_latest(&state, common::time::now_ms(), true);
    Ok(Json(
        onchain_comparison::build_replenishment(&state, &request).await?,
    ))
}

async fn replenishment_plans(
    State(state): State<AppState>,
    Query(query): Query<ExecutionRunsQuery>,
) -> Json<OnchainReplenishmentPlansResponse> {
    Json(onchain_comparison::replenishment_plans(
        &state,
        query.limit.unwrap_or(20),
    ))
}

async fn authorize_replenishment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainReplenishmentAuthorizeRequest>,
) -> Result<Json<OnchainReplenishmentRun>, AppError> {
    let actor = audit::extract_actor(&headers);
    Ok(Json(onchain_comparison::authorize_replenishment(
        &state, &request, &actor,
    )?))
}

async fn submit_replenishment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainReplenishmentSubmitRequest>,
) -> Result<Json<OnchainReplenishmentRun>, AppError> {
    let current = onchain_comparison::snapshot(&state);
    if let Err(error) = crate::lifecycle::refresh_onchain_transfer_networks(&state, &current).await
    {
        tracing::warn!(%error, "on-chain replenishment submit evidence refresh failed");
    }
    onchain_comparison::project_latest(&state, common::time::now_ms(), true);
    let actor = audit::extract_actor(&headers);
    Ok(Json(
        onchain_comparison::submit_replenishment(&state, &request, &actor).await?,
    ))
}

async fn replenishment_runs(
    State(state): State<AppState>,
    Query(query): Query<ExecutionRunsQuery>,
) -> Json<OnchainReplenishmentRunsResponse> {
    Json(onchain_comparison::replenishment_runs(
        &state,
        query.limit.unwrap_or(20),
    ))
}

async fn recheck_replenishment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<shared_types::OnchainReplenishmentRecheckRequest>,
) -> Result<Json<OnchainReplenishmentRun>, AppError> {
    let actor = audit::extract_actor(&headers);
    let result = onchain_comparison::recheck_replenishment(&state, &request, &actor);
    audit::record_http_event(&headers, "onchain_replenishment.recheck", &request.run_id,
        if result.is_ok() { "success" } else { "rejected" },
        json!({"clientTransferId":request.expected_client_transfer_id,"readOnly":true,"broadcast":false}));
    result.map(Json)
}

async fn submit_execution(
    State(state): State<AppState>,
    Json(request): Json<OnchainExecutionSubmitRequest>,
) -> Result<Json<OnchainExecutionSubmitResponse>, AppError> {
    Ok(Json(
        onchain_comparison::submit_execution(&state, &request).await?,
    ))
}

async fn build_token_approval(
    State(state): State<AppState>,
    Json(request): Json<OnchainTokenApprovalBuildRequest>,
) -> Result<Json<OnchainTokenApprovalBuildResponse>, AppError> {
    Ok(Json(
        onchain_comparison::build_token_approval(&state, &request).await?,
    ))
}

async fn submit_token_approval(
    State(state): State<AppState>,
    Json(request): Json<OnchainTokenApprovalSubmitRequest>,
) -> Result<Json<OnchainTokenApprovalSubmitResponse>, AppError> {
    Ok(Json(
        onchain_comparison::submit_token_approval(&state, &request).await?,
    ))
}

async fn token_approval_runs(
    State(state): State<AppState>,
    Query(query): Query<ExecutionRunsQuery>,
) -> Json<OnchainTokenApprovalRunsResponse> {
    Json(onchain_comparison::token_approval_runs(
        &state,
        query.limit.unwrap_or(20),
    ))
}

async fn provider_credentials() -> Json<OnchainProviderCredentialsResponse> {
    Json(onchain_provider_credentials::status())
}

async fn update_provider_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainProviderCredentialUpdateRequest>,
) -> Result<Json<OnchainProviderCredentialMutationResponse>, AppError> {
    provider_credential_mutation(&state, headers, ProviderCredentialMutation::Update(request)).await
}

async fn clear_provider_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainProviderCredentialClearRequest>,
) -> Result<Json<OnchainProviderCredentialMutationResponse>, AppError> {
    provider_credential_mutation(&state, headers, ProviderCredentialMutation::Clear(request)).await
}

enum ProviderCredentialMutation {
    Update(OnchainProviderCredentialUpdateRequest),
    Clear(OnchainProviderCredentialClearRequest),
}

impl ProviderCredentialMutation {
    fn provider(&self) -> &str {
        match self {
            Self::Update(request) => &request.provider,
            Self::Clear(request) => &request.provider,
        }
    }

    fn kind(&self) -> ActionRunKind {
        match self {
            Self::Update(_) => ActionRunKind::OnchainProviderCredentialsUpdate,
            Self::Clear(_) => ActionRunKind::OnchainProviderCredentialsClear,
        }
    }

    fn accepted_message(&self) -> &'static str {
        match self {
            Self::Update(_) => "on-chain provider credentials update accepted",
            Self::Clear(_) => "on-chain provider credentials clear accepted",
        }
    }

    fn completed_message(&self) -> &'static str {
        match self {
            Self::Update(_) => "on-chain provider credentials updated",
            Self::Clear(_) => "on-chain provider credentials cleared",
        }
    }

    fn canonical(&self) -> String {
        match self {
            Self::Update(request) => canonical_update(request),
            Self::Clear(request) => canonical_clear(request),
        }
    }

    async fn execute(
        self,
    ) -> Result<OnchainProviderCredentialMutationResponse, ProviderCredentialError> {
        match self {
            Self::Update(request) => onchain_provider_credentials::update(request).await,
            Self::Clear(request) => onchain_provider_credentials::clear(request).await,
        }
    }
}

async fn provider_credential_mutation(
    state: &AppState,
    headers: HeaderMap,
    request: ProviderCredentialMutation,
) -> Result<Json<OnchainProviderCredentialMutationResponse>, AppError> {
    let completed_message = request.completed_message();
    let key = provider_credential_idempotency_key(&headers, &request);
    let begin = action_runs::begin_idempotent(
        state,
        ActionRunStart::new(
            request.kind(),
            &headers,
            Some(request.provider().to_owned()),
            request.accepted_message(),
        )
        .with_idempotency_key(Some(key)),
    )?;
    if begin.is_replayed() {
        return replay_provider_credentials(begin.run());
    }
    let run = begin.run();
    let result = provider_credential_response(state, request, run).await;
    action_runs::finish_result_with_payload(state, &run.id, result, completed_message).map(Json)
}

async fn provider_credential_response(
    state: &AppState,
    request: ProviderCredentialMutation,
    run: &ActionRun,
) -> Result<OnchainProviderCredentialMutationResponse, AppError> {
    let mut response = request
        .execute()
        .await
        .map_err(|error| map_provider_credential_error(&error))?;
    onchain_comparison::refresh_provider_runtime(state, common::time::now_ms());
    response.action_run_id = Some(run.id.clone());
    response.request_id = run.request_id.clone();
    Ok(response)
}

fn replay_provider_credentials(
    run: &ActionRun,
) -> Result<Json<OnchainProviderCredentialMutationResponse>, AppError> {
    match run.status {
        ActionRunStatus::Succeeded => {
            let mut response: OnchainProviderCredentialMutationResponse =
                action_runs::replay_payload(run)?;
            response.action_run_id = Some(run.id.clone());
            response.request_id = run.request_id.clone();
            response.message = format!("{}；重复请求已回放", response.message);
            Ok(Json(response))
        }
        ActionRunStatus::Failed => Err(AppError::domain(
            StatusCode::CONFLICT,
            codes::ACTION_RUN_REPLAY_FAILED,
            "provider credential mutation idempotency key already failed",
        )
        .with_details(json!({
            "actionRunId": run.id,
            "requestId": run.request_id,
            "problem": run.problem,
        }))),
        ActionRunStatus::Accepted => Err(AppError::domain(
            StatusCode::CONFLICT,
            codes::ACTION_RUN_IN_FLIGHT,
            "provider credential mutation idempotency key is already in flight",
        )
        .with_details(json!({
            "actionRunId": run.id,
            "requestId": run.request_id,
        }))),
    }
}

fn provider_credential_idempotency_key(
    headers: &HeaderMap,
    request: &ProviderCredentialMutation,
) -> String {
    action_runs::explicit_idempotency_key(headers).unwrap_or_else(|| {
        let digest = common::signing::hmac_sha256_hex(
            PROVIDER_CREDENTIAL_REPLAY_KEY,
            request.canonical().as_bytes(),
        );
        format!(
            "onchain-provider-credentials:{}:{}",
            request.provider().trim().to_ascii_lowercase(),
            &digest[..16]
        )
    })
}

fn canonical_update(request: &OnchainProviderCredentialUpdateRequest) -> String {
    let mut fields = request
        .fields
        .iter()
        .map(|field| (field.key.trim(), field.value.trim()))
        .collect::<Vec<_>>();
    fields.sort_unstable();
    let mut canonical = format!("update\n{}", request.provider.trim().to_ascii_lowercase());
    for (key, value) in fields {
        canonical.push('\n');
        canonical.push_str(key);
        canonical.push('=');
        canonical.push_str(value);
    }
    canonical
}

fn canonical_clear(request: &OnchainProviderCredentialClearRequest) -> String {
    let mut fields = request
        .fields
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    fields.sort_unstable();
    format!(
        "clear\n{}\n{}",
        request.provider.trim().to_ascii_lowercase(),
        fields.join("\n")
    )
}

fn map_provider_credential_error(error: &ProviderCredentialError) -> AppError {
    let message = error.to_string();
    match error {
        ProviderCredentialError::UnknownProvider(_)
        | ProviderCredentialError::UnknownField { .. }
        | ProviderCredentialError::DuplicateField { .. }
        | ProviderCredentialError::NoFields
        | ProviderCredentialError::InvalidCredential { .. } => AppError::domain(
            StatusCode::BAD_REQUEST,
            "ONCHAIN_PROVIDER_CREDENTIAL_INVALID",
            message,
        ),
        ProviderCredentialError::Storage(_) => AppError::domain(
            StatusCode::SERVICE_UNAVAILABLE,
            "ONCHAIN_PROVIDER_CREDENTIAL_STORAGE_FAILED",
            message,
        ),
    }
}

async fn snapshot(State(state): State<AppState>) -> Json<OnchainComparisonSnapshot> {
    Json((*onchain_comparison::snapshot(&state)).clone())
}

async fn update_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<OnchainComparisonConfigPatch>,
) -> Result<Json<OnchainComparisonSnapshot>, AppError> {
    let claim = action_runs::begin_idempotent(&state, ActionRunStart::new(
        ActionRunKind::OnchainComparisonConfigUpdate, &headers,
        Some("onchain-cex-comparison".to_owned()), "onchain configuration accepted",
    ).with_idempotency_key(action_runs::explicit_idempotency_key(&headers)))?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let result = onchain_comparison::update_config(&state, &patch, common::time::now_ms())
        .await.map(|snapshot| (*snapshot).clone());
    let snapshot = action_runs::finish_result_with_payload(
        &state, &claim.run().id, result, "onchain configuration saved",
    )?;
    audit::record_http_event(
        &headers,
        "onchain_comparison.config.update",
        "onchain-cex-comparison",
        "success",
        json!({
            "enabled": snapshot.config.enabled,
            "chain": snapshot.config.chain,
            "provider": snapshot.config.provider,
            "poolOrRoute": snapshot.config.pool_or_route,
            "rpcMode": snapshot.config.rpc.mode,
            "rpcConfigured": snapshot.rpc_status.configured,
            "rpcReady": snapshot.rpc_status.ready,
            "cexVenue": snapshot.config.cex_venue,
            "cexSymbol": snapshot.config.cex_symbol,
            "readOnly": true,
        }),
    );
    Ok(Json(snapshot))
}

async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<OnchainComparisonSnapshot> {
    onchain_comparison::refresh(&state, common::time::now_ms()).await;
    let snapshot = onchain_comparison::snapshot(&state);
    audit::record_http_event(
        &headers,
        "onchain_comparison.refresh",
        "onchain-cex-comparison",
        "success",
        json!({ "quality": snapshot.quality, "readOnly": true }),
    );
    Json((*snapshot).clone())
}

async fn refresh_transfer_networks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OnchainComparisonSnapshot>, AppError> {
    let current = onchain_comparison::snapshot(&state);
    let result = crate::lifecycle::refresh_onchain_transfer_networks(&state, &current).await;
    if let Err(error) = &result {
        tracing::warn!(%error, "on-chain transfer evidence refresh failed");
    }
    onchain_comparison::project_latest(&state, common::time::now_ms(), true);
    let snapshot = onchain_comparison::snapshot(&state);
    let requested = matches!(&result, Ok(true));
    let problem = result.as_ref().err().cloned();
    audit::record_http_event(
        &headers,
        "onchain_comparison.transfer_networks.refresh",
        "onchain-cex-comparison",
        if result.is_ok() {
            "success"
        } else {
            "degraded"
        },
        json!({
            "requested": requested,
            "problem": problem.as_deref(),
            "venue": snapshot.config.cex_venue,
            "readOnly": true,
        }),
    );
    if let Some(problem) = problem {
        return Err(AppError::domain(
            StatusCode::CONFLICT,
            "ONCHAIN_TRANSFER_EVIDENCE_UNAVAILABLE",
            problem,
        )
        .with_details(json!({
            "venue": snapshot.config.cex_venue,
            "readOnly": true,
        })));
    }
    Ok(Json((*snapshot).clone()))
}

async fn batch_snapshot(State(state): State<AppState>) -> Json<OnchainBatchSnapshot> {
    Json((*onchain_comparison::batch_snapshot(&state)).clone())
}

async fn add_to_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<OnchainComparisonConfigPatch>,
) -> Result<Json<OnchainBatchSnapshot>, AppError> {
    let claim = action_runs::begin_idempotent(&state, ActionRunStart::new(
        ActionRunKind::OnchainBatchAdd, &headers,
        Some("onchain-cex-comparison".to_owned()), "onchain batch addition accepted",
    ).with_idempotency_key(action_runs::explicit_idempotency_key(&headers)))?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let result = onchain_comparison::add_batch_config(&state, &patch, common::time::now_ms())
        .await.map(|snapshot| (*snapshot).clone());
    let snapshot = action_runs::finish_result_with_payload(
        &state, &claim.run().id, result, "onchain batch market added",
    )?;
    audit::record_http_event(
        &headers,
        "onchain_comparison.batch.add",
        "onchain-cex-comparison",
        "success",
        json!({
            "items": snapshot.items.len(),
            "activeConfigChanged": false,
            "readOnly": true,
        }),
    );
    Ok(Json(snapshot))
}

async fn remove_batch_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<OnchainBatchRemoveRequest>,
) -> Result<Json<OnchainBatchSnapshot>, AppError> {
    let claim = action_runs::begin_idempotent(&state, ActionRunStart::new(
        ActionRunKind::OnchainBatchRemove, &headers,
        Some(request.item_id.trim().to_owned()), "onchain batch removal accepted",
    ).with_idempotency_key(action_runs::explicit_idempotency_key(&headers)))?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let result = onchain_comparison::remove_batch_item(
        &state,
        request.item_id.trim(),
        common::time::now_ms(),
    )
    .await.map(|snapshot| (*snapshot).clone());
    let snapshot = action_runs::finish_result_with_payload(
        &state, &claim.run().id, result, "onchain batch market removed",
    )?;
    audit::record_http_event(
        &headers,
        "onchain_comparison.batch.remove",
        "onchain-cex-comparison",
        "success",
        json!({
            "itemId": request.item_id,
            "items": snapshot.items.len(),
            "readOnly": true,
        }),
    );
    Ok(Json(snapshot))
}

#[cfg(test)]
#[path = "onchain/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "onchain/cross_chain_tests.rs"]
mod cross_chain_tests;

#[cfg(test)]
#[path = "onchain/replenishment_recovery_tests.rs"]
mod replenishment_recovery_tests;
