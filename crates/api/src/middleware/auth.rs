//! Bearer token 认证中间件。
//!
//! 行为契约：
//! 1. 若 `SecurityConfig::auth_required() == false`（dev / 测试），直接放行。
//! 2. 若请求路径命中 `auth_exempt_paths`（默认 `/health`），直接放行。
//! 3. 若请求路径是 `/ws`，放行到 WebSocket router 执行一次性 ticket 鉴权。
//! 4. 否则要求 `Authorization: Bearer <token>` 与 `auth_token` 严格相等。
//!
//! 浏览器 `WebSocket` API 无法设置 `Authorization` header，因此 `/ws` 不能复用长期
//! Bearer header。客户端必须先用 REST Bearer 换取短期一次性 ticket，再在 WS 首帧提交。

use crate::middleware::audit;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap};
use axum::middleware::Next;
use axum::response::Response;
use common::AppError;

mod denial;

pub(crate) async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let security = &state.config().security;
    let path = request.uri().path().to_owned();
    audit::clear_verified_actor(request.headers_mut());

    if auth_not_required(security, path.as_str()) || is_websocket_upgrade_path(path.as_str()) {
        return Ok(next.run(request).await);
    }

    let token = match bearer_token(request.headers(), path.as_str()) {
        Ok(token) => token.to_owned(),
        Err(error) => return denial::reject(&request, error),
    };
    let expected = security.auth_token.as_deref().unwrap_or("");
    if !token.is_empty() && constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        audit::insert_verified_bearer_actor(
            request.headers_mut(),
            token.as_str(),
            security.verified_actor_label(),
        );
        Ok(next.run(request).await)
    } else {
        tracing::warn!(path = path.as_str(), "auth: token mismatch");
        denial::reject(&request, AppError::Unauthorized("token mismatch".into()))
    }
}

fn auth_not_required(security: &common::config::SecurityConfig, path: &str) -> bool {
    !security.auth_required() || security.is_path_exempt(path)
}

fn is_websocket_upgrade_path(path: &str) -> bool {
    path == "/ws"
}

fn bearer_token<'a>(headers: &'a HeaderMap, path: &str) -> Result<&'a str, AppError> {
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        tracing::warn!(path, "auth: missing authorization header");
        return Err(AppError::Unauthorized(
            "missing authorization header".into(),
        ));
    };
    let Some(rest) = value.strip_prefix("Bearer ") else {
        tracing::warn!(path, "auth: malformed authorization header");
        return Err(AppError::Unauthorized(
            "malformed authorization header".into(),
        ));
    };
    Ok(rest.trim())
}

