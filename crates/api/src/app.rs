//! 路由组装。

use crate::middleware as mw;
use crate::route_specs;
use crate::state::AppState;
use axum::http::{header, HeaderName, HeaderValue, Method};
use axum::Router;
use common::config::SecurityConfig;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub(crate) fn build_router(state: AppState) -> Router {
    let surface = state.config().api_surface.clone();
    let cors = build_cors_layer(&state.config().security);
    let auth = axum::middleware::from_fn_with_state(state.clone(), mw::auth::require_auth);
    tracing::info!(
        enabled_route_groups = route_specs::enabled_route_spec_count(&surface),
        total_route_groups = route_specs::route_specs().len(),
        seeded_route_endpoints = route_specs::seeded_endpoint_spec_count(&surface),
        seeded_route_endpoint_metadata = ?route_specs::seeded_endpoint_registry_summary(&surface),
        enabled_route_group_keys = ?route_specs::enabled_route_registry_summary(&surface),
        disabled_route_feature_flags = ?route_specs::disabled_route_registry_summary(&surface),
        "api route registry assembled"
    );

    route_specs::build_surface_router(&surface)
        // layer 顺序：内 → 外。auth 必须在 cors 内，否则 OPTIONS preflight 会被 401。
        .layer(auth)
        .layer(axum::middleware::from_fn(mw::trace::trace_request))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors)
        .with_state(state)
}

/// 根据 `SecurityConfig` 构造 CORS layer。
///
/// 行为：
/// - `allowed_origins` 为空：fallback `Any`（dev / 测试），打 warn。
/// - 非空：解析为 `HeaderValue` 列表，无效项打 warn 后跳过；全部失败时阻断所有 Origin。
/// - methods / headers 始终显式列出（不再 Any），以便配合 `allow_credentials` 安全升级。
fn build_cors_layer(security: &SecurityConfig) -> CorsLayer {
    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];
    let request_id_header = HeaderName::from_static("x-request-id");
    let idempotency_header = HeaderName::from_static("idempotency-key");
    let x_idempotency_header = HeaderName::from_static("x-idempotency-key");
    let headers = [
        header::CONTENT_TYPE,
        header::AUTHORIZATION,
        header::ACCEPT,
        request_id_header.clone(),
        idempotency_header,
        x_idempotency_header,
    ];
    let response_headers = [
        header::RETRY_AFTER,
        header::WWW_AUTHENTICATE,
        request_id_header,
    ];

    let allow_origin = parse_allow_origin(&security.allowed_origins);

    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods(methods)
        .allow_headers(headers)
        .expose_headers(response_headers)
        .max_age(std::time::Duration::from_secs(3600))
}

fn parse_allow_origin(origins: &[String]) -> AllowOrigin {
    if origins.is_empty() {
        return any_origin_with_warning(
            "CORS: allowed_origins empty; falling back to `Any` (dev mode). \
             Production must set APP_SECURITY__ALLOWED_ORIGINS.",
        );
    }
    let parsed = parse_origin_values(origins);
    if parsed.is_empty() {
        tracing::warn!("CORS: all configured origins invalid; blocking all origins");
        return AllowOrigin::list(Vec::<HeaderValue>::new());
    }
    AllowOrigin::list(parsed)
}

fn parse_origin_values(origins: &[String]) -> Vec<HeaderValue> {
    origins
        .iter()
        .filter_map(|origin| parse_origin_value(origin))
        .collect()
}

fn parse_origin_value(raw: &str) -> Option<HeaderValue> {
    match HeaderValue::from_str(raw.trim()) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(
                origin = %raw,
                %error,
                "CORS: invalid origin, skipped"
            );
            None
        }
    }
}

