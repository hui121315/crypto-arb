use crate::services::{
    action_runs::{self, ActionRunStart},
    market_data_diagnostics, system_health, venue_operation_health, venue_operation_scope,
};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde::Deserialize;
use shared_types::{
    ActionRunKind, GateCrossExModeConfigPatch, GateCrossExModeSnapshot, GateCrossExProduct,
    GateCrossExRouteCatalogResponse, MarketDataDiagnosticsSnapshot, MarketSubscriptionPatch,
    MarketSubscriptionsResponse, SystemHealthEnvelope, VenueOperationHealthSnapshot,
    VenueRuntimeHealthSnapshot,
};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/system/health", get(snapshot))
        .route(
            "/api/system/venue-operation-health",
            get(venue_operation_snapshot),
        )
        .route(
            "/api/system/venue-runtime-health",
            get(venue_runtime_health_snapshot),
        )
        .route(
            "/api/system/market-data-diagnostics",
            get(market_data_diagnostics_snapshot),
        )
        .route(
            "/api/system/market-subscriptions",
            get(market_subscriptions_snapshot),
        )
        .route(
            "/api/system/market-subscriptions/config",
            patch(update_market_subscription),
        )
        .route("/api/system/gate-crossex", get(gate_crossex_snapshot))
        .route("/api/system/gate-crossex/routes", get(gate_crossex_routes))
        .route(
            "/api/system/gate-crossex/config",
            patch(update_gate_crossex_config),
        )
}

/// 状态热路径只读 lifecycle updater 5s 重建的缓存快照，不在请求内触发计算或 I/O。
async fn snapshot(State(state): State<AppState>) -> Json<SystemHealthEnvelope> {
    if let Some(health) = state.system_health_snapshot().value_now() {
        return Json(system_health::envelope(health));
    }
    Json(system_health::warming_envelope(common::time::now_ms()))
}

async fn venue_operation_snapshot(
    State(state): State<AppState>,
    Query(query): Query<VenueOperationQuery>,
) -> Json<VenueOperationHealthSnapshot> {
    let snapshot = venue_operation_health::snapshot(&state);
    Json(venue_operation_scope::filter_snapshot(
        snapshot,
        query.venue.as_deref(),
    ))
}

async fn venue_runtime_health_snapshot(
    State(state): State<AppState>,
    Query(query): Query<VenueOperationQuery>,
) -> Json<VenueRuntimeHealthSnapshot> {
    let snapshot = venue_operation_health::snapshot(&state);
    let snapshot = venue_operation_scope::filter_snapshot(snapshot, query.venue.as_deref());
    Json(VenueRuntimeHealthSnapshot::from_operation_rows(
        &snapshot.rows,
        snapshot.generated_at_ms,
    ))
}

#[derive(Debug, Deserialize)]
struct VenueOperationQuery {
    venue: Option<String>,
}

async fn market_data_diagnostics_snapshot(
    State(state): State<AppState>,
) -> Json<MarketDataDiagnosticsSnapshot> {
    Json(market_data_diagnostics::snapshot(&state))
}

async fn market_subscriptions_snapshot(
    State(state): State<AppState>,
) -> Result<Json<MarketSubscriptionsResponse>, common::AppError> {
    state.market_subscriptions().ensure_restored().map_err(market_subscription_error)?;
    let health = state.market_data().runtime_health_snapshot();
    Ok(Json(
        state
            .market_subscriptions()
            .snapshot_with_runtime(state.aggregator().names(), &health),
    ))
}

async fn gate_crossex_snapshot(State(state): State<AppState>) -> Json<GateCrossExModeSnapshot> {
    Json((*state.gate_crossex_mode().snapshot()).clone())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GateCrossExRouteQuery {
    search: Option<String>,
    product: Option<GateCrossExProduct>,
    underlying_venue: Option<String>,
    limit: Option<usize>,
}

async fn gate_crossex_routes(
    State(state): State<AppState>,
    Query(query): Query<GateCrossExRouteQuery>,
) -> Json<GateCrossExRouteCatalogResponse> {
    Json(state.gate_crossex_mode().catalog(
        state.instrument_registry(),
        query.search.as_deref(),
        query.product,
        query.underlying_venue.as_deref(),
        query.limit,
    ))
}

async fn update_gate_crossex_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<GateCrossExModeConfigPatch>,
) -> Result<Json<GateCrossExModeSnapshot>, common::AppError> {
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::GateCrossExModeUpdate,
            &headers,
            Some("gate_crossex".to_owned()),
            "Gate CrossEx mode update accepted",
        )
        .with_idempotency_key(action_runs::explicit_idempotency_key(&headers)),
    )?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let _guard = state.gate_crossex_mode_mutation_lock().lock().await;
    let result = state
        .gate_crossex_mode()
        .update(patch, state.instrument_registry())
        .map_err(gate_crossex_error)
        .map(|_| (*state.gate_crossex_mode().snapshot()).clone());
    action_runs::finish_result_with_payload(
        &state,
        &claim.run().id,
        result,
        "Gate CrossEx mode updated",
    )
    .map(Json)
}

