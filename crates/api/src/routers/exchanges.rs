use crate::services::action_runs::{self, ActionRunStart};
use crate::services::venue_credentials::{self, CredentialUpdateError};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::AppError;
use serde::Deserialize;
use serde_json::json;
use shared_types::problem::codes;
use shared_types::{
    ActionRun, ActionRunKind, MarketDataEnvelope, OrderBookInfo, VenueCredentialClearRequest,
    VenueCredentialMaintenanceResponse, VenueCredentialMigrateRequest,
    VenueCredentialUpdateRequest, VenueCredentialUpdateResponse, VenueCredentialsResponse,
};

const HEADER_IDEMPOTENCY_KEY: &str = "idempotency-key";
const HEADER_X_IDEMPOTENCY_KEY: &str = "x-idempotency-key";
const CREDENTIAL_REPLAY_KEY: &[u8] = b"crossline-credential-idempotency-v1";

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/exchanges/credentials",
            get(credentials).post(update_credentials),
        )
        .route("/api/exchanges/credentials/clear", post(clear_credentials))
        .route(
            "/api/exchanges/credentials/migrate",
            post(migrate_credentials),
        )
        .route("/api/exchanges/:venue/orderbook", get(orderbook))
}

#[derive(Debug, Deserialize)]
struct OrderbookParams {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    depth: Option<u32>,
}

async fn credentials() -> Json<VenueCredentialsResponse> {
    Json(venue_credentials::status())
}

async fn update_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<VenueCredentialUpdateRequest>,
) -> Result<Json<VenueCredentialUpdateResponse>, AppError> {
    let venue = request.venue.clone();
    let key = credential_idempotency_key(&headers, &request);
    let begin = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::VenueCredentialsUpdate,
            &headers,
            Some(venue),
            "venue credentials update accepted",
        )
        .with_idempotency_key(Some(key)),
    )?;
    if begin.is_replayed() {
        return replay_credentials_response(begin.run());
    }
    let run = begin.run();
    let result = update_credentials_response(&state, request, run).await;
    action_runs::finish_result_with_payload(&state, &run.id, result, "venue credentials updated")
        .map(Json)
}

async fn update_credentials_response(
    state: &AppState,
    request: VenueCredentialUpdateRequest,
    run: &ActionRun,
) -> Result<VenueCredentialUpdateResponse, AppError> {
    let credentials_changed =
        venue_credentials::update_changes_saved_values(&request).map_err(map_credential_error)?;
    let mut response = venue_credentials::update(request)
        .await
        .map_err(map_credential_error)?;
    if credentials_changed {
        refresh_credential_runtime(state, &response.venue).await?;
    }
    response.action_run_id = Some(run.id.clone());
    response.request_id = run.request_id.clone();
    Ok(response)
}

async fn clear_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<VenueCredentialClearRequest>,
) -> Result<Json<VenueCredentialMaintenanceResponse>, AppError> {
    credential_maintenance(
        &state,
        headers,
        CredentialMaintenanceRequest::Clear(request),
    )
    .await
}

async fn migrate_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<VenueCredentialMigrateRequest>,
) -> Result<Json<VenueCredentialMaintenanceResponse>, AppError> {
    credential_maintenance(
        &state,
        headers,
        CredentialMaintenanceRequest::Migrate(request),
    )
    .await
}

enum CredentialMaintenanceRequest {
    Clear(VenueCredentialClearRequest),
    Migrate(VenueCredentialMigrateRequest),
}

impl CredentialMaintenanceRequest {
    fn venue(&self) -> &str {
        match self {
            Self::Clear(request) => &request.venue,
            Self::Migrate(request) => &request.venue,
        }
    }

    fn kind(&self) -> ActionRunKind {
        match self {
            Self::Clear(_) => ActionRunKind::VenueCredentialsClear,
            Self::Migrate(_) => ActionRunKind::VenueCredentialsMigrate,
        }
    }