fn any_origin_with_warning(message: &'static str) -> AllowOrigin {
    tracing::warn!("{message}");
    AllowOrigin::from(Any)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{HeaderMap, Request, StatusCode};
    use common::config::AppConfig;
    use shared_types::{ActionRunKind, ActionRunStatus};
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    use tower::ServiceExt;

    #[allow(clippy::panic)]
    fn fail(message: &str) -> ! {
        panic!("{message}");
    }

    #[tokio::test]
    async fn trading_alias_routes_are_registered() {
        let router = build_router(test_state().await);

        for path in [
            "/api/trading/portfolio/snapshot",
            "/api/trading/venues/quality",
            "/api/trading/adapters",
            "/api/trading/ws/venues",
            "/api/trading/rest/endpoints",
            "/api/trading/fee-schedules",
            "/api/exchanges/credentials",
            "/api/system/market-data-diagnostics",
        ] {
            assert_eq!(route_status(router.clone(), path).await, StatusCode::OK);
        }

        assert_eq!(
            route_status(router, "/api/trading/live-readiness").await,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn json_payloads_negotiate_gzip_transfer_encoding() {
        let router = build_router(test_state().await);
        let response = route_response(
            router,
            Request::builder()
                .uri("/api/trading/portfolio/snapshot")
                .header(header::ACCEPT_ENCODING, "gzip")
                .body(Body::empty())
                .unwrap_or_else(|error| fail(&format!("request build failed: {error:?}"))),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header_value(response.headers(), header::CONTENT_ENCODING),
            Some("gzip")
        );
        assert!(header_value(response.headers(), header::VARY)
            .is_some_and(|value| value.to_ascii_lowercase().contains("accept-encoding")));
    }

    #[tokio::test]
    async fn fee_schedule_registry_route_returns_official_fixture_rows() {
        let router = build_router(test_state().await);
        let response = route_response(router, request("/api/trading/fee-schedules")).await;
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        let venues = body["venues"]
            .as_array()
            .unwrap_or_else(|| fail("fee schedule venues missing"));
        assert_eq!(venues.len(), 8);

        let row_count: usize = venues
            .iter()
            .map(|venue| {
                venue["schedules"]
                    .as_array()
                    .unwrap_or_else(|| fail("fee schedule rows missing"))
                    .len()
            })
            .sum();
        assert_eq!(row_count, 16);

        let binance = venues
            .iter()
            .find(|venue| venue["venue"] == "binance")
            .unwrap_or_else(|| fail("binance fee schedule missing"));
        let schedules = binance["schedules"]
            .as_array()
            .unwrap_or_else(|| fail("binance fee schedule rows missing"));
        assert_eq!(schedules.len(), 2);
        let schedule = schedules
            .iter()
            .find(|row| row["product"] == "spot")
            .unwrap_or_else(|| fail("binance spot fee schedule row missing"));
        assert_eq!(schedule["product"], "spot");
        assert!(schedule["fixtureId"]
            .as_str()
            .is_some_and(|value| value.starts_with("standard-fee:binance:spot")));
        assert_eq!(schedule["snapshotTtlMs"], 86_400_000);
        assert!(schedule["evidence"]["sourceUrl"]
            .as_str()
            .is_some_and(|url| url.starts_with("https://")));
        assert!(schedule["evidence"]["scheduleVersion"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty()));
        assert!(schedule["evidence"]["tier"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty()));
        assert!(schedule["evidence"]["scope"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty()));
    }

    #[tokio::test]
    async fn onchain_execution_runs_route_returns_backend_owned_snapshot() {
        let router = build_router(test_state().await);
        let response =
            route_response(router, request("/api/onchain/execution/runs?limit=20")).await;
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        assert_eq!(body["rows"].as_array().map(Vec::len), Some(0));
        assert!(body["observedAtMs"].as_i64().is_some_and(|value| value > 0));
    }

    #[tokio::test]
    async fn onchain_token_approval_runs_route_returns_backend_owned_snapshot() {
        let router = build_router(test_state().await);
        let response =
            route_response(router, request("/api/onchain/token-approval/runs?limit=20")).await;
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        assert_eq!(body["rows"].as_array().map(Vec::len), Some(0));
        assert!(body["observedAtMs"].as_i64().is_some_and(|value| value > 0));
    }

    async fn route_status(router: Router, path: &str) -> StatusCode {
        let response = match router.oneshot(request(path)).await {
            Ok(response) => response,
            Err(error) => match error {},
        };
        response.status()
    }

    async fn test_state() -> AppState {
        state_from(test_config()).await
    }

    fn test_config() -> AppConfig {
        let mut config = AppConfig::default();
        config.history.enabled = false;
        config
    }

    async fn state_from(config: AppConfig) -> AppState {
        // 测试默认不开 auth（auth_token=None），保留 dev fallback。
        match AppState::new(config).await {
            Ok(state) => state,
            Err(error) => fail(&format!("state init failed: {error:?}")),
        }
    }

    #[tokio::test]
    async fn legacy_routes_gated_off_by_default() {
        let router = build_router(test_state().await);
        // 无前端引用的 legacy/diagnostic 默认不注册（路径整体 404）。
        for path in [
            "/api/chat",
            "/api/chat/providers",
            "/api/llm/explain-opportunity",
            "/api/options/positions",
            "/api/v1/spot/ticks",
            "/api/v1/strategy/kinds",
        ] {
            assert_eq!(
                route_status(router.clone(), path).await,
                StatusCode::NOT_FOUND,
                "{path} should be gated off by default"
            );
        }
    }

    #[tokio::test]
    async fn deleted_simulation_routes_stay_absent_with_all_remaining_surfaces_enabled() {
        let router = build_router(state_from(all_surface_enabled_config()).await);

        for path in [
            "/api/simulation/portfolio",
            "/api/simulation/positions/open",
            "/api/simulation/positions/example/close",
        ] {
            assert_eq!(
                route_status(router.clone(), path).await,
                StatusCode::NOT_FOUND,
                "{path} must remain absent after the simulation HTTP surface was deleted"
            );
        }
    }

    #[tokio::test]
    async fn strategy_v1_requires_explicit_gate_while_main_strategy_stays_available() {
        let default_router = build_router(test_state().await);
        assert_eq!(
            route_status(default_router.clone(), "/api/strategy/main-kinds").await,
            StatusCode::OK
        );
        assert_eq!(
            route_status(default_router, "/api/v1/strategy/kinds").await,
            StatusCode::NOT_FOUND
        );

        let mut config = test_config();
        config.api_surface.strategy_v1 = true;
        let enabled_router = build_router(state_from(config).await);
        assert_eq!(
            route_status(enabled_router.clone(), "/api/strategy/main-kinds").await,
            StatusCode::OK
        );
        assert_eq!(
            route_status(enabled_router, "/api/v1/strategy/kinds").await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn gated_routes_register_when_enabled() {
        let mut config = test_config();
        config.api_surface.options = true;
        config.api_surface.spot_v1 = true;
        config.api_surface.chat = true;
        config.api_surface.llm_diagnostics = true;
        config.api_surface.watchlist_alerts = true;
        config.api_surface.strategy_v1 = true;
        let router = build_router(state_from(config).await);
        assert_legacy_chat_surface(router.clone()).await;
        assert_legacy_options_surface(router.clone()).await;
        assert_eq!(
            route_status(router.clone(), "/api/v1/spot/ticks").await,
            StatusCode::OK
        );
        assert_eq!(
            route_status(router.clone(), "/api/v1/strategy/kinds").await,
            StatusCode::OK
        );
        assert_eq!(
            route_status(router.clone(), "/api/watchlist").await,
            StatusCode::OK
        );
        assert_eq!(
            route_status(router.clone(), "/api/alerts/rules").await,
            StatusCode::OK
        );
    }

    #[cfg(feature = "legacy-chat")]
    async fn assert_legacy_chat_surface(router: Router) {
        assert_eq!(
            route_status(router.clone(), "/api/chat/providers").await,
            StatusCode::OK
        );
        // llm 为 POST 路由：启用后 GET 命中返回 405（已注册）而非 404。
        assert_eq!(
            route_status(router, "/api/llm/explain-opportunity").await,
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[cfg(not(feature = "legacy-chat"))]
    async fn assert_legacy_chat_surface(router: Router) {
        assert_eq!(
            route_status(router.clone(), "/api/chat/providers").await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            route_status(router, "/api/llm/explain-opportunity").await,
            StatusCode::NOT_FOUND
        );
    }

    #[cfg(feature = "legacy-options")]
    async fn assert_legacy_options_surface(router: Router) {
        assert_eq!(
            route_status(router, "/api/options/positions").await,
            StatusCode::NOT_IMPLEMENTED
        );
    }

    #[cfg(not(feature = "legacy-options"))]
    async fn assert_legacy_options_surface(router: Router) {
        assert_eq!(
            route_status(router, "/api/options/positions").await,
            StatusCode::NOT_FOUND
        );
    }

    #[cfg(feature = "legacy-options")]
    #[tokio::test]
    async fn options_positions_fail_closed_when_surface_enabled() {
        let mut config = test_config();
        config.api_surface.options = true;
        let router = build_router(state_from(config).await);
        let response = route_response(router, request("/api/options/positions")).await;

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], "OPTIONS_POSITIONS_UNSUPPORTED");
        assert_eq!(
            body["error"]["details"]["reason"],
            "no_verified_option_account_source"
        );
    }

    #[cfg(feature = "legacy-options")]
    #[tokio::test]
    async fn options_calculator_invalid_input_returns_typed_problem() {
        let mut config = test_config();
        config.api_surface.options = true;
        let router = build_router(state_from(config).await);
        let response = route_response(
            router,
            post_json(
                "/api/options/price",
                &serde_json::json!({
                    "spotPrice": -1.0,
                    "strike": 100.0,
                    "daysToExpiry": 0.0,
                    "riskFreeRate": 0.02,
                    "volatility": 6.0,
                    "optionType": "call"
                }),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], "OPTIONS_CALCULATOR_INVALID_INPUT");
        assert!(body["error"]["details"]["fields"].is_array());
    }

    #[tokio::test]
    async fn route_inventory_default_exposure_matches_build_router() {
        let default_router = build_router(test_state().await);
        let enabled_router = build_router(state_from(all_surface_enabled_config()).await);

        for row in route_inventory_rows() {
            let path = sample_inventory_path(&row.path);
            match row.default_exposure.as_str() {
                "always" | "default_on" => {
                    assert_route_methods_match_inventory(default_router.clone(), &row, &path).await
                }
                "default_off" => {
                    assert_route_not_registered(default_router.clone(), &row, &path).await
                }
                other => fail(&format!(
                    "unknown default_exposure `{other}` for {}",
                    row.path
                )),
            }

            if compiled_surface_available(&row.feature_flag) {
                assert_route_methods_match_inventory(enabled_router.clone(), &row, &path).await;
            } else if row.feature_flag.starts_with("api_surface.") {
                assert_route_not_registered(enabled_router.clone(), &row, &path).await;
            }
        }
    }

    #[test]
    fn route_inventory_optional_surfaces_are_default_off() {
        let mut optional_surface_count = 0;

        for row in route_inventory_rows()
            .into_iter()
            .filter(|row| row.feature_flag.starts_with("api_surface."))
        {
            optional_surface_count += 1;
            assert_eq!(
                row.default_exposure, "default_off",
                "{} optional route must not be enabled by default",
                row.path
            );
        }

        assert!(
            optional_surface_count > 0,
            "inventory should contain api_surface-gated routes"
        );
    }

    #[tokio::test]
    async fn route_inventory_auth_policy_matches_runtime_security() {
        let router = build_router(state_from(all_surface_enabled_prod_like_config()).await);

        for row in route_inventory_rows() {
            if !route_inventory_surface_available(&row) {
                continue;
            }
            assert_inventory_auth_policy(router.clone(), &row).await;
        }
    }

    #[test]
    fn route_inventory_high_risk_routes_declare_mutation_audit_policy() {
        for row in route_inventory_rows() {
            if row.risk != "high" {
                continue;
            }
            assert!(
                matches!(
                    row.audit_policy.as_str(),
                    "action_run" | "required" | "secret_mutation" | "external_payload"
                ),
                "{} high-risk route has weak audit_policy={}",
                row.path,
                row.audit_policy
            );
        }
    }

    #[test]
    fn route_inventory_audit_policy_matches_runtime() {
        for row in route_inventory_rows() {
            assert_audit_policy_methods_are_coherent(&row);
        }
    }

    #[test]
    fn route_inventory_groups_are_covered_by_registry() {
        for row in route_inventory_rows() {
            if !route_inventory_surface_available(&row) {
                continue;
            }
            assert!(
                route_registry_covers(&row.router, &row.feature_flag),
                "{} uses router={} feature_flag={} but no RouteSpec covers it",
                row.path,
                row.router,
                row.feature_flag
            );
        }
    }

    #[test]
    fn route_inventory_and_registry_group_pairs_match_exactly() {
        let duplicate_pairs = duplicate_route_registry_pairs();
        assert!(
            duplicate_pairs.is_empty(),
            "duplicate RouteSpec router/feature pairs: {duplicate_pairs:?}"
        );

        let inventory_pairs = route_inventory_group_pairs();
        let registry_pairs = route_registry_group_pairs();
        assert_eq!(
            inventory_pairs, registry_pairs,
            "RouteSpec runtime registry and API_ROUTE_INVENTORY router/feature pairs drifted"
        );
    }

    #[test]
    fn route_registry_seeded_endpoint_metadata_matches_inventory() {
        let inventory = route_inventory_endpoint_rows();
        for spec in route_specs::route_specs() {
            for endpoint in spec.endpoints() {
                let key = (endpoint.path().to_owned(), endpoint.methods().to_owned());
                let Some(row) = inventory.get(&key) else {
                    fail(&format!(
                        "RouteSpec endpoint {} {} is missing from API_ROUTE_INVENTORY",
                        endpoint.methods(),
                        endpoint.path()
                    ));
                };
                assert_eq!(row.router, spec.router_key(), "{} router drift", row.path);
                assert_eq!(
                    row.feature_flag,
                    spec.feature_flag(),
                    "{} feature_flag drift",
                    row.path
                );
                assert_eq!(row.class, endpoint.class(), "{} class drift", row.path);
                assert_eq!(
                    row.default_exposure,
                    endpoint.default_exposure(),
                    "{} default_exposure drift",
                    row.path
                );
                assert_eq!(row.risk, endpoint.risk(), "{} risk drift", row.path);
                assert_eq!(
                    row.auth_policy,
                    endpoint.auth_policy(),
                    "{} auth_policy drift",
                    row.path
                );
                assert_eq!(
                    row.audit_policy,
                    endpoint.audit_policy(),
                    "{} audit_policy drift",
                    row.path
                );
            }
        }
    }

    #[test]
    fn route_registry_seeds_core_endpoints() {
        let seeded = route_seeded_endpoint_keys();
        let missing: Vec<String> = route_inventory_rows()
            .into_iter()
            .filter(|row| row.feature_flag == "core")
            .filter(|row| !seeded.contains(&(row.path.clone(), row.methods_label.clone())))
            .map(|row| format!("{} {}", row.methods_label, row.path))
            .collect();

        assert!(
            missing.is_empty(),
            "core routes missing RouteSpec endpoint metadata: {missing:?}"
        );
    }

    #[test]
    fn route_registry_seeds_all_available_inventory_endpoints() {
        let seeded = route_seeded_endpoint_keys();
        let missing: Vec<String> = route_inventory_rows()
            .into_iter()
            .filter(route_inventory_surface_available)
            .filter(|row| !seeded.contains(&(row.path.clone(), row.methods_label.clone())))
            .map(|row| format!("{} {}", row.methods_label, row.path))
            .collect();

        assert!(
            missing.is_empty(),
            "available routes missing RouteSpec endpoint metadata: {missing:?}"
        );
    }

    #[test]
    fn route_inventory_action_run_policies_match_typed_runtime_registry() {
        let inventory: BTreeSet<(String, String)> = route_inventory_rows()
            .into_iter()
            .filter(|row| matches!(row.audit_policy.as_str(), "action_run" | "secret_mutation"))
            .map(|row| (row.path, row.methods_label))
            .collect();
        let typed: BTreeMap<(String, String), ActionRunKind> = route_specs::route_specs()
            .iter()
            .flat_map(|spec| spec.endpoints())
            .filter_map(|endpoint| {
                endpoint.action_run_kind().map(|kind| {
                    (
                        (endpoint.path().to_owned(), endpoint.methods().to_owned()),
                        kind,
                    )
                })
            })
            .collect();

        assert_eq!(
            typed.keys().cloned().collect::<BTreeSet<_>>(),
            inventory,
            "every inventory action_run or secret_mutation route must declare its ActionRun kind"
        );

        let kinds: HashSet<ActionRunKind> = typed.values().copied().collect();
        assert_eq!(
            kinds.len(),
            typed.len(),
            "an ActionRun kind must have one canonical route owner"
        );
        assert_eq!(
            kinds.len(),
            21,
            "all active ActionRun kinds must remain represented by route runtime metadata"
        );
        assert!(!kinds.contains(&ActionRunKind::AutomationLiveUnlock));
    }

    #[tokio::test]
    async fn submit_order_requires_client_order_id() {
        let router = build_router(test_state().await);
        let response = route_response(
            router,
            post_json(
                "/api/trading/orders",
                &serde_json::json!({
                    "exchange": "mock",
                    "symbol": "BTCUSDT",
                    "side": "buy",
                    "orderType": "limit",
                    "quantity": 0.01,
                    "price": 50000.0
                }),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = body_string(response).await;
        assert!(body.contains("SUBMIT_ORDER_INVALID"), "body: {body}");
        assert!(body.contains("clientOrderId"), "body: {body}");
    }

    #[tokio::test]
    async fn submit_order_replays_client_order_id_without_second_action_run() {
        let state = test_state().await;
        let router = build_router(state.clone());
        let payload = serde_json::json!({
            "id": "manual-1",
            "clientOrderId": "client-replay-1",
            "exchange": "mock",
            "symbol": "BTCUSDT",
            "side": "buy",
            "orderType": "limit",
            "quantity": 0.01,
            "price": 50000.0
        });

        let first = submit_order(router.clone(), &payload).await;
        let second = submit_order(router, &payload).await;

        assert_eq!(first["intent"]["id"], second["intent"]["id"]);
        assert_eq!(state.trading_service().list_orders().len(), 1);
        assert_eq!(state.action_runs().len(), 1);
    }

    #[tokio::test]
    async fn cors_allows_idempotency_headers_for_browser_actions() {
        let router = build_router(test_state().await);
        let response = route_response(
            router,
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/exchanges/credentials")
                .header(header::ORIGIN, "http://127.0.0.1:8080")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, Method::POST.as_str())
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "content-type,x-request-id,idempotency-key,x-idempotency-key",
                )
                .body(Body::empty())
                .unwrap_or_else(|error| fail(&format!("request build failed: {error:?}"))),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let allow_headers = response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        assert!(allow_headers.contains("idempotency-key"));
        assert!(allow_headers.contains("x-idempotency-key"));
    }

    #[tokio::test]
    async fn prod_like_security_contract_rejects_no_auth_and_evil_origin() {
        let router = build_router(state_from(prod_like_security_config()).await);

        let missing_auth = route_response(
            router.clone(),
            get_with_request_id_and_origin(
                "/api/trading/status",
                "security-rid-1",
                "http://127.0.0.1:8080",
            ),
        )
        .await;
        assert_eq!(missing_auth.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            header_value(
                missing_auth.headers(),
                header::HeaderName::from_static("x-request-id")
            ),
            Some("security-rid-1")
        );
        assert_eq!(
            header_value(missing_auth.headers(), header::WWW_AUTHENTICATE),
            Some("Bearer realm=\"crossline-api\"")
        );
        assert_eq!(
            header_value(missing_auth.headers(), header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some("http://127.0.0.1:8080")
        );
        let exposed = header_value(
            missing_auth.headers(),
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
        )
        .unwrap_or("")
        .to_ascii_lowercase();
        for expected in ["retry-after", "www-authenticate", "x-request-id"] {
            assert!(exposed.contains(expected), "exposed headers: {exposed}");
        }
        let body = json_body(missing_auth).await;
        assert_eq!(body["error"]["code"], "UNAUTHORIZED");
        assert_eq!(body["error"]["requestId"], "security-rid-1");

        let authed = route_response(
            router.clone(),
            get_with_auth("/api/trading/status", "test-token"),
        )
        .await;
        assert_ne!(authed.status(), StatusCode::UNAUTHORIZED);

        let evil = route_response(
            router.clone(),
            preflight_request(
                "/api/trading/orders",
                "https://evil.example",
                &Method::POST,
                "content-type,idempotency-key",
            ),
        )
        .await;
        assert_ne!(
            header_value(evil.headers(), header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some("https://evil.example")
        );

        let allowed = route_response(
            router,
            preflight_request(
                "/api/trading/orders",
                "http://127.0.0.1:8080",
                &Method::POST,
                "content-type,x-request-id,idempotency-key,x-idempotency-key",
            ),
        )
        .await;
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(
            header_value(allowed.headers(), header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some("http://127.0.0.1:8080")
        );
        let allow_headers =
            header_value(allowed.headers(), header::ACCESS_CONTROL_ALLOW_HEADERS).unwrap_or("");
        assert!(allow_headers.contains("idempotency-key"));
        assert!(allow_headers.contains("x-idempotency-key"));
    }

    #[tokio::test]
    async fn prod_like_security_contract_protects_runtime_diagnostics() {
        let router = build_router(state_from(prod_like_security_config()).await);

        let missing_auth = route_response(
            router.clone(),
            get_with_request_id("/api/system/venue-operation-health", "diag-rid-1"),
        )
        .await;
        assert_eq!(missing_auth.status(), StatusCode::UNAUTHORIZED);
        let body = json_body(missing_auth).await;
        assert_eq!(body["error"]["code"], "UNAUTHORIZED");
        assert_eq!(body["error"]["requestId"], "diag-rid-1");

        let authed = route_response(
            router,
            get_with_auth("/api/system/venue-operation-health", "test-token"),
        )
        .await;
        assert_eq!(authed.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn invalid_configured_cors_origins_do_not_fallback_to_any() {
        let mut config = prod_like_security_config();
        config.security.allowed_origins = vec!["https://bad\norigin.example".to_owned()];
        let router = build_router(state_from(config).await);

        let response = route_response(
            router,
            preflight_request(
                "/api/trading/orders",
                "https://evil.example",
                &Method::POST,
                "content-type,idempotency-key",
            ),
        )
        .await;

        assert_ne!(
            header_value(response.headers(), header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some("https://evil.example")
        );
        assert_ne!(
            header_value(response.headers(), header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some("*")
        );
    }

    #[tokio::test]
    async fn kill_switch_action_run_writes_audit_entries() {
        let dir =
            tempfile::tempdir().unwrap_or_else(|error| fail(&format!("tempdir failed: {error:?}")));
        let audit_path = dir.path().join("security_audit.jsonl");
        mw::audit::init(audit_path.to_str());
        let state = state_from(prod_like_security_config()).await;
        let router = build_router(state.clone());

        let response = route_response(
            router,
            post_json_with_auth(
                "/api/trading/kill-switch",
                &serde_json::json!({
                    "active": true,
                    "expectedActive": false,
                    "expectedOpenOrderCount": 0,
                    "reason": "security.audit.test"
                }),
                "test-token",
                "security-audit-rid",
                "203.0.113.10",
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        let action_run_id = body["actionRunId"]
            .as_str()
            .unwrap_or_else(|| fail("kill switch response actionRunId missing"));
        let run = state
            .action_runs()
            .get(action_run_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_else(|| fail("kill switch ActionRun missing"));
        assert_eq!(run.kind, ActionRunKind::TradingKillSwitch);
        assert_eq!(run.status, ActionRunStatus::Succeeded);
        assert_eq!(run.request_id.as_deref(), Some("security-audit-rid"));
        assert!(run.actor.starts_with("api-token:"));
        assert!(state.trading_service().risk_config().kill_switch_active);

        mw::audit::flush_for_test();
        let contents = std::fs::read_to_string(&audit_path)
            .unwrap_or_else(|error| fail(&format!("audit read failed: {error:?}")));
        let events: Vec<serde_json::Value> = contents
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|error| fail(&format!("audit json failed: {error}: {line}")))
            })
            .collect();

        assert!(has_audit_outcome(&events, "accepted"), "events: {events:?}");
        assert!(has_audit_outcome(&events, "success"), "events: {events:?}");
        let linked: Vec<&serde_json::Value> = events
            .iter()
            .filter(|event| event["detail"]["actionRunId"] == action_run_id)
            .collect();
        assert_eq!(linked.len(), 2, "events: {events:?}");
        for event in linked {
            assert_eq!(event["detail"]["requestId"], "security-audit-rid");
            assert_eq!(event["actor"], run.actor);
        }
        assert!(
            !contents.contains("test-token"),
            "audit records must not contain bearer tokens: {contents}"
        );
    }

    fn prod_like_security_config() -> AppConfig {
        let mut config = test_config();
        config.security.auth_token = Some("test-token".to_owned());
        config.security.allowed_origins = vec!["http://127.0.0.1:8080".to_owned()];
        config
    }

    fn all_surface_enabled_prod_like_config() -> AppConfig {
        let mut config = all_surface_enabled_config();
        config.security.auth_token = Some("test-token".to_owned());
        config.security.allowed_origins = vec!["http://127.0.0.1:8080".to_owned()];
        config
    }

    fn request(path: &str) -> Request<Body> {
        match Request::builder().uri(path).body(Body::empty()) {
            Ok(request) => request,
            Err(error) => fail(&format!("request build failed: {error:?}")),
        }
    }

    fn post_json(path: &str, body: &serde_json::Value) -> Request<Body> {
        match Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
        {
            Ok(request) => request,
            Err(error) => fail(&format!("request build failed: {error:?}")),
        }
    }

    fn post_json_with_auth(
        path: &str,
        body: &serde_json::Value,
        token: &str,
        request_id: &str,
        actor: &str,
    ) -> Request<Body> {
        match Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::HeaderName::from_static("x-request-id"), request_id)
            .header("x-forwarded-for", actor)
            .body(Body::from(body.to_string()))
        {
            Ok(request) => request,
            Err(error) => fail(&format!("request build failed: {error:?}")),
        }
    }

    fn get_with_request_id(path: &str, request_id: &str) -> Request<Body> {
        match Request::builder()
            .uri(path)
            .header(header::HeaderName::from_static("x-request-id"), request_id)
            .body(Body::empty())
        {
            Ok(request) => request,
            Err(error) => fail(&format!("request build failed: {error:?}")),
        }
    }

    fn get_with_request_id_and_origin(path: &str, request_id: &str, origin: &str) -> Request<Body> {
        match Request::builder()
            .uri(path)
            .header(header::HeaderName::from_static("x-request-id"), request_id)
            .header(header::ORIGIN, origin)
            .body(Body::empty())
        {
            Ok(request) => request,
            Err(error) => fail(&format!("request build failed: {error:?}")),
        }
    }

    fn get_with_auth(path: &str, token: &str) -> Request<Body> {
        match Request::builder()
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
        {
            Ok(request) => request,
            Err(error) => fail(&format!("request build failed: {error:?}")),
        }
    }

    fn preflight_request(
        path: &str,
        origin: &str,
        method: &Method,
        request_headers: &str,
    ) -> Request<Body> {
        match Request::builder()
            .method(Method::OPTIONS)
            .uri(path)
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, method.as_str())
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, request_headers)
            .body(Body::empty())
        {
            Ok(request) => request,
            Err(error) => fail(&format!("request build failed: {error:?}")),
        }
    }

    fn header_value(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }

    async fn submit_order(router: Router, body: &serde_json::Value) -> serde_json::Value {
        let response = route_response(router, post_json("/api/trading/orders", body)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let text = body_string(response).await;
        match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => fail(&format!(
                "submit order response decode failed: {error}: {text}"
            )),
        }
    }

    async fn route_response(router: Router, request: Request<Body>) -> axum::response::Response {
        match router.oneshot(request).await {
            Ok(response) => response,
            Err(error) => match error {},
        }
    }

    async fn body_string(response: axum::response::Response) -> String {
        let body = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(body) => body,
            Err(error) => fail(&format!("body bytes failed: {error:?}")),
        };
        match String::from_utf8(body.to_vec()) {
            Ok(text) => text,
            Err(error) => fail(&format!("utf8 body failed: {error:?}")),
        }
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        let text = body_string(response).await;
        match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(error) => fail(&format!("json body decode failed: {error}: {text}")),
        }
    }

    async fn route_method_status(router: Router, method: Method, path: &str) -> StatusCode {
        let response = route_response(router, method_request(method, path)).await;
        response.status()
    }

    async fn assert_route_methods_match_inventory(
        router: Router,
        row: &RouteInventoryRow,
        path: &str,
    ) {
        for method in &row.methods {
            let status = route_method_status(router.clone(), method.clone(), path).await;
            if !row.path.contains("/:") {
                assert_ne!(
                    status,
                    StatusCode::NOT_FOUND,
                    "{} {path} is listed but returned 404",
                    method.as_str()
                );
            }
            assert_ne!(
                status,
                StatusCode::METHOD_NOT_ALLOWED,
                "{} {path} is listed but returned 405",
                method.as_str()
            );
        }
    }

    async fn assert_route_not_registered(router: Router, row: &RouteInventoryRow, path: &str) {
        for method in route_probe_methods() {
            let status = route_method_status(router.clone(), method.clone(), path).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{} {path} should be gated off for {} ({status})",
                method.as_str(),
                row.path
            );
        }
    }

    async fn assert_inventory_auth_policy(router: Router, row: &RouteInventoryRow) {
        let path = sample_inventory_path(&row.path);
        let method = first_inventory_method(row);
        let status = route_method_status(router, method.clone(), &path).await;
        match row.auth_policy.as_str() {
            "public_liveness" => assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "{} {path} must remain public liveness",
                method.as_str()
            ),
            "bearer" | "scrape_bearer" => assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{} {path} must reject missing auth for auth_policy={}",
                method.as_str(),
                row.auth_policy
            ),
            "bearer_ws" => {}
            other => fail(&format!(
                "{} has unsupported auth_policy `{other}` in inventory",
                row.path
            )),
        }
    }

    fn route_probe_methods() -> [Method; 5] {
        [
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::PUT,
        ]
    }

    fn method_request(method: Method, path: &str) -> Request<Body> {
        match Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
        {
            Ok(request) => request,
            Err(error) => fail(&format!("request build failed: {error:?}")),
        }
    }

    fn all_surface_enabled_config() -> AppConfig {
        let mut config = test_config();
        config.api_surface.chat = true;
        config.api_surface.llm_diagnostics = true;
        config.api_surface.options = true;
        config.api_surface.watchlist_alerts = true;
        config.api_surface.spot_v1 = true;
        config.api_surface.strategy_v1 = true;
        config
    }

    fn compiled_surface_available(feature_flag: &str) -> bool {
        compiled_legacy_chat_surface(feature_flag)
            || compiled_legacy_options_surface(feature_flag)
            || matches!(
                feature_flag,
                "api_surface.watchlist_alerts" | "api_surface.spot_v1" | "api_surface.strategy_v1"
            )
    }

    fn route_inventory_surface_available(row: &RouteInventoryRow) -> bool {
        row.feature_flag == "core" || compiled_surface_available(&row.feature_flag)
    }

    fn assert_audit_policy_methods_are_coherent(row: &RouteInventoryRow) {
        let has_get = row.methods.contains(&Method::GET);
        match row.audit_policy.as_str() {
            "none" | "read" | "metrics" => {}
            "action_run" | "required" | "secret_mutation" | "external_payload" => assert!(
                !has_get,
                "{} mixes GET with mutation/external audit_policy={}",
                row.path, row.audit_policy
            ),
            other => fail(&format!(
                "{} has unsupported audit_policy `{other}` in inventory",
                row.path
            )),
        }
    }

    #[cfg(feature = "legacy-chat")]
    fn compiled_legacy_chat_surface(feature_flag: &str) -> bool {
        matches!(
            feature_flag,
            "api_surface.chat" | "api_surface.llm_diagnostics"
        )
    }

    #[cfg(not(feature = "legacy-chat"))]
    fn compiled_legacy_chat_surface(_feature_flag: &str) -> bool {
        false
    }

    #[cfg(feature = "legacy-options")]
    fn compiled_legacy_options_surface(feature_flag: &str) -> bool {
        matches!(feature_flag, "api_surface.options")
    }

    #[cfg(not(feature = "legacy-options"))]
    fn compiled_legacy_options_surface(_feature_flag: &str) -> bool {
        false
    }

    #[derive(Debug)]
    struct RouteInventoryRow {
        path: String,
        router: String,
        methods_label: String,
        methods: Vec<Method>,
        class: String,
        default_exposure: String,
        risk: String,
        auth_policy: String,
        audit_policy: String,
        feature_flag: String,
    }

    fn route_inventory_rows() -> Vec<RouteInventoryRow> {
        const INVENTORY: &str = include_str!("../../../docs/API_ROUTE_INVENTORY.tsv");
        let mut rows = Vec::new();
        for (line_no, line) in INVENTORY.lines().enumerate() {
            if line_no == 0 || line.trim().is_empty() {
                continue;
            }
            let cells: Vec<&str> = line.split('\t').collect();
            if cells.len() != 13 {
                fail(&format!(
                    "route inventory line {} has {} cells",
                    line_no + 1,
                    cells.len()
                ));
            }
            rows.push(RouteInventoryRow {
                path: cells[0].to_owned(),
                methods_label: cells[1].to_owned(),
                methods: parse_inventory_methods(cells[1], line_no + 1),
                router: cells[2].to_owned(),
                class: cells[3].to_owned(),
                default_exposure: cells[4].to_owned(),
                risk: cells[5].to_owned(),
                auth_policy: cells[6].to_owned(),
                audit_policy: cells[7].to_owned(),
                feature_flag: cells[8].to_owned(),
            });
        }
        rows
    }

    fn route_inventory_endpoint_rows() -> BTreeMap<(String, String), RouteInventoryRow> {
        let mut rows = BTreeMap::new();
        for row in route_inventory_rows() {
            let key = (row.path.clone(), row.methods_label.clone());
            let previous = rows.insert(key.clone(), row);
            assert!(
                previous.is_none(),
                "duplicate inventory endpoint metadata for {} {}",
                key.1,
                key.0
            );
        }
        rows
    }

    fn route_registry_covers(router_key: &str, feature_flag: &str) -> bool {
        route_specs::route_specs()
            .iter()
            .any(|spec| spec.router_key() == router_key && spec.feature_flag() == feature_flag)
    }

    fn route_inventory_group_pairs() -> BTreeSet<(String, String)> {
        route_inventory_rows()
            .into_iter()
            .filter(route_inventory_surface_available)
            .map(|row| (row.router, row.feature_flag))
            .collect()
    }

    fn route_registry_group_pairs() -> BTreeSet<(String, String)> {
        route_specs::route_specs()
            .iter()
            .map(|spec| (spec.router_key().to_owned(), spec.feature_flag().to_owned()))
            .collect()
    }

    fn route_seeded_endpoint_keys() -> BTreeSet<(String, String)> {
        route_specs::route_specs()
            .iter()
            .flat_map(|spec| spec.endpoints())
            .map(|endpoint| (endpoint.path().to_owned(), endpoint.methods().to_owned()))
            .collect()
    }

    fn duplicate_route_registry_pairs() -> Vec<(String, String)> {
        let mut seen = BTreeSet::new();
        let mut duplicates = BTreeSet::new();
        for pair in route_specs::route_specs()
            .iter()
            .map(|spec| (spec.router_key().to_owned(), spec.feature_flag().to_owned()))
        {
            if !seen.insert(pair.clone()) {
                duplicates.insert(pair);
            }
        }
        duplicates.into_iter().collect()
    }

    fn first_inventory_method(row: &RouteInventoryRow) -> Method {
        match row.methods.first() {
            Some(method) => method.clone(),
            None => fail(&format!("{} has no inventory methods", row.path)),
        }
    }

    fn parse_inventory_methods(value: &str, line_no: usize) -> Vec<Method> {
        value
            .split(',')
            .map(|method| match method {
                "GET" => Method::GET,
                "POST" => Method::POST,
                "PATCH" => Method::PATCH,
                "DELETE" => Method::DELETE,
                "PUT" => Method::PUT,
                other => fail(&format!(
                    "route inventory line {line_no} has unsupported method `{other}`"
                )),
            })
            .collect()
    }

    fn sample_inventory_path(path: &str) -> String {
        path.split('/')
            .map(sample_inventory_segment)
            .collect::<Vec<_>>()
            .join("/")
    }

    fn sample_inventory_segment(segment: &str) -> &str {
        match segment {
            ":id" => "sample-id",
            ":venue" => "binance",
            ":symbol" => "BTCUSDT",
            _ => segment,
        }
    }

    fn has_audit_outcome(events: &[serde_json::Value], outcome: &str) -> bool {
        events.iter().any(|event| {
            event["actor"]
                .as_str()
                .is_some_and(|actor| actor.starts_with("api-token:") && actor != "secure-token")
                && event["action"] == "trading.kill_switch.set"
                && event["resource"] == "kill-switch:on"
                && event["outcome"] == outcome
                && event["detail"]["requestId"] == "security-audit-rid"
        })
    }
}