/// 常数时间比较，避免 timing attack。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use common::config::{AppConfig, SecurityConfig};
    use tower::ServiceExt;

    async fn make_state(security: SecurityConfig) -> AppState {
        let mut config = AppConfig::default();
        config.history.enabled = false;
        config.security = security;
        match AppState::new(config).await {
            Ok(state) => state,
            Err(error) => panic!("state init failed: {error}"),
        }
    }

    fn make_router(state: AppState) -> Router {
        Router::new()
            .route("/health", get(|| async { "ok" }))
            .route("/api/protected", get(|| async { "secret" }))
            .route("/api/actor", get(actor))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth,
            ))
            .with_state(state)
    }

    async fn actor(headers: HeaderMap) -> String {
        audit::extract_actor(&headers)
    }

    fn request(path: &str, token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(path);
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        match builder.body(Body::empty()) {
            Ok(request) => request,
            Err(error) => panic!("request build failed: {error}"),
        }
    }

    fn raw_auth_request(path: &str, authorization: &str) -> Request<Body> {
        let builder = Request::builder()
            .uri(path)
            .header("authorization", authorization);
        match builder.body(Body::empty()) {
            Ok(request) => request,
            Err(error) => panic!("request build failed: {error}"),
        }
    }

    fn rid_request(path: &str, request_id: &str) -> Request<Body> {
        let builder = Request::builder()
            .uri(path)
            .header("x-request-id", request_id);
        match builder.body(Body::empty()) {
            Ok(request) => request,
            Err(error) => panic!("request build failed: {error}"),
        }
    }

    fn internal_actor_request(path: &str, actor: &str) -> Request<Body> {
        let builder = Request::builder()
            .uri(path)
            .header(audit::VERIFIED_ACTOR_HEADER, actor);
        match builder.body(Body::empty()) {
            Ok(request) => request,
            Err(error) => panic!("request build failed: {error}"),
        }
    }

    #[tokio::test]
    async fn dev_mode_allows_all_when_no_token_configured() {
        let state = make_state(SecurityConfig::default()).await;
        let router = make_router(state);
        let response = route_response(router, request("/api/protected", None)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_missing_authorization_header_when_required() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            ..Default::default()
        })
        .await;
        let router = make_router(state);
        let response = route_response(router, request("/api/protected", None)).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer realm=\"crossline-api\"")
        );
        let body = response_body(response).await;
        assert!(body.contains("\"code\":\"UNAUTHORIZED\""));
        assert!(body.contains("\"source\":\"api.auth\""));
        assert!(body.contains("provide_valid_bearer_token"));
        assert!(!body.contains("missing authorization header"));
    }

    #[tokio::test]
    async fn rejects_wrong_bearer_token() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            ..Default::default()
        })
        .await;
        let router = make_router(state);
        let response = route_response(router, request("/api/protected", Some("wrong"))).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = response_body(response).await;
        assert!(
            body.contains("provide a valid Bearer token"),
            "body: {body}"
        );
        assert!(!body.contains("token mismatch"), "body: {body}");
    }

    #[tokio::test]
    async fn rejects_malformed_authorization_header() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            ..Default::default()
        })
        .await;
        let router = make_router(state);
        let response = route_response(router, raw_auth_request("/api/protected", "Token x")).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = response_body(response).await;
        assert!(
            body.contains("provide a valid Bearer token"),
            "body: {body}"
        );
        assert!(
            !body.contains("malformed authorization header"),
            "body: {body}"
        );
    }

    #[tokio::test]
    async fn unauthorized_carries_request_id_behind_trace_layer() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            ..Default::default()
        })
        .await;
        let router = Router::new()
            .route("/api/protected", get(|| async { "secret" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth,
            ))
            .layer(axum::middleware::from_fn(
                crate::middleware::trace::trace_request,
            ))
            .with_state(state);

        let response = route_response(router, rid_request("/api/protected", "auth-rid-1")).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = response_body(response).await;
        assert!(body.contains("\"code\":\"UNAUTHORIZED\""), "body: {body}");
        assert!(
            body.contains("\"requestId\":\"auth-rid-1\""),
            "body: {body}"
        );
        assert!(body.contains("provide_valid_bearer_token"), "body: {body}");
    }

    #[tokio::test]
    async fn accepts_valid_bearer_token() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            ..Default::default()
        })
        .await;
        let router = make_router(state);
        let response =
            route_response(router, request("/api/protected", Some("super-secret"))).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn valid_bearer_token_inserts_verified_actor() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            ..Default::default()
        })
        .await;
        let router = make_router(state);

        let response = route_response(router, request("/api/actor", Some("super-secret"))).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert!(body.starts_with("api-token:"), "body: {body}");
        assert!(!body.contains("super-secret"), "body: {body}");
    }

    #[tokio::test]
    async fn valid_bearer_token_uses_configured_operator_label_without_exposing_token() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            auth_actor_label: Some("operator-primary".to_owned()),
            ..Default::default()
        })
        .await;
        let router = make_router(state);

        let response = route_response(router, request("/api/actor", Some("super-secret"))).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert!(
            body.starts_with("api-token:operator-primary:"),
            "body: {body}"
        );
        assert!(!body.contains("super-secret"), "body: {body}");
    }

    #[tokio::test]
    async fn dev_mode_strips_client_supplied_verified_actor() {
        let state = make_state(SecurityConfig::default()).await;
        let router = make_router(state);

        let response = route_response(
            router,
            internal_actor_request("/api/actor", "api-token:1234567890abcdef"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert_eq!(body, "unknown");
    }

    #[tokio::test]
    async fn websocket_upgrade_delegates_auth_to_ws_router() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            ..Default::default()
        })
        .await;
        let router = Router::new()
            .route("/ws", get(|| async { "ws-router" }))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth,
            ))
            .with_state(state);

        let response = route_response(router, request("/ws", None)).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn exempt_path_bypasses_auth() {
        let state = make_state(SecurityConfig {
            auth_token: Some("super-secret".to_owned()),
            auth_exempt_paths: vec!["/health".to_owned()],
            ..Default::default()
        })
        .await;
        let router = make_router(state);
        let response = route_response(router, request("/health", None)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn constant_time_eq_matches_normal_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }

    async fn response_body(response: axum::response::Response) -> String {
        let body = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(body) => body,
            Err(error) => panic!("body bytes failed: {error}"),
        };
        match String::from_utf8(body.to_vec()) {
            Ok(text) => text,
            Err(error) => panic!("utf8 body failed: {error}"),
        }
    }

    async fn route_response(router: Router, request: Request<Body>) -> axum::response::Response {
        match router.oneshot(request).await {
            Ok(response) => response,
            Err(error) => match error {},
        }
    }
}
