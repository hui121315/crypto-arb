//! 请求追踪中间件：注入 request id（X-Request-Id）+ 记录耗时。
//!
//! 行为契约：
//! - 若客户端已带安全 `x-request-id`，复用；否则生成 uuid v4 simple format。
//! - 在 response header 回写 `x-request-id`，便于前后端日志关联。
//! - 用 `tracing::info_span` 包裹请求，输出 `method/path/status/elapsed_ms`。
//!
//! 使用方式：
//! ```ignore
//! use axum::middleware::from_fn;
//! router.layer(from_fn(crate::middleware::trace::trace_request))
//! ```

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

const HEADER_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RequestId(String);

impl RequestId {
    fn normalized(value: Option<&str>) -> Self {
        Self(common::request_id::normalize(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// 请求追踪 middleware handler。
///
/// 改为 `async fn` 而非手写 `fn -> Pin<Box<...>>`，让 axum 的 `from_fn` 能正确推导
/// 出 `FromFnLayer<F, S, T>` 的 `T = (Request,)` 元组类型 + `Service<Request>` impl。
/// 手动 desugar 的 `Pin<Box<...>>` 形式不满足 axum 的 trait bound。
pub(crate) async fn trace_request(mut request: Request, next: Next) -> Response {
    let started = std::time::Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    let request_id = RequestId::normalized(
        request
            .headers()
            .get(&HEADER_REQUEST_ID)
            .and_then(|v| v.to_str().ok()),
    );
    let request_id_value = request_id.as_str().to_owned();
    request.extensions_mut().insert(request_id);

    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id_value,
        method = %method,
        path = %path,
    );
    // 不得手持 `span.enter()` guard 跨 await（tracing 明确警告的反模式：
    // await 让出线程后，同线程其他 task 的日志会被错误挂进本 span，span
    // 归属串台污染按 request_id 排障的数据）。`.instrument` 在每次 poll
    // 时正确进出 span。
    let mut response = {
        use tracing::Instrument;
        common::request_id::scope(request_id_value.clone(), next.run(request))
            .instrument(span)
            .await
    };

    let elapsed_ms = started.elapsed().as_millis();
    let status = response.status().as_u16();

    if let Ok(value) = HeaderValue::from_str(&request_id_value) {
        response.headers_mut().insert(&HEADER_REQUEST_ID, value);
    }

    tracing::info!(
        request_id = %request_id_value,
        method = %method,
        path = %path,
        status,
        elapsed_ms,
        "http_request_completed"
    );

    response
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::routing::get;
    use axum::{Extension, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn injects_request_id_when_absent() {
        let router: Router<()> = Router::new()
            .route("/ping", get(|| async { "pong" }))
            .layer(axum::middleware::from_fn(trace_request));

        let response = response_for(router, request("/ping", None)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let id = response_header(&response, "x-request-id");
        assert_eq!(id.len(), 32, "uuid simple format = 32 hex chars");
    }

    #[tokio::test]
    async fn preserves_safe_client_request_id() {
        let router: Router<()> = Router::new()
            .route("/ping", get(|| async { "pong" }))
            .layer(axum::middleware::from_fn(trace_request));

        let response = response_for(router, request("/ping", Some("client-supplied-id"))).await;
        let id = response_header(&response, "x-request-id");
        assert_eq!(id, "client-supplied-id");
    }

    #[tokio::test]
    async fn replaces_unsafe_client_request_id() {
        let router: Router<()> = Router::new()
            .route("/ping", get(|| async { "pong" }))
            .layer(axum::middleware::from_fn(trace_request));

        let response = response_for(router, request("/ping", Some("unsafe/request id"))).await;
        let id = response_header(&response, "x-request-id");
        assert_eq!(id.len(), 32);
        assert_ne!(id, "unsafe/request id");
    }

    #[tokio::test]
    async fn error_response_body_carries_request_id() {
        async fn boom() -> Result<&'static str, common::AppError> {
            Err(common::AppError::NotFound("missing".into()))
        }
        let router: Router<()> = Router::new()
            .route("/boom", get(boom))
            .layer(axum::middleware::from_fn(trace_request));

        let response = response_for(router, request("/boom", Some("rid-9"))).await;
        let text = response_text(response).await;
        assert!(text.contains("\"requestId\":\"rid-9\""), "body: {text}");
    }

    #[tokio::test]
    async fn request_extension_carries_normalized_request_id() {
        async fn extension_id(Extension(request_id): Extension<RequestId>) -> String {
            request_id.as_str().to_owned()
        }

        let router: Router<()> = Router::new()
            .route("/extension", get(extension_id))
            .layer(axum::middleware::from_fn(trace_request));

        let response = response_for(router, request("/extension", Some("rid-extension-1"))).await;
        assert_eq!(response_text(response).await, "rid-extension-1");
    }

    async fn response_text(response: axum::response::Response) -> String {
        let body = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(body) => body,
            Err(error) => panic!("body bytes failed: {error}"),
        };
        match String::from_utf8(body.to_vec()) {
            Ok(text) => text,
            Err(error) => panic!("utf8 body failed: {error}"),
        }
    }

    fn request(path: &str, request_id: Option<&str>) -> HttpRequest<Body> {
        let mut builder = HttpRequest::builder().uri(path);
        if let Some(value) = request_id {
            builder = builder.header("x-request-id", value);
        }
        match builder.body(Body::empty()) {
            Ok(request) => request,
            Err(error) => panic!("request build failed: {error}"),
        }
    }

    async fn response_for(
        router: Router<()>,
        request: HttpRequest<Body>,
    ) -> axum::response::Response {
        match router.oneshot(request).await {
            Ok(response) => response,
            Err(error) => match error {},
        }
    }

    fn response_header(response: &axum::response::Response, name: &str) -> String {
        let Some(value) = response.headers().get(name) else {
            panic!("missing response header {name}");
        };
        match value.to_str() {
            Ok(value) => value.to_owned(),
            Err(error) => panic!("invalid response header {name}: {error}"),
        }
    }
}