async fn update_market_subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<MarketSubscriptionPatch>,
) -> Result<Json<MarketSubscriptionsResponse>, common::AppError> {
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::MarketSubscriptionsUpdate,
            &headers,
            Some(patch.venue.trim().to_ascii_lowercase()),
            "market subscription update accepted",
        )
        .with_idempotency_key(action_runs::explicit_idempotency_key(&headers)),
    )?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let _guard = state.market_subscription_mutation_lock().lock().await;
    let result = state
        .market_subscriptions()
        .update(&patch)
        .map_err(market_subscription_error)
        .map(|updated| {
            state.market_data().apply_market_subscription(&updated);
            state.request_arbitrage_refresh();
            let health = state.market_data().runtime_health_snapshot();
            state
                .market_subscriptions()
                .snapshot_with_runtime(state.aggregator().names(), &health)
        });
    action_runs::finish_result_with_payload(
        &state,
        &claim.run().id,
        result,
        "market subscription updated",
    )
    .map(Json)
}

fn market_subscription_error(
    error: crate::services::market_subscriptions::MarketSubscriptionsError,
) -> common::AppError {
    match error {
        crate::services::market_subscriptions::MarketSubscriptionsError::EmptyVenue
        | crate::services::market_subscriptions::MarketSubscriptionsError::UnsupportedVenue(_) => {
            common::AppError::BadRequest(error.to_string())
        }
        crate::services::market_subscriptions::MarketSubscriptionsError::RestoreBlocked => {
            common::AppError::domain(axum::http::StatusCode::SERVICE_UNAVAILABLE,
                shared_types::problem::codes::MARKET_SUBSCRIPTION_RESTORE_FAILED,
                error.to_string())
                .with_details(serde_json::json!({ "source": "market_subscription_store",
                    "phase": "restore", "subscriptionsBlocked": true, "originalFilePreserved": true }))
        }
        other => {
            tracing::error!(%other, "market subscription persistence failed");
            common::AppError::domain(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                shared_types::problem::codes::MARKET_SUBSCRIPTION_STORAGE_FAILED,
                "行情订阅保存失败，当前订阅未改变；请检查存储并核验原操作",
            ).with_details(serde_json::json!({
                "source": "market_subscription_store", "runtimeApplied": false,
                "persistence": "unconfirmed",
            }))
        }
    }
}

