//! Default-off LLM diagnostics with a fail-closed external payload boundary.

use crate::middleware::audit;
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::AppError;
use dashmap::DashMap;
use llm::{ChatRequest, LlmError, Message, ProviderSelection, Usage};
use serde::Serialize;
use serde_json::json;
use shared_types::{
    problem::codes, LlmExternalPayloadError, LlmExternalRequest, LlmExternalResponse,
    LlmPromptContext, LlmProviderId,
};
use std::collections::VecDeque;
use std::sync::OnceLock;

const CHAT_RATE_LIMIT: usize = 10;
const CHAT_RATE_WINDOW_MS: i64 = 60_000;
const DIAGNOSTIC_MAX_TOKENS: u32 = 800;

static CHAT_RATE: OnceLock<DashMap<String, VecDeque<i64>>> = OnceLock::new();

struct PreparedExternalPrompt {
    provider: Option<LlmProviderId>,
    context: LlmPromptContext,
    outbound_json: String,
    redacted_bytes: usize,
    selection: ProviderSelection,
}

struct LlmAuditContext<'a> {
    route: &'static str,
    selection: &'a ProviderSelection,
    context: LlmPromptContext,
    redacted_bytes: usize,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/chat", post(chat))
        .route("/api/chat/providers", get(providers))
}

/// LLM diagnostics are not a product UI. They remain default-off and only
/// accept the shared allowlisted external payload contract.
pub(crate) fn llm_router() -> Router<AppState> {
    Router::new()
        .route("/api/llm/explain-opportunity", post(explain_opportunity))
        .route("/api/llm/diagnose-failure", post(diagnose_failure))
        .route("/api/llm/daily-brief", post(daily_brief))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProvidersResponse {
    default: Option<String>,
    available: Vec<llm::ProviderApiEvidence>,
}

async fn providers(State(state): State<AppState>) -> Json<ProvidersResponse> {
    let router = state.llm_router();
    Json(ProvidersResponse {
        default: router.default_name(),
        available: router.registered_evidence(),
    })
}

async fn chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LlmExternalRequest>,
) -> Result<Json<LlmExternalResponse>, AppError> {
    check_chat_rate(&headers)?;
    run_external_prompt(&state, &headers, "/api/chat", request, None).await
}

async fn explain_opportunity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LlmExternalRequest>,
) -> Result<Json<LlmExternalResponse>, AppError> {
    check_chat_rate(&headers)?;
    run_external_prompt(
        &state,
        &headers,
        "/api/llm/explain-opportunity",
        request,
        Some(LlmPromptContext::OpportunityExplanation),
    )
    .await
}

async fn diagnose_failure(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LlmExternalRequest>,
) -> Result<Json<LlmExternalResponse>, AppError> {
    check_chat_rate(&headers)?;
    run_external_prompt(
        &state,
        &headers,
        "/api/llm/diagnose-failure",
        request,
        Some(LlmPromptContext::FailureDiagnosis),
    )
    .await
}

async fn daily_brief(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LlmExternalRequest>,
) -> Result<Json<LlmExternalResponse>, AppError> {
    check_chat_rate(&headers)?;
    run_external_prompt(
        &state,
        &headers,
        "/api/llm/daily-brief",
        request,
        Some(LlmPromptContext::DailyBrief),
    )
    .await
}

async fn run_external_prompt(
    state: &AppState,
    headers: &HeaderMap,
    route: &'static str,
    request: LlmExternalRequest,
    expected_context: Option<LlmPromptContext>,
) -> Result<Json<LlmExternalResponse>, AppError> {
    let PreparedExternalPrompt {
        provider,
        context,
        outbound_json,
        redacted_bytes,
        selection,
    } = prepare_external_prompt(state, &request, expected_context)?;
    let audit_context = LlmAuditContext {
        route,
        selection: &selection,
        context,
        redacted_bytes,
    };
    record_llm_durable_audit(headers, &audit_context, "accepted")?;

    let response = state
        .llm_router()
        .chat(
            provider.map(LlmProviderId::as_str),
            diagnostic_request(context, outbound_json),
        )
        .await;
    finish_external_prompt(headers, &audit_context, response)
}

fn finish_external_prompt(
    headers: &HeaderMap,
    audit_context: &LlmAuditContext<'_>,
    response: Result<llm::ChatResponse, LlmError>,
) -> Result<Json<LlmExternalResponse>, AppError> {
    match response {
        Ok(response) => Ok(finish_external_success(headers, audit_context, response)),
        Err(error) => Err(finish_external_failure(headers, audit_context, error)),
    }
}