    fn accepted_message(&self) -> &'static str {
        match self {
            Self::Clear(_) => "venue credentials clear accepted",
            Self::Migrate(_) => "venue credentials migration accepted",
        }
    }

    fn completed_message(&self) -> &'static str {
        match self {
            Self::Clear(_) => "venue credentials cleared",
            Self::Migrate(_) => "venue credentials migrated",
        }
    }

    fn canonical(&self) -> String {
        match self {
            Self::Clear(request) => {
                let mut fields = request
                    .fields
                    .iter()
                    .map(|field| field.trim())
                    .collect::<Vec<_>>();
                fields.sort_unstable();
                let mut text = format!(
                    "clear\n{}",
                    shared_types::normalized_venue_name(&request.venue)
                );
                for field in fields {
                    text.push('\n');
                    text.push_str(field);
                }
                text
            }
            Self::Migrate(request) => format!(
                "migrate\n{}",
                shared_types::normalized_venue_name(&request.venue)
            ),
        }
    }

    async fn execute(self) -> Result<VenueCredentialMaintenanceResponse, CredentialUpdateError> {
        match self {
            Self::Clear(request) => venue_credentials::clear(request).await,
            Self::Migrate(request) => venue_credentials::migrate(request).await,
        }
    }
}

async fn credential_maintenance(
    state: &AppState,
    headers: HeaderMap,
    request: CredentialMaintenanceRequest,
) -> Result<Json<VenueCredentialMaintenanceResponse>, AppError> {
    let venue = request.venue().to_owned();
    let completed_message = request.completed_message();
    let key = credential_maintenance_idempotency_key(&headers, &request);
    let begin = action_runs::begin_idempotent(
        state,
        ActionRunStart::new(
            request.kind(),
            &headers,
            Some(venue),
            request.accepted_message(),
        )
        .with_idempotency_key(Some(key)),
    )?;
    if begin.is_replayed() {
        return replay_credential_maintenance_response(begin.run());
    }
    let run = begin.run();
    let result = credential_maintenance_response(state, request, run).await;
    action_runs::finish_result_with_payload(state, &run.id, result, completed_message).map(Json)
}

async fn credential_maintenance_response(
    state: &AppState,
    request: CredentialMaintenanceRequest,
    run: &ActionRun,
) -> Result<VenueCredentialMaintenanceResponse, AppError> {
    let mut response = request.execute().await.map_err(map_credential_error)?;
    refresh_credential_runtime(state, &response.venue).await?;
    response.action_run_id = Some(run.id.clone());
    response.request_id = run.request_id.clone();
    Ok(response)
}

async fn refresh_credential_runtime(state: &AppState, venue: &str) -> Result<(), AppError> {
    let _mutation = state.trading_runtime_config_mutation_lock().lock().await;
    // Revoke before rebuilding: even a failed replacement must not keep ingesting old-account data.
    state.trading_service().invalidate_private_ws_account(venue);
    state.private_ws_health().invalidate_credentials_update(venue);
    state
        .live_order_proof_health()
        .invalidate_credentials_update(venue);
    state
        .run_finality_health()
        .invalidate_credentials_update(venue);
    crate::lifecycle::refresh_exchange(state, venue).map_err(AppError::BadRequest)
}

fn replay_credentials_response(
    run: &ActionRun,
) -> Result<Json<VenueCredentialUpdateResponse>, AppError> {
    match run.status {
        shared_types::ActionRunStatus::Succeeded => replay_succeeded_credentials(run).map(Json),
        shared_types::ActionRunStatus::Failed => Err(replay_failed_credentials(run)),
        shared_types::ActionRunStatus::Accepted => Err(replay_in_flight(run)),
    }
}

fn replay_credential_maintenance_response(
    run: &ActionRun,
) -> Result<Json<VenueCredentialMaintenanceResponse>, AppError> {
    match run.status {
        shared_types::ActionRunStatus::Succeeded => {
            replay_succeeded_credential_maintenance(run).map(Json)
        }
        shared_types::ActionRunStatus::Failed => Err(replay_failed_credentials(run)),
        shared_types::ActionRunStatus::Accepted => Err(replay_in_flight(run)),
    }
}

