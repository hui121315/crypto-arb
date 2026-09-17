use super::*;
use crate::app::build_router;
use axum::body::{to_bytes, Body};
use axum::http::{header, Request};
use common::config::AppConfig;
use tower::ServiceExt;

const TOKEN: &str = "onchain-provider-credentials-test-token";

#[tokio::test]
async fn credential_status_route_is_authenticated_and_never_returns_secret_values(
) -> Result<(), String> {
    let router = build_router(test_state().await?);

    let unauthorized = response(
        router.clone(),
        Request::builder()
            .uri("/api/onchain/credentials")
            .body(Body::empty())
            .map_err(|error| format!("request build failed: {error:?}"))?,
    )
    .await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = response(router, authenticated_get("/api/onchain/credentials")?).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await?;
    let decoded: OnchainProviderCredentialsResponse = serde_json::from_str(&body)
        .map_err(|error| format!("credential status decode failed: {error:?}"))?;

    assert_eq!(decoded.providers.len(), 3);
    let jupiter = decoded
        .providers
        .iter()
        .find(|provider| provider.provider == "jupiter_swap_v2_keyed")
        .ok_or_else(|| "missing keyed Jupiter credential contract".to_owned())?;
    assert_eq!(jupiter.fields.len(), 1);
    assert_eq!(jupiter.fields[0].key, "api_key");
    assert!(jupiter.fields[0].required);
    assert!(decoded
        .providers
        .iter()
        .any(|provider| provider.provider == "zeroex_swap_v2"));
    assert!(decoded
        .providers
        .iter()
        .any(|provider| provider.provider == "okx_dex_v6"));
    assert!(!body.contains("\"value\""));
    Ok(())
}

#[tokio::test]
async fn invalid_save_is_fail_closed_and_records_a_terminal_action_run() -> Result<(), String> {
    let state = test_state().await?;
    let router = build_router(state.clone());
    let request = Request::builder()
        .method("POST")
        .uri("/api/onchain/credentials")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", "onchain-credential-empty-fields")
        .body(Body::from(r#"{"provider":"zeroex_swap_v2","fields":[]}"#))
        .map_err(|error| format!("request build failed: {error:?}"))?;

    let response = response(router, request).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let runs = action_runs::recent(&state);
    let run = runs
        .iter()
        .find(|run| run.kind == ActionRunKind::OnchainProviderCredentialsUpdate)
        .ok_or_else(|| "missing provider credential action run".to_owned())?;
    assert_eq!(run.status, ActionRunStatus::Failed);
    assert_eq!(
        run.idempotency_key.as_deref(),
        Some("onchain-credential-empty-fields")
    );
    Ok(())
}

#[tokio::test]
async fn native_token_identity_route_is_authenticated_and_network_free() -> Result<(), String> {
    let router = build_router(test_state().await?);
    let request = Request::builder()
        .method("POST")
        .uri("/api/onchain/token/resolve")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(
            r#"{{"chain":"base","address":"{}"}}"#,
            shared_types::EVM_NATIVE_TOKEN_ADDRESS
        )))
        .map_err(|error| format!("request build failed: {error:?}"))?;

    let response = response(router, request).await;

    assert_eq!(response.status(), StatusCode::OK);
    let decoded: shared_types::OnchainTokenResolution =
        serde_json::from_str(&body_text(response).await?)
            .map_err(|error| format!("identity decode failed: {error:?}"))?;
    assert_eq!(decoded.decimals, 18);
    let identity = decoded
        .identity
        .ok_or_else(|| "native token identity missing".to_owned())?;
    assert_eq!(identity.symbol, "ETH");
    assert_eq!(identity.decimals, 18);
    assert!(identity.native);
    Ok(())
}

#[tokio::test]
async fn transfer_refresh_route_is_network_free_when_monitor_is_disabled() -> Result<(), String> {
    let router = build_router(test_state().await?);
    let request = Request::builder()
        .method("POST")
        .uri("/api/onchain/transfer-networks/refresh")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .map_err(|error| format!("request build failed: {error:?}"))?;

    let response = response(router, request).await;

    assert_eq!(response.status(), StatusCode::OK);
    let decoded: OnchainComparisonSnapshot = serde_json::from_str(&body_text(response).await?)
        .map_err(|error| format!("transfer refresh decode failed: {error:?}"))?;
    assert!(!decoded.config.enabled);
    Ok(())
}

async fn test_state() -> Result<AppState, String> {
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.security.auth_token = Some(TOKEN.to_owned());
    AppState::new(config)
        .await
        .map_err(|error| format!("state init failed: {error:?}"))
}

fn authenticated_get(path: &str) -> Result<Request<Body>, String> {
    Request::builder()
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .map_err(|error| format!("request build failed: {error:?}"))
}

async fn response(router: Router, request: Request<Body>) -> axum::response::Response {
    match router.oneshot(request).await {
        Ok(response) => response,
        Err(error) => match error {},
    }
}

async fn body_text(response: axum::response::Response) -> Result<String, String> {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| format!("response body failed: {error:?}"))?;
    String::from_utf8(bytes.to_vec()).map_err(|error| format!("response is not utf-8: {error:?}"))
}