fn finish_external_success(
    headers: &HeaderMap,
    audit_context: &LlmAuditContext<'_>,
    response: llm::ChatResponse,
) -> Json<LlmExternalResponse> {
    record_llm_http_audit(
        headers,
        audit_context,
        "success",
        &AppError::domain(StatusCode::OK, "LLM_OK", "completed"),
        Some(&response.usage),
    );
    tracing::info!(
        route = audit_context.route,
        provider = audit_context.selection.name.as_str(),
        model = response.model.as_str(),
        payload_class = audit_context.context.as_str(),
        redacted_bytes = audit_context.redacted_bytes,
        prompt_tokens = response.usage.prompt_tokens,
        completion_tokens = response.usage.completion_tokens,
        total_tokens = response.usage.total_tokens,
        "llm outbound completed"
    );
    Json(LlmExternalResponse {
        content: response.content,
        provider: response.provider,
        model: response.model,
        prompt_tokens: response.usage.prompt_tokens,
        completion_tokens: response.usage.completion_tokens,
        total_tokens: response.usage.total_tokens,
    })
}

fn finish_external_failure(
    headers: &HeaderMap,
    audit_context: &LlmAuditContext<'_>,
    error: LlmError,
) -> AppError {
    let problem = map_llm_err(error);
    record_llm_http_audit(headers, audit_context, "error", &problem, None);
    tracing::warn!(
        route = audit_context.route,
        provider = audit_context.selection.name.as_str(),
        model = audit_context.selection.default_model.as_str(),
        payload_class = audit_context.context.as_str(),
        redacted_bytes = audit_context.redacted_bytes,
        problem_code = problem.code(),
        "llm outbound failed"
    );
    problem
}

fn prepare_external_prompt(
    state: &AppState,
    request: &LlmExternalRequest,
    expected_context: Option<LlmPromptContext>,
) -> Result<PreparedExternalPrompt, AppError> {
    let context = request.payload.context;
    ensure_context(context, expected_context)?;
    let outbound_json = request
        .payload
        .outbound_json()
        .map_err(|error| external_payload_error(&error))?;
    ensure_external_readiness(state)?;
    let provider = request.provider;
    let selection = state
        .llm_router()
        .selection(provider.map(LlmProviderId::as_str))
        .map_err(map_llm_err)?;
    Ok(PreparedExternalPrompt {
        provider,
        context,
        redacted_bytes: outbound_json.len(),
        outbound_json,
        selection,
    })
}

fn diagnostic_request(context: LlmPromptContext, outbound_json: String) -> ChatRequest {
    ChatRequest::new(vec![
        Message::system(instruction_for(context)),
        Message::user(outbound_json),
    ])
    .temperature(0.2)
    .max_tokens(DIAGNOSTIC_MAX_TOKENS)
}

fn ensure_context(
    actual: LlmPromptContext,
    expected: Option<LlmPromptContext>,
) -> Result<(), AppError> {
    if expected.is_none() || expected == Some(actual) {
        return Ok(());
    }
    let expected = expected.map(LlmPromptContext::as_str).unwrap_or("any");
    Err(AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::LLM_CONTEXT_MISMATCH,
        "LLM diagnostic payload context does not match this route",
    )
    .with_details(json!({
        "expectedContext": expected,
        "actualContext": actual.as_str(),
    })))
}

fn ensure_external_readiness(state: &AppState) -> Result<(), AppError> {
    ensure_llm_auth_ready(state.config().security.auth_required())?;
    let audit_health = audit::health_snapshot(common::time::now_ms());
    ensure_llm_audit_ready(
        audit_health.configured,
        audit_health.opened,
        audit_health.writer_alive,
    )
}

fn ensure_llm_auth_ready(auth_required: bool) -> Result<(), AppError> {
    if auth_required {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::SERVICE_UNAVAILABLE,
        codes::LLM_OUTBOUND_AUTH_REQUIRED,
        "LLM outbound diagnostics require bearer authentication",
    ))
}

fn ensure_llm_audit_ready(
    configured: bool,
    opened: bool,
    writer_alive: bool,
) -> Result<(), AppError> {
    if configured && opened && writer_alive {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::SERVICE_UNAVAILABLE,
        codes::LLM_AUDIT_WRITE_FAILED,
        "LLM outbound diagnostics require a writable audit sink",
    )
    .with_details(json!({
        "auditConfigured": configured,
        "auditOpened": opened,
        "auditWriterAlive": writer_alive,
    })))
}