fn gate_crossex_error(
    error: crate::services::gate_crossex_mode::GateCrossExModeError,
) -> common::AppError {
    match error {
        crate::services::gate_crossex_mode::GateCrossExModeError::Validation(_) => {
            common::AppError::BadRequest(error.to_string())
        }
        other => common::AppError::Config(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::build_router;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use common::config::AppConfig;
    use serde_json::Value;
    use shared_types::{normalized_venue_name, venue_family, ResourceStatus};
    use tower::ServiceExt;

    const PATH: &str = "/api/system/venue-runtime-health";
    const HEALTH_PATH: &str = "/api/system/health";
    const MARKET_SUBSCRIPTIONS_PATH: &str = "/api/system/market-subscriptions";
    const MARKET_SUBSCRIPTIONS_CONFIG_PATH: &str = "/api/system/market-subscriptions/config";
    const GATE_CROSSEX_PATH: &str = "/api/system/gate-crossex";
    const GATE_CROSSEX_CONFIG_PATH: &str = "/api/system/gate-crossex/config";
    const TOKEN: &str = "venue-runtime-health-test-token";

    #[tokio::test]
    async fn api_returns_authenticated_all_venue_projection() {
        let state = test_state().await;
        let router = build_router(state);

        let unauthorized = response(router.clone(), request(PATH, None)).await;
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let snapshot = api_snapshot(router, PATH).await;
        assert!(snapshot.venue_count > 1);
        assert_eq!(snapshot.venue_count, snapshot.venues.len());
        assert!(snapshot
            .venues
            .windows(2)
            .all(|pair| pair[0].venue < pair[1].venue));
    }

    #[tokio::test]
    async fn system_health_route_returns_warming_envelope_before_lifecycle_publish() {
        let router = build_router(test_state().await);
        let envelope = api_health_envelope(router).await;

        assert_eq!(envelope.status, ResourceStatus::Warming);
        assert_eq!(envelope.source, "system-health-snapshot");
        assert!(envelope.data.is_none());
        assert_eq!(
            envelope
                .problems
                .first()
                .map(|problem| problem.code.as_str()),
            Some(shared_types::problem::codes::SYSTEM_HEALTH_SNAPSHOT_WARMING)
        );
    }

    #[tokio::test]
    async fn api_normalizes_optional_venue_filter() {
        let router = build_router(test_state().await);
        let all = api_snapshot(router.clone(), PATH).await;
        let selected = all
            .venues
            .iter()
            .find(|health| !health.venue.contains(':'))
            .map(|health| health.venue.clone())
            .unwrap_or_else(|| fail("all-venue projection has no family venue"));
        let query = format!("{PATH}?venue=%20{}%20", selected.to_ascii_uppercase());

        let filtered = api_snapshot(router, &query).await;
        let expected = all
            .venues
            .into_iter()
            .filter(|health| normalized_venue_name(venue_family(&health.venue)) == selected)
            .collect::<Vec<_>>();

        assert!(!expected.is_empty());
        assert_eq!(filtered.venue_count, filtered.venues.len());
        let operation_count = filtered.operation_count;
        let currently_usable_count = filtered.currently_usable_count;
        let attention_count = filtered.attention_count;
        assert_eq!(
            stable_payload(filtered),
            stable_payload(VenueRuntimeHealthSnapshot {
                venue_count: expected.len(),
                venues: expected,
                generated_at_ms: 0,
                operation_count,
                currently_usable_count,
                attention_count,
            })
        );
    }

    #[tokio::test]
    async fn api_projects_existing_operation_snapshot_without_external_probes() {
        let state = test_state().await;
        let operation_snapshot = venue_operation_health::snapshot(&state);
        let expected = VenueRuntimeHealthSnapshot::from_operation_rows(
            &operation_snapshot.rows,
            operation_snapshot.generated_at_ms,
        );
        let actual = api_snapshot(build_router(state), PATH).await;

        assert_eq!(stable_payload(actual), stable_payload(expected));
    }

    #[tokio::test]
    async fn market_subscription_patch_is_immediately_visible_to_reads() {
        let router = build_router(test_state().await);
        let response = response(
            router.clone(),
            patch_request(
                MARKET_SUBSCRIPTIONS_CONFIG_PATH,
                &MarketSubscriptionPatch {
                    venue: "kraken".to_owned(),
                    spot_enabled: Some(false),
                    perp_enabled: None,
                    funding_enabled: None,
                },
            ),
        )
        .await;
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap_or_else(|error| fail(&format!("response body failed: {error:?}")));
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

        let snapshot = api_market_subscriptions(router).await;
        let kraken = snapshot
            .venues
            .iter()
            .find(|row| row.venue == "kraken")
            .unwrap_or_else(|| fail("kraken subscription row is missing"));
        assert!(!kraken.spot_enabled);
        assert!(kraken.perp_enabled);
        assert!(kraken.funding_enabled);
    }

    #[tokio::test]
    async fn gate_crossex_mode_patch_is_immediately_visible_to_reads() {
        let router = build_router(test_state().await);
        let response = response(
            router.clone(),
            gate_crossex_patch_request(&GateCrossExModeConfigPatch {
                mode: Some(shared_types::GateCrossExMode::Monitor),
                selected_routes: None,
                min_gross_spread_pct: Some(0.25),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let snapshot = api_gate_crossex(router).await;
        assert_eq!(snapshot.config.mode, shared_types::GateCrossExMode::Monitor);
        assert_eq!(snapshot.config.min_gross_spread_pct, 0.25);
        assert_eq!(
            snapshot.runtime_state,
            shared_types::GateCrossExRuntimeState::Warming
        );
    }

    async fn test_state() -> AppState {
        let mut config = AppConfig::default();
        config.history.enabled = false;
        config.security.auth_token = Some(TOKEN.to_owned());
        match AppState::new(config).await {
            Ok(state) => state,
            Err(error) => fail(&format!("state init failed: {error:?}")),
        }
    }

    async fn api_snapshot(router: Router, path: &str) -> VenueRuntimeHealthSnapshot {
        let response = response(router, request(path, Some(TOKEN))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(error) => fail(&format!("response body failed: {error:?}")),
        };
        match serde_json::from_slice(&bytes) {
            Ok(snapshot) => snapshot,
            Err(error) => fail(&format!("response decode failed: {error:?}")),
        }
    }

    async fn api_health_envelope(router: Router) -> SystemHealthEnvelope {
        let response = response(router, request(HEALTH_PATH, Some(TOKEN))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(error) => fail(&format!("response body failed: {error:?}")),
        };
        match serde_json::from_slice(&bytes) {
            Ok(envelope) => envelope,
            Err(error) => fail(&format!("response decode failed: {error:?}")),
        }
    }

    async fn api_market_subscriptions(router: Router) -> MarketSubscriptionsResponse {
        let response = response(router, request(MARKET_SUBSCRIPTIONS_PATH, Some(TOKEN))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(error) => fail(&format!("response body failed: {error:?}")),
        };
        match serde_json::from_slice(&bytes) {
            Ok(snapshot) => snapshot,
            Err(error) => fail(&format!("response decode failed: {error:?}")),
        }
    }

    async fn api_gate_crossex(router: Router) -> GateCrossExModeSnapshot {
        let response = response(router, request(GATE_CROSSEX_PATH, Some(TOKEN))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(error) => fail(&format!("response body failed: {error:?}")),
        };
        match serde_json::from_slice(&bytes) {
            Ok(snapshot) => snapshot,
            Err(error) => fail(&format!("response decode failed: {error:?}")),
        }
    }

    async fn response(router: Router, request: Request<Body>) -> axum::response::Response {
        match router.oneshot(request).await {
            Ok(response) => response,
            Err(error) => match error {},
        }
    }

    fn request(path: &str, token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(path);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder
            .body(Body::empty())
            .unwrap_or_else(|error| fail(&format!("request build failed: {error:?}")))
    }

    fn patch_request(path: &str, patch: &MarketSubscriptionPatch) -> Request<Body> {
        let body = serde_json::to_vec(patch)
            .unwrap_or_else(|error| fail(&format!("request encode failed: {error:?}")));
        Request::builder()
            .method("PATCH")
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(
                "idempotency-key",
                "settings-market-subscription-kraken-test",
            )
            .body(Body::from(body))
            .unwrap_or_else(|error| fail(&format!("request build failed: {error:?}")))
    }

    fn gate_crossex_patch_request(patch: &GateCrossExModeConfigPatch) -> Request<Body> {
        let body = serde_json::to_vec(patch)
            .unwrap_or_else(|error| fail(&format!("request encode failed: {error:?}")));
        Request::builder()
            .method("PATCH")
            .uri(GATE_CROSSEX_CONFIG_PATH)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", "gate-crossex-mode-update-test")
            .body(Body::from(body))
            .unwrap_or_else(|error| fail(&format!("request build failed: {error:?}")))
    }

    fn stable_payload(snapshot: VenueRuntimeHealthSnapshot) -> Value {
        let mut value = serde_json::to_value(snapshot)
            .unwrap_or_else(|error| fail(&format!("snapshot encode failed: {error:?}")));
        remove_clock_fields(&mut value);
        value
    }

    fn remove_clock_fields(value: &mut Value) {
        match value {
            Value::Array(values) => values.iter_mut().for_each(remove_clock_fields),
            Value::Object(object) => {
                for field in ["generatedAtMs", "observedAtMs", "freshnessMs"] {
                    object.remove(field);
                }
                object.values_mut().for_each(remove_clock_fields);
            }
            _ => {}
        }
    }

    #[allow(clippy::panic)]
    fn fail(message: &str) -> ! {
        panic!("{message}")
    }
}
