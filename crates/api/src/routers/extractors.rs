use async_trait::async_trait;
use axum::extract::{rejection::JsonRejection, FromRequest, Json, Request};
use common::AppError;
use serde::de::DeserializeOwned;
use shared_types::problem::codes;

pub(crate) struct ApiJson<T>(pub(crate) T);

#[async_trait]
impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|rejection| json_rejection_problem(&rejection))
    }
}

fn json_rejection_problem(rejection: &JsonRejection) -> AppError {
    let status = rejection.status();
    let kind = json_rejection_kind(rejection);
    AppError::domain(
        status,
        codes::REQUEST_BODY_INVALID,
        "invalid JSON request body",
    )
    .with_details(serde_json::json!({
        "extractor": "Json",
        "kind": kind,
        "rejection": rejection.body_text(),
    }))
}

fn json_rejection_kind(rejection: &JsonRejection) -> &'static str {
    match rejection {
        JsonRejection::JsonDataError(_) => "data",
        JsonRejection::JsonSyntaxError(_) => "syntax",
        JsonRejection::MissingJsonContentType(_) => "content_type",
        JsonRejection::BytesRejection(_) => "bytes",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Method, Request, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use serde::Deserialize;
    use shared_types::ApiProblemEnvelope;
    use tower::ServiceExt;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RequiredPayload {
        active: bool,
        reason: String,
    }

    #[tokio::test]
    async fn json_data_error_returns_typed_problem_with_request_id() {
        let router: Router<()> =
            Router::new()
                .route("/json", post(accept_json))
                .layer(axum::middleware::from_fn(
                    crate::middleware::trace::trace_request,
                ));
        let response = route_response(
            router,
            Request::builder()
                .method(Method::POST)
                .uri("/json")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-request-id", "extractor-rid")
                .body(Body::from(r#"{"active":true}"#)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let envelope = problem_envelope(response).await;
        assert_eq!(envelope.error.code, codes::REQUEST_BODY_INVALID);
        assert_eq!(envelope.error.status, Some(422));
        assert_eq!(envelope.error.request_id.as_deref(), Some("extractor-rid"));
        assert_eq!(
            envelope
                .error
                .details
                .as_ref()
                .and_then(|details| details.get("extractor"))
                .and_then(|value| value.as_str()),
            Some("Json")
        );
        assert_eq!(
            envelope
                .error
                .details
                .as_ref()
                .and_then(|details| details.get("kind"))
                .and_then(|value| value.as_str()),
            Some("data")
        );
    }

    async fn accept_json(ApiJson(payload): ApiJson<RequiredPayload>) -> StatusCode {
        let _ = (payload.active, payload.reason);
        StatusCode::NO_CONTENT
    }

    async fn route_response(
        router: Router<()>,
        request: Result<Request<Body>, axum::http::Error>,
    ) -> axum::response::Response {
        let request = match request {
            Ok(request) => request,
            Err(error) => panic!("request build failed: {error}"),
        };
        match router.oneshot(request).await {
            Ok(response) => response,
            Err(error) => match error {},
        }
    }

    async fn problem_envelope(response: axum::response::Response) -> ApiProblemEnvelope {
        let body = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(body) => body,
            Err(error) => panic!("body bytes failed: {error}"),
        };
        match serde_json::from_slice(&body) {
            Ok(envelope) => envelope,
            Err(error) => panic!("problem envelope decode failed: {error}"),
        }
    }
}