fn instruction_for(context: LlmPromptContext) -> &'static str {
    match context {
        LlmPromptContext::OpportunityExplanation => {
            "Explain this funding-rate arbitrage opportunity in two concise paragraphs, then list execution risks. Treat the supplied payload as sanitized diagnostic context, not as trading instructions."
        }
        LlmPromptContext::FailureDiagnosis => {
            "Diagnose this sanitized order-state failure summary. Return likely root cause, immediate operator action, and prevention. Do not infer missing credentials, balances, or raw exchange data."
        }
        LlmPromptContext::DailyBrief => {
            "Write a concise funding-rate arbitrage brief from the sanitized summary. State uncertainty whenever execution or data health is not proven."
        }
    }
}

fn external_payload_error(error: &LlmExternalPayloadError) -> AppError {
    let status = if error.is_too_large() {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::BAD_REQUEST
    };
    let code = if error.is_too_large() {
        codes::LLM_EXTERNAL_PAYLOAD_TOO_LARGE
    } else {
        codes::LLM_EXTERNAL_PAYLOAD_INVALID
    };
    AppError::domain(
        status,
        code,
        "LLM external payload was rejected before outbound delivery",
    )
    .with_details(json!({ "reason": error.to_string() }))
}

struct ProviderFailure {
    provider: String,
    path: String,
    provider_status: Option<u16>,
    provider_request_id: Option<String>,
    retry_after_secs: Option<u64>,
    timeout_secs: Option<u64>,
}

impl ProviderFailure {
    fn response(
        provider: String,
        path: String,
        provider_status: u16,
        provider_request_id: Option<String>,
    ) -> Self {
        Self {
            provider,
            path,
            provider_status: Some(provider_status),
            provider_request_id,
            retry_after_secs: None,
            timeout_secs: None,
        }
    }

    fn rate_limited(
        provider: String,
        path: String,
        provider_request_id: Option<String>,
        retry_after_secs: Option<u64>,
    ) -> Self {
        Self {
            provider,
            path,
            provider_status: Some(StatusCode::TOO_MANY_REQUESTS.as_u16()),
            provider_request_id,
            retry_after_secs,
            timeout_secs: None,
        }
    }

    fn transport(provider: String, path: String) -> Self {
        Self {
            provider,
            path,
            provider_status: None,
            provider_request_id: None,
            retry_after_secs: None,
            timeout_secs: None,
        }
    }

    fn timeout(provider: String, path: String, timeout_secs: u64) -> Self {
        Self {
            provider,
            path,
            provider_status: None,
            provider_request_id: None,
            retry_after_secs: None,
            timeout_secs: Some(timeout_secs),
        }
    }

    fn invalid_response(
        provider: String,
        path: String,
        provider_request_id: Option<String>,
    ) -> Self {
        Self {
            provider,
            path,
            provider_status: None,
            provider_request_id,
            retry_after_secs: None,
            timeout_secs: None,
        }
    }
}

fn map_llm_err(error: LlmError) -> AppError {
    match error {
        LlmError::ProviderNotFound(provider) => AppError::domain(
            StatusCode::NOT_FOUND,
            codes::LLM_PROVIDER_UNAVAILABLE,
            "requested LLM provider is not configured",
        )
        .with_details(json!({ "provider": provider })),
        LlmError::Auth {
            provider,
            path,
            status,
            request_id,
        } => provider_problem(
            StatusCode::BAD_GATEWAY,
            codes::LLM_PROVIDER_AUTH_FAILED,
            &ProviderFailure::response(provider, path, status, request_id),
        ),
        LlmError::RateLimited {
            provider,
            path,
            retry_after_secs,
            request_id,
        } => provider_problem(
            StatusCode::TOO_MANY_REQUESTS,
            codes::LLM_PROVIDER_RATE_LIMITED,
            &ProviderFailure::rate_limited(provider, path, request_id, retry_after_secs),
        ),
        LlmError::Upstream {
            provider,
            path,
            status,
            request_id,
            ..
        } => provider_problem(
            StatusCode::BAD_GATEWAY,
            codes::LLM_PROVIDER_UPSTREAM_FAILED,
            &ProviderFailure::response(provider, path, status, request_id),
        ),
        LlmError::Network { provider, path, .. } => provider_problem(
            StatusCode::BAD_GATEWAY,
            codes::LLM_PROVIDER_NETWORK_FAILED,
            &ProviderFailure::transport(provider, path),
        ),
        LlmError::Timeout {
            provider,
            path,
            seconds,
        } => provider_problem(
            StatusCode::GATEWAY_TIMEOUT,
            codes::LLM_PROVIDER_TIMEOUT,
            &ProviderFailure::timeout(provider, path, seconds),
        ),
        LlmError::InvalidResponse {
            provider,
            path,
            request_id,
            ..
        } => provider_problem(
            StatusCode::BAD_GATEWAY,
            codes::LLM_PROVIDER_INVALID_RESPONSE,
            &ProviderFailure::invalid_response(provider, path, request_id),
        ),
        LlmError::NotImplemented(feature) => AppError::domain(
            StatusCode::NOT_IMPLEMENTED,
            codes::NOT_IMPLEMENTED,
            "requested LLM capability is not implemented",
        )
        .with_details(json!({ "feature": feature })),
    }
}