fn replay_succeeded_credentials(
    run: &ActionRun,
) -> Result<VenueCredentialUpdateResponse, AppError> {
    let mut response = action_runs::replay_payload::<VenueCredentialUpdateResponse>(run)?;
    response.action_run_id = Some(run.id.clone());
    response.request_id = run.request_id.clone();
    response.message = format!("{}；重复请求已回放", response.message);
    Ok(response)
}

fn replay_succeeded_credential_maintenance(
    run: &ActionRun,
) -> Result<VenueCredentialMaintenanceResponse, AppError> {
    let mut response = action_runs::replay_payload::<VenueCredentialMaintenanceResponse>(run)?;
    response.action_run_id = Some(run.id.clone());
    response.request_id = run.request_id.clone();
    response.message = format!("{}；重复请求已回放", response.message);
    Ok(response)
}

fn replay_failed_credentials(run: &ActionRun) -> AppError {
    if let Some(problem) = run.problem.as_ref() {
        if problem.code == codes::ACTION_RUN_REPLAY_UNAVAILABLE {
            let status = problem
                .status
                .and_then(|status| StatusCode::from_u16(status).ok())
                .unwrap_or(StatusCode::CONFLICT);
            return AppError::domain(
                status,
                codes::ACTION_RUN_REPLAY_UNAVAILABLE,
                problem.message.clone(),
            )
            .with_details(json!({
                "actionRunId": run.id,
                "requestId": run.request_id,
                "idempotencyKey": run.idempotency_key,
                "replayed": true,
                "originalProblem": problem,
            }));
        }
    }
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_FAILED,
        "credential mutation idempotency key already failed",
    )
    .with_details(json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": run.idempotency_key,
        "problem": run.problem,
    }))
}

fn replay_in_flight(run: &ActionRun) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_IN_FLIGHT,
        "credential mutation idempotency key is already in flight",
    )
    .with_details(json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": run.idempotency_key,
    }))
}

fn credential_idempotency_key(
    headers: &HeaderMap,
    request: &VenueCredentialUpdateRequest,
) -> String {
    explicit_idempotency_key(headers).unwrap_or_else(|| derived_credential_idempotency_key(request))
}

fn explicit_idempotency_key(headers: &HeaderMap) -> Option<String> {
    header_text(headers, HEADER_IDEMPOTENCY_KEY)
        .or_else(|| header_text(headers, HEADER_X_IDEMPOTENCY_KEY))
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn derived_credential_idempotency_key(request: &VenueCredentialUpdateRequest) -> String {
    let canonical = canonical_credential_update(request);
    let digest = common::signing::hmac_sha256_hex(CREDENTIAL_REPLAY_KEY, canonical.as_bytes());
    let venue = shared_types::normalized_venue_name(&request.venue);
    format!("venue-credentials:{venue}:{}", &digest[..16])
}

fn credential_maintenance_idempotency_key(
    headers: &HeaderMap,
    request: &CredentialMaintenanceRequest,
) -> String {
    explicit_idempotency_key(headers).unwrap_or_else(|| {
        let canonical = request.canonical();
        let digest = common::signing::hmac_sha256_hex(CREDENTIAL_REPLAY_KEY, canonical.as_bytes());
        let operation = match request {
            CredentialMaintenanceRequest::Clear(_) => "clear",
            CredentialMaintenanceRequest::Migrate(_) => "migrate",
        };
        format!(
            "venue-credentials-{operation}:{}:{}",
            shared_types::normalized_venue_name(request.venue()),
            &digest[..16]
        )
    })
}

fn canonical_credential_update(request: &VenueCredentialUpdateRequest) -> String {
    let mut fields = request
        .fields
        .iter()
        .map(|field| (field.key.trim(), field.value.trim()))
        .collect::<Vec<_>>();
    fields.sort_unstable_by(|left, right| left.0.cmp(right.0).then_with(|| left.1.cmp(right.1)));
    let mut text = shared_types::normalized_venue_name(&request.venue);
    for (key, value) in fields {
        text.push('\n');
        text.push_str(key);
        text.push('=');
        text.push_str(value);
    }
    text
}

async fn orderbook(
    State(state): State<AppState>,
    Path(venue): Path<String>,
    Query(params): Query<OrderbookParams>,
) -> Result<Json<MarketDataEnvelope<Option<OrderBookInfo>>>, AppError> {
    let symbol = required_orderbook_symbol(&params)?;
    let now_ms = common::time::now_ms();
    let depth = params.depth.unwrap_or(5).clamp(1, 100);
    let read = state
        .market_data()
        .refresh_orderbook_from_ws(state.aggregator(), &venue, &symbol, depth, now_ms)
        .await;
    Ok(Json(
        crate::services::market_data::envelope::orderbook_envelope(
            read,
            &venue,
            &symbol,
            depth as usize,
            now_ms,
        ),
    ))
}

fn required_orderbook_symbol(params: &OrderbookParams) -> Result<String, AppError> {
    params
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| AppError::BadRequest("symbol is required".into()))
}

