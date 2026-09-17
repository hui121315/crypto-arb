//! Settings mutation transport tasks and response evidence normalization.

use crate::api::rest::{
    with_mutation_timeout, ApiClient, ApiError, MutationRequestContext, TradingStatusResponse,
};
use shared_types::{
    ActionEvidence, KillSwitchRequest, KillSwitchResponse, RiskConfigPatch,
    SelectTradingAdapterRequest, VenueCredentialUpdateResponse,
};

pub(super) async fn select_trading_adapter_task(
    client: ApiClient,
    adapter_id: String,
    context: MutationRequestContext,
) -> Result<TradingStatusResponse, ApiError> {
    with_mutation_timeout(
        "执行环境切换",
        client.select_trading_adapter_with_context(
            &SelectTradingAdapterRequest { adapter_id },
            &context,
        ),
    )
    .await
}

pub(super) async fn save_venue_credentials_task(
    client: ApiClient,
    venue: String,
    fields: Vec<(String, String)>,
    context: MutationRequestContext,
) -> Result<VenueCredentialUpdateResponse, ApiError> {
    with_mutation_timeout(
        "凭证保存",
        client.save_venue_credentials_with_context(&venue, &fields, &context),
    )
    .await
}

pub(super) async fn set_kill_switch_task(
    client: ApiClient,
    request: KillSwitchRequest,
    context: MutationRequestContext,
) -> Result<KillSwitchResponse, ApiError> {
    with_mutation_timeout(
        "Kill Switch 更新",
        client.set_kill_switch_with_context(&request, &context),
    )
    .await
}

pub(super) async fn update_risk_config_task(
    client: ApiClient,
    patch: RiskConfigPatch,
    context: MutationRequestContext,
) -> Result<TradingStatusResponse, ApiError> {
    with_mutation_timeout(
        "风控参数保存",
        client.update_trading_risk_config_with_context(&patch, &context),
    )
    .await
}

pub(super) fn credential_response_evidence(
    response: &VenueCredentialUpdateResponse,
    idempotency_key: &str,
    pending: ActionEvidence,
) -> ActionEvidence {
    pending
        .with_request_id(response.request_id.clone())
        .with_action_run_id(response.action_run_id.clone())
        .with_idempotency_key(Some(idempotency_key.to_owned()))
}

pub(super) fn trading_status_evidence(
    response: &TradingStatusResponse,
    idempotency_key: &str,
    pending: ActionEvidence,
) -> ActionEvidence {
    pending
        .with_request_id(response.request_id.clone())
        .with_action_run_id(response.action_run_id.clone())
        .with_idempotency_key(
            response
                .idempotency_key
                .clone()
                .or_else(|| Some(idempotency_key.to_owned())),
        )
}

pub(super) fn kill_switch_evidence(
    response: &KillSwitchResponse,
    idempotency_key: &str,
    pending: ActionEvidence,
) -> ActionEvidence {
    pending
        .with_request_id(response.request_id.clone())
        .with_action_run_id(response.action_run_id.clone())
        .with_idempotency_key(
            response
                .idempotency_key
                .clone()
                .or_else(|| Some(idempotency_key.to_owned())),
        )
}