fn provider_problem(status: StatusCode, code: &'static str, failure: &ProviderFailure) -> AppError {
    AppError::domain(status, code, "LLM provider request failed").with_details(json!({
        "provider": failure.provider,
        "endpointPath": failure.path,
        "providerStatus": failure.provider_status,
        "providerRequestId": failure.provider_request_id,
        "retryAfterMs": failure.retry_after_secs.map(|seconds| seconds.saturating_mul(1_000)),
        "timeoutSecs": failure.timeout_secs,
    }))
}

fn record_llm_durable_audit(
    headers: &HeaderMap,
    context: &LlmAuditContext<'_>,
    outcome: &'static str,
) -> Result<(), AppError> {
    let actor = audit::extract_actor(headers);
    let event = audit::AuditEvent::now(
        actor.as_str(),
        "llm.outbound",
        context.route,
        outcome,
        llm_audit_detail(context, StatusCode::ACCEPTED.as_u16(), None, None),
    );
    audit::record_durable(&event).map_err(|reason| {
        tracing::error!(route = context.route, provider = context.selection.name.as_str(), %reason, "llm outbound audit write failed");
        AppError::domain(
            StatusCode::SERVICE_UNAVAILABLE,
            codes::LLM_AUDIT_WRITE_FAILED,
            "LLM outbound audit storage is unavailable",
        )
        .with_details(json!({
            "route": context.route,
            "provider": context.selection.name.as_str(),
            "payloadClass": context.context.as_str(),
        }))
    })
}

fn record_llm_http_audit(
    headers: &HeaderMap,
    context: &LlmAuditContext<'_>,
    outcome: &'static str,
    problem: &AppError,
    usage: Option<&Usage>,
) {
    let detail = llm_audit_detail(
        context,
        problem.status().as_u16(),
        Some(problem.code()),
        usage,
    );
    audit::record_http_event(headers, "llm.outbound", context.route, outcome, detail);
}

fn llm_audit_detail(
    context: &LlmAuditContext<'_>,
    status: u16,
    problem_code: Option<&str>,
    usage: Option<&Usage>,
) -> serde_json::Value {
    json!({
        "route": context.route,
        "provider": context.selection.name.as_str(),
        "model": context.selection.default_model.as_str(),
        "payloadClass": context.context.as_str(),
        "redactedBytes": context.redacted_bytes,
        "status": status,
        "tokens": usage.map(|usage| json!({
            "prompt": usage.prompt_tokens,
            "completion": usage.completion_tokens,
            "total": usage.total_tokens,
        })),
        "problemCode": problem_code,
        "providerEvidence": {
            "checkedAt": context.selection.evidence.checked_at,
            "endpointPath": context.selection.evidence.endpoint_path,
            "docUrl": context.selection.evidence.doc_url,
        },
    })
}

fn check_chat_rate(headers: &HeaderMap) -> Result<(), AppError> {
    let actor = audit::extract_actor(headers);
    allow_chat_request(&actor, common::time::now_ms())
}

fn allow_chat_request(actor: &str, now_ms: i64) -> Result<(), AppError> {
    let mut window = chat_rate().entry(actor.to_owned()).or_default();
    prune_window(&mut window, now_ms);
    if window.len() >= CHAT_RATE_LIMIT {
        return Err(AppError::RateLimited {
            retry_after_secs: retry_after_secs(&window, now_ms),
        });
    }
    window.push_back(now_ms);
    Ok(())
}

fn chat_rate() -> &'static DashMap<String, VecDeque<i64>> {
    CHAT_RATE.get_or_init(DashMap::new)
}

fn prune_window(window: &mut VecDeque<i64>, now_ms: i64) {
    let cutoff = now_ms.saturating_sub(CHAT_RATE_WINDOW_MS);
    while window.front().is_some_and(|ts| *ts <= cutoff) {
        window.pop_front();
    }
}