fn map_credential_error(error: CredentialUpdateError) -> AppError {
    let message = error.to_string();
    match error {
        CredentialUpdateError::UnknownVenue(venue) => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::CREDENTIAL_UNKNOWN_VENUE,
            message,
        )
        .with_details(json!({ "venue": venue })),
        CredentialUpdateError::UnknownField { venue, field } => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::CREDENTIAL_UNKNOWN_FIELD,
            message,
        )
        .with_details(json!({ "venue": venue, "field": field })),
        CredentialUpdateError::MissingField { venue, field } => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::CREDENTIAL_MISSING_FIELD,
            message,
        )
        .with_details(json!({ "venue": venue, "field": field })),
        CredentialUpdateError::ValidationTimeout => AppError::domain(
            StatusCode::GATEWAY_TIMEOUT,
            codes::CREDENTIAL_VALIDATION_TIMEOUT,
            message,
        ),
        CredentialUpdateError::PermissionDenied(_) => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::CREDENTIAL_PERMISSION_DENIED,
            message,
        ),
        CredentialUpdateError::Validation(_) => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::CREDENTIAL_VALIDATION_FAILED,
            message,
        ),
        CredentialUpdateError::NoFields => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::CREDENTIAL_NO_FIELDS,
            message,
        ),
        CredentialUpdateError::MigrationUnavailable => AppError::domain(
            StatusCode::CONFLICT,
            codes::CREDENTIAL_MIGRATION_UNAVAILABLE,
            message,
        ),
        CredentialUpdateError::Persist(_) | CredentialUpdateError::SecretBackend(_) => {
            AppError::domain(
                StatusCode::INTERNAL_SERVER_ERROR,
                codes::CREDENTIAL_PERSIST_FAILED,
                message,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;

    #[test]
    fn credential_errors_map_to_distinct_codes_and_status() {
        let cases = [
            (
                CredentialUpdateError::UnknownVenue("foo".into()),
                "CREDENTIAL_UNKNOWN_VENUE",
                400,
            ),
            (
                CredentialUpdateError::UnknownField {
                    venue: "binance".into(),
                    field: "x".into(),
                },
                "CREDENTIAL_UNKNOWN_FIELD",
                400,
            ),
            (
                CredentialUpdateError::ValidationTimeout,
                "CREDENTIAL_VALIDATION_TIMEOUT",
                504,
            ),
            (
                CredentialUpdateError::PermissionDenied("auth failed".into()),
                "CREDENTIAL_PERMISSION_DENIED",
                400,
            ),
            (
                CredentialUpdateError::Validation("bad".into()),
                "CREDENTIAL_VALIDATION_FAILED",
                400,
            ),
            (CredentialUpdateError::NoFields, "CREDENTIAL_NO_FIELDS", 400),
            (
                CredentialUpdateError::MigrationUnavailable,
                "CREDENTIAL_MIGRATION_UNAVAILABLE",
                409,
            ),
        ];
        for (error, code, status) in cases {
            let mapped = map_credential_error(error);
            assert_eq!(mapped.code(), code);
            assert_eq!(mapped.status().as_u16(), status);
        }
    }

    #[test]
    fn persist_failure_maps_to_server_error_code() {
        let error = CredentialUpdateError::Persist(std::io::Error::other("disk full"));
        let mapped = map_credential_error(error);
        assert_eq!(mapped.code(), "CREDENTIAL_PERSIST_FAILED");
        assert_eq!(mapped.status().as_u16(), 500);
    }

    #[test]
    fn orderbook_missing_symbol_uses_typed_bad_request() {
        let result = required_orderbook_symbol(&OrderbookParams {
            symbol: None,
            depth: None,
        });
        let error = match result {
            Err(error) => error,
            Ok(symbol) => panic!("missing symbol unexpectedly resolved as {symbol}"),
        };

        assert_eq!(error.code(), "BAD_REQUEST");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn missing_field_carries_venue_and_field_details() {
        let mapped = map_credential_error(CredentialUpdateError::MissingField {
            venue: "okx".into(),
            field: "passphrase".into(),
        });
        let AppError::Domain {
            details: Some(details),
            ..
        } = mapped
        else {
            panic!("expected domain error");
        };
        assert_eq!(details["venue"], "okx");
        assert_eq!(details["field"], "passphrase");
    }

    #[test]
    fn credential_idempotency_key_is_stable_and_hides_values() {
        let headers = HeaderMap::new();
        let request = VenueCredentialUpdateRequest {
            venue: " OKX ".into(),
            fields: vec![
                shared_types::VenueCredentialValue {
                    key: "api_secret".into(),
                    value: "secret-value".into(),
                },
                shared_types::VenueCredentialValue {
                    key: "api_key".into(),
                    value: "key-value".into(),
                },
            ],
        };
        let same = VenueCredentialUpdateRequest {
            venue: "okx".into(),
            fields: request.fields.iter().rev().cloned().collect(),
        };

        let key = credential_idempotency_key(&headers, &request);

        assert_eq!(key, credential_idempotency_key(&headers, &same));
        assert!(key.starts_with("venue-credentials:okx:"));
        assert!(!key.contains("secret-value"));
        assert!(!key.contains("key-value"));
    }

    #[test]
    fn explicit_idempotency_header_wins_for_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HEADER_IDEMPOTENCY_KEY,
            axum::http::HeaderValue::from_static("client-save-1"),
        );
        let request = VenueCredentialUpdateRequest {
            venue: "okx".into(),
            fields: Vec::new(),
        };

        assert_eq!(
            credential_idempotency_key(&headers, &request),
            "client-save-1"
        );
    }

    #[test]
    fn credential_maintenance_idempotency_is_stable_and_never_uses_field_values() {
        let headers = HeaderMap::new();
        let clear = CredentialMaintenanceRequest::Clear(VenueCredentialClearRequest {
            venue: " OKX ".into(),
            fields: vec!["api_secret".into(), "api_key".into()],
        });
        let same_clear = CredentialMaintenanceRequest::Clear(VenueCredentialClearRequest {
            venue: "okx".into(),
            fields: vec!["api_key".into(), "api_secret".into()],
        });
        let migrate = CredentialMaintenanceRequest::Migrate(VenueCredentialMigrateRequest {
            venue: "okx".into(),
        });

        let clear_key = credential_maintenance_idempotency_key(&headers, &clear);

        assert_eq!(
            clear_key,
            credential_maintenance_idempotency_key(&headers, &same_clear)
        );
        assert_ne!(
            clear_key,
            credential_maintenance_idempotency_key(&headers, &migrate)
        );
        assert!(clear_key.starts_with("venue-credentials-clear:okx:"));
        assert!(!clear_key.contains("api_key"));
        assert!(!clear_key.contains("api_secret"));
    }

    #[test]
    fn replay_succeeded_credentials_uses_stored_payload() {
        let run = ActionRun {
            id: "act-cred".into(),
            kind: ActionRunKind::VenueCredentialsUpdate,
            status: shared_types::ActionRunStatus::Succeeded,
            actor: "test".into(),
            target: Some("okx".into()),
            request_id: Some("req-cred".into()),
            idempotency_key: Some("idem-cred".into()),
            message: "done".into(),
            problem: None,
            result: Some(json!({
                "venue": "okx",
                "label": "OKX",
                "configuredCount": 3,
                "fieldCount": 3,
                "message": "已保存 3 个字段",
                "actionRunId": "act-cred",
                "requestId": "req-cred"
            })),
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };

        let response = match replay_succeeded_credentials(&run) {
            Ok(response) => response,
            Err(error) => panic!("replayed response failed: {error}"),
        };

        assert_eq!(response.venue, "okx");
        assert_eq!(response.action_run_id.as_deref(), Some("act-cred"));
        assert_eq!(response.request_id.as_deref(), Some("req-cred"));
        assert!(response.message.contains("重复请求已回放"));
    }

    #[tokio::test]
    async fn update_credentials_replays_same_request_without_new_action_run() {
        let state = test_state().await;
        let request = VenueCredentialUpdateRequest {
            venue: "binance".into(),
            fields: vec![
                shared_types::VenueCredentialValue {
                    key: "api_key".into(),
                    value: "key-1".into(),
                },
                shared_types::VenueCredentialValue {
                    key: "api_secret".into(),
                    value: "secret-1".into(),
                },
            ],
        };
        let first = credential_update_result(&state, request.clone()).await;
        let second = credential_update_result(&state, request).await;

        assert_eq!(state.action_runs().len(), 1);
        assert_eq!(first.action_run_id, second.action_run_id);
        assert!(second.message.contains("重复请求已回放"));
    }

    #[tokio::test]
    async fn update_credentials_invalidates_existing_live_order_proof() {
        let state = test_state().await;
        record_live_order_proof(&state, "binance", "internal-1", 1_000).await;
        state.private_ws_health().record_connected("binance");
        state.private_ws_health().record_connected("okx");
        assert_eq!(
            state.live_order_proof_health().snapshot(1_200)[0].status,
            shared_types::VenueOperationStatus::Ok
        );

        let request = VenueCredentialUpdateRequest {
            venue: "binance".into(),
            fields: vec![
                shared_types::VenueCredentialValue {
                    key: "api_key".into(),
                    value: "key-rotated".into(),
                },
                shared_types::VenueCredentialValue {
                    key: "api_secret".into(),
                    value: "secret-rotated".into(),
                },
            ],
        };

        credential_update_result(&state, request).await;

        assert!(state.live_order_proof_health().snapshot(1_300).is_empty());
        let health = state.private_ws_health().snapshot(common::time::now_ms());
        assert_eq!(health.iter().filter(|row| row.venue == "binance").count(), 4);
        assert!(health.iter().filter(|row| row.venue == "binance")
            .all(|row| row.status == shared_types::VenueOperationStatus::Unknown));
        assert!(health.iter().any(|row| row.venue == "okx" && row.status == shared_types::VenueOperationStatus::Ok));
    }

    #[tokio::test]
    async fn update_credentials_invalidates_existing_run_finality_proof() {
        let state = test_state().await;
        record_run_finality_proof(&state, &["binance"]);
        assert_eq!(
            state.run_finality_health().snapshot(1_200)[0].status,
            shared_types::VenueOperationStatus::Ok
        );

        let request = VenueCredentialUpdateRequest {
            venue: "binance".into(),
            fields: vec![
                shared_types::VenueCredentialValue {
                    key: "api_key".into(),
                    value: "key-finality-rotated".into(),
                },
                shared_types::VenueCredentialValue {
                    key: "api_secret".into(),
                    value: "secret-finality-rotated".into(),
                },
            ],
        };

        credential_update_result(&state, request).await;

        assert!(state.run_finality_health().snapshot(1_300).is_empty());
    }

    #[tokio::test]
    async fn update_credentials_invalidates_hyperliquid_family_runtime_proofs() {
        let state = test_state().await;
        record_live_order_proof(&state, "hyperliquid:xyz", "internal-hl-xyz", 1_000).await;
        record_live_order_proof(&state, "hyperliquid:km", "internal-hl-km", 1_100).await;
        record_live_order_proof(&state, "binance", "internal-binance", 1_200).await;
        record_run_finality_proof(&state, &["hyperliquid:xyz", "hyperliquid:km", "binance"]);

        let response = credential_update_result(
            &state,
            VenueCredentialUpdateRequest {
                venue: " Hyperliquid:XYZ ".into(),
                fields: vec![shared_types::VenueCredentialValue {
                    key: "account_address".into(),
                    value: unique_hyperliquid_test_address(),
                }],
            },
        )
        .await;

        assert_eq!(response.venue, "hyperliquid");
        assert_eq!(
            snapshot_venues(state.live_order_proof_health().snapshot(1_300)),
            vec!["binance"]
        );
        assert_eq!(
            finality_snapshot_venues(state.run_finality_health().snapshot(1_300)),
            vec!["binance"]
        );
    }

    #[tokio::test]
    async fn credential_update_replay_does_not_reinvalidate_runtime_proofs() {
        let state = test_state().await;
        let initial_account_epoch = state.trading_service().account_cache_epoch();
        let request = VenueCredentialUpdateRequest {
            venue: "binance".into(),
            fields: vec![
                shared_types::VenueCredentialValue {
                    key: "api_key".into(),
                    value: "key-replay".into(),
                },
                shared_types::VenueCredentialValue {
                    key: "api_secret".into(),
                    value: "secret-replay".into(),
                },
            ],
        };
        let first = credential_update_result(&state, request.clone()).await;
        let updated_account_epoch = state.trading_service().account_cache_epoch();
        assert_eq!(updated_account_epoch, initial_account_epoch + 1);
        record_live_order_proof(&state, "binance", "internal-replay", 1_000).await;
        record_run_finality_proof(&state, &["binance"]);

        let replayed = credential_update_result(&state, request).await;

        assert_eq!(first.action_run_id, replayed.action_run_id);
        assert!(replayed.message.contains("重复请求已回放"));
        assert_eq!(
            state.trading_service().account_cache_epoch(),
            updated_account_epoch
        );
        assert_eq!(
            snapshot_venues(state.live_order_proof_health().snapshot(1_300)),
            vec!["binance"]
        );
        assert_eq!(
            finality_snapshot_venues(state.run_finality_health().snapshot(1_300)),
            vec!["binance"]
        );
    }

    #[tokio::test]
    async fn clear_credentials_replays_the_same_action_run() {
        let state = test_state().await;
        let request = VenueCredentialClearRequest {
            venue: "kucoin".into(),
            fields: vec!["api_key".into()],
        };

        let first = credential_clear_result(&state, request.clone()).await;
        let second = credential_clear_result(&state, request).await;

        assert_eq!(state.action_runs().len(), 1);
        assert_eq!(first.action_run_id, second.action_run_id);
        assert_eq!(
            first.operation,
            shared_types::VenueCredentialMaintenanceOperation::Clear
        );
        assert!(second.message.contains("重复请求已回放"));
        let run_id = first
            .action_run_id
            .unwrap_or_else(|| panic!("missing action run id"));
        let kind = state
            .action_runs()
            .get(&run_id)
            .map(|run| run.kind)
            .unwrap_or_else(|| panic!("missing action run"));
        assert_eq!(kind, ActionRunKind::VenueCredentialsClear);
    }

    async fn credential_update_result(
        state: &AppState,
        request: VenueCredentialUpdateRequest,
    ) -> VenueCredentialUpdateResponse {
        match update_credentials(State(state.clone()), HeaderMap::new(), Json(request)).await {
            Ok(Json(response)) => response,
            Err(error) => panic!("credential update failed: {error}"),
        }
    }

    async fn credential_clear_result(
        state: &AppState,
        request: VenueCredentialClearRequest,
    ) -> VenueCredentialMaintenanceResponse {
        match clear_credentials(State(state.clone()), HeaderMap::new(), Json(request)).await {
            Ok(Json(response)) => response,
            Err(error) => panic!("credential clear failed: {error}"),
        }
    }

    async fn test_state() -> AppState {
        let mut config = common::config::AppConfig::default();
        config.history.enabled = false;
        config.storage.portfolio_nav_path = None;
        match AppState::new(config).await {
            Ok(state) => state,
            Err(error) => panic!("state init failed: {error}"),
        }
    }

    async fn record_live_order_proof(
        state: &AppState,
        venue: &str,
        internal_order_id: &str,
        checked_at_ms: i64,
    ) {
        if crate::trading_service::TradingService::configured_account_scope(venue, shared_types::FeeProduct::Perp).is_none() {
            let fields = if venue.starts_with("hyperliquid") {
                vec![("HYPERLIQUID_ACCOUNT_ADDRESS".into(), "0x1111111111111111111111111111111111111111".into()),
                     ("HYPERLIQUID_PRIVATE_KEY".into(), "11".repeat(32))]
            } else {
                vec![("BINANCE_API_KEY".into(), "isolated-proof-key".into()),
                     ("BINANCE_API_SECRET".into(), "isolated-proof-secret".into())]
            };
            // The credential storage's test implementation is memory-only.
            crate::services::venue_credentials::persist_secrets(&fields).await.unwrap();
        }
        state
            .live_order_proof_health()
            .record_place_ack(live_order_sample(venue, internal_order_id, checked_at_ms));
        state
            .live_order_proof_health()
            .record_cancel_finality(live_order_sample(
                venue,
                internal_order_id,
                checked_at_ms + 100,
            ));
    }

    fn record_run_finality_proof(state: &AppState, venues: &[&str]) {
        let mut outcome = crate::services::run_finality::RunFinalityOutcome::default();
        for venue in venues {
            outcome.venue_outcomes.insert(
                (*venue).to_owned(),
                crate::services::run_finality::RunFinalityVenueOutcome {
                    scanned_order_count: 1,
                    refreshed_order_count: 1,
                    ..crate::services::run_finality::RunFinalityVenueOutcome::default()
                },
            );
        }
        state.run_finality_health().record_outcome(&outcome);
    }

    fn snapshot_venues(
        rows: Vec<crate::services::live_order_proof_health::LiveOrderProofRuntimeHealth>,
    ) -> Vec<String> {
        sorted_normalized_venues(rows.into_iter().map(|row| row.venue))
    }

    fn finality_snapshot_venues(
        rows: Vec<crate::services::run_finality_health::RunFinalityRuntimeHealth>,
    ) -> Vec<String> {
        sorted_normalized_venues(rows.into_iter().map(|row| row.venue))
    }

    fn sorted_normalized_venues(venues: impl Iterator<Item = String>) -> Vec<String> {
        let mut venues = venues
            .map(|venue| shared_types::normalized_venue_name(&venue))
            .collect::<Vec<_>>();
        venues.sort_unstable();
        venues
    }

    fn unique_hyperliquid_test_address() -> String {
        let seed = format!("{}:{}", common::time::now_ms(), std::process::id());
        let digest = common::signing::hmac_sha256_hex(b"hyperliquid-test-address", seed.as_bytes());
        format!("0x{}", &digest[..40])
    }

    fn live_order_sample(
        venue: &str,
        internal_order_id: &str,
        checked_at_ms: i64,
    ) -> crate::services::live_order_proof_health::LiveOrderProofSample {
        crate::services::live_order_proof_health::LiveOrderProofSample {
            venue: venue.to_owned(),
            account_scope: crate::trading_service::TradingService::configured_account_scope(venue, shared_types::FeeProduct::Perp),
            product: shared_types::FeeProduct::Perp,
            symbol: "BTCUSDT".to_owned(),
            internal_order_id: internal_order_id.to_owned(),
            exchange_order_id: Some(format!("exchange-{internal_order_id}")),
            client_order_id: Some(format!("client-{internal_order_id}")),
            source: "test".to_owned(),
            checked_at_ms,
            request_id: Some("req-proof".to_owned()),
            native_transport: None,
            native_request_id: None,
            native_response_id: None,
        }
    }
}