fn retry_after_secs(window: &VecDeque<i64>, now_ms: i64) -> u64 {
    window
        .front()
        .map(|first| first + CHAT_RATE_WINDOW_MS - now_ms)
        .map(|ms| ms.max(1))
        .map(|ms| ((ms + 999) / 1_000) as u64)
        .unwrap_or(1)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use llm::OPENAI_EVIDENCE;
    use shared_types::{LlmExternalPayload, LlmOrderState, LlmSanitizedOrderState};

    fn payload() -> LlmExternalPayload {
        LlmExternalPayload {
            context: LlmPromptContext::FailureDiagnosis,
            summary: "order rejected after an explicit preflight blocker".to_owned(),
            symbol: Some("BTCUSDT".to_owned()),
            venue: Some("okx".to_owned()),
            error_code: Some("HEDGE_TICKET_BLOCKED".to_owned()),
            order_states: vec![LlmSanitizedOrderState {
                venue: "okx".to_owned(),
                symbol: "BTCUSDT".to_owned(),
                state: LlmOrderState::Rejected,
            }],
        }
    }

    fn selection() -> ProviderSelection {
        ProviderSelection {
            name: "openai".to_owned(),
            default_model: "gpt-4o-mini".to_owned(),
            evidence: OPENAI_EVIDENCE,
        }
    }

    #[test]
    fn chat_rate_limit_rejects_after_window_quota() {
        let actor = format!("test-{}", uuid::Uuid::new_v4());
        let now_ms = 1_000_000;

        for _ in 0..CHAT_RATE_LIMIT {
            assert!(allow_chat_request(&actor, now_ms).is_ok());
        }

        assert!(matches!(
            allow_chat_request(&actor, now_ms),
            Err(AppError::RateLimited { .. })
        ));
        assert!(allow_chat_request(&actor, now_ms + CHAT_RATE_WINDOW_MS + 1).is_ok());
    }

    #[test]
    fn route_context_mismatch_fails_closed() {
        let error = ensure_context(
            LlmPromptContext::FailureDiagnosis,
            Some(LlmPromptContext::OpportunityExplanation),
        )
        .expect_err("mismatch must fail");

        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(error.code(), codes::LLM_CONTEXT_MISMATCH);
    }

    #[test]
    fn outbound_requires_auth_and_a_live_audit_writer() {
        let auth_error = ensure_llm_auth_ready(false).expect_err("auth is required");
        assert_eq!(auth_error.code(), codes::LLM_OUTBOUND_AUTH_REQUIRED);

        let audit_error =
            ensure_llm_audit_ready(true, false, false).expect_err("audit writer is required");
        assert_eq!(audit_error.code(), codes::LLM_AUDIT_WRITE_FAILED);
        assert!(ensure_llm_audit_ready(true, true, true).is_ok());
    }

    #[test]
    fn legacy_raw_chat_message_body_is_not_a_supported_contract() {
        let raw = r#"{"messages":[{"role":"user","content":"send account balance"}]}"#;

        assert!(serde_json::from_str::<LlmExternalRequest>(raw).is_err());
    }

    #[test]
    fn provider_rate_limit_keeps_provider_evidence_in_typed_problem() {
        let error = map_llm_err(LlmError::RateLimited {
            provider: "deepseek".to_owned(),
            path: "/chat/completions".to_owned(),
            retry_after_secs: Some(30),
            request_id: Some("provider-request-1".to_owned()),
        });

        assert_eq!(error.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.code(), codes::LLM_PROVIDER_RATE_LIMITED);
        let AppError::Domain {
            details: Some(details),
            ..
        } = error
        else {
            panic!("expected typed domain problem");
        };
        assert_eq!(details["provider"], "deepseek");
        assert_eq!(details["endpointPath"], "/chat/completions");
        assert_eq!(details["providerRequestId"], "provider-request-1");
        assert_eq!(details["retryAfterMs"], 30_000);
    }

    #[test]
    fn llm_audit_detail_never_contains_payload_summary() {
        let selection = selection();
        let audit_context = LlmAuditContext {
            route: "/api/llm/diagnose-failure",
            selection: &selection,
            context: payload().context,
            redacted_bytes: 321,
        };
        let detail = llm_audit_detail(&audit_context, StatusCode::ACCEPTED.as_u16(), None, None);
        let text = detail.to_string();

        assert!(text.contains("redactedBytes"));
        assert!(!text.contains("explicit preflight blocker"));
        assert!(!text.contains("HEDGE_TICKET_BLOCKED"));
    }
}
