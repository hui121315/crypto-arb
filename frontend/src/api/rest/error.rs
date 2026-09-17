//! REST 客户端错误类型 `ApiError` 与 HTTP 响应 → typed problem 的解析。
//! HTTP 传输动词见 `transport.rs`，Retry-After 解析见 `retry_after.rs`。

use shared_types::{ApiProblem, ApiProblemEnvelope};

use super::retry_after::parse_retry_after_ms;

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub problem: ApiProblem,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.problem.message)?;
        if let Some(status) = self.problem.status {
            write!(f, " · HTTP {status}")?;
        }
        if let Some(request_id) = self.problem.request_id.as_deref() {
            write!(f, " · request_id {request_id}")?;
        }
        if let Some(retry_after_ms) = self.problem.retry_after_ms {
            write!(f, " · retry {retry_after_ms}ms")?;
        }
        Ok(())
    }
}

impl ApiError {
    pub fn from_problem(problem: ApiProblem) -> Self {
        Self { problem }
    }

    pub fn client(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::client_with_request_id(code, message, None)
    }

    pub fn client_with_request_id(
        code: impl Into<String>,
        message: impl Into<String>,
        request_id: Option<String>,
    ) -> Self {
        Self {
            problem: ApiProblem::new(code, message)
                .with_request_id(request_id)
                .with_source("frontend"),
        }
    }

    pub(in crate::api::rest) fn network(
        error: impl std::fmt::Display,
        request_id: Option<String>,
    ) -> Self {
        Self::client_with_request_id("NETWORK", format!("network: {error}"), request_id)
    }

    pub(in crate::api::rest) fn parse(
        error: impl std::fmt::Display,
        request_id: Option<String>,
    ) -> Self {
        Self::client_with_request_id("PARSE", format!("parse: {error}"), request_id)
    }

    pub(in crate::api::rest) fn encode(
        error: impl std::fmt::Display,
        request_id: Option<String>,
    ) -> Self {
        Self::client_with_request_id("ENCODE", format!("encode: {error}"), request_id)
    }

    fn from_http_response(
        status: u16,
        request_id: Option<String>,
        retry_after_ms: Option<u64>,
        body: &str,
    ) -> Self {
        let mut problem = match parse_problem_body(body) {
            Ok(problem) => problem,
            Err(error) => fallback_http_problem(status, body, &error),
        };
        if problem.status.is_none() {
            problem.status = Some(status);
        }
        if problem.request_id.is_none() {
            problem.request_id = request_id;
        }
        if problem.retry_after_ms.is_none() {
            problem.retry_after_ms = retry_after_ms;
        }
        Self { problem }
    }

    fn unreadable_http_error_body(
        status: u16,
        request_id: Option<String>,
        retry_after_ms: Option<u64>,
        error: impl std::fmt::Display,
    ) -> Self {
        let problem = ApiProblem::new(
            "HTTP_ERROR_BODY_UNREADABLE",
            format!("HTTP {status} error body unreadable: {error}"),
        )
        .with_status(status)
        .with_request_id(request_id)
        .with_retry_after_ms(retry_after_ms)
        .with_source("frontend-rest");
        Self { problem }
    }
}

pub(in crate::api::rest) async fn error_from_response(
    resp: gloo_net::http::Response,
    client_request_id: &str,
) -> ApiError {
    let status = resp.status();
    let headers = resp.headers();
    let request_id = headers
        .get("x-request-id")
        .or_else(|| Some(client_request_id.to_owned()));
    let retry_after_ms = headers
        .get("retry-after")
        .and_then(|value| parse_retry_after_ms(&value));
    match resp.text().await {
        Ok(body) => ApiError::from_http_response(status, request_id, retry_after_ms, &body),
        Err(error) => {
            ApiError::unreadable_http_error_body(status, request_id, retry_after_ms, error)
        }
    }
}

fn parse_problem_body(body: &str) -> Result<ApiProblem, serde_json::Error> {
    serde_json::from_str::<ApiProblemEnvelope>(body).map(|envelope| envelope.error)
}

fn fallback_http_problem(status: u16, body: &str, error: &serde_json::Error) -> ApiProblem {
    let body_excerpt = truncate(body, 260);
    let mut problem = ApiProblem::new("HTTP_ERROR", fallback_http_message(status, &body_excerpt));
    problem.details = Some(serde_json::json!({
        "problemParseError": error.to_string(),
        "bodyExcerpt": body_excerpt,
    }));
    problem
}

fn fallback_http_message(status: u16, body: &str) -> String {
    if body.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {body}")
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_backend_problem_and_header_context() {
        let error = ApiError::from_http_response(
            429,
            Some("req-1".into()),
            Some(2_000),
            r#"{"error":{"code":"RATE_LIMITED","message":"slow down"}}"#,
        );

        assert_eq!(error.problem.code, "RATE_LIMITED");
        assert_eq!(error.problem.status, Some(429));
        assert_eq!(error.problem.request_id.as_deref(), Some("req-1"));
        assert_eq!(error.problem.retry_after_ms, Some(2_000));
    }

    #[test]
    fn falls_back_when_body_is_not_problem_json() {
        let error = ApiError::from_http_response(502, None, None, "bad gateway");

        assert_eq!(error.problem.code, "HTTP_ERROR");
        assert_eq!(error.problem.status, Some(502));
        assert!(error.problem.message.contains("bad gateway"));
        assert_eq!(
            error
                .problem
                .details
                .as_ref()
                .and_then(|details| details.get("bodyExcerpt"))
                .and_then(|value| value.as_str()),
            Some("bad gateway")
        );
        assert!(error
            .problem
            .details
            .as_ref()
            .and_then(|details| details.get("problemParseError"))
            .and_then(|value| value.as_str())
            .is_some_and(|message| message.contains("expected value")));
    }

    #[test]
    fn malformed_problem_json_preserves_header_context() {
        let error = ApiError::from_http_response(
            503,
            Some("req-malformed".into()),
            Some(4_000),
            r#"{"error":{"code":"UPSTREAM","message":42}}"#,
        );

        assert_eq!(error.problem.code, "HTTP_ERROR");
        assert_eq!(error.problem.status, Some(503));
        assert_eq!(error.problem.request_id.as_deref(), Some("req-malformed"));
        assert_eq!(error.problem.retry_after_ms, Some(4_000));
        assert!(error.problem.message.contains("HTTP 503"));
        assert_eq!(
            error
                .problem
                .details
                .as_ref()
                .and_then(|details| details.get("bodyExcerpt"))
                .and_then(|value| value.as_str()),
            Some(r#"{"error":{"code":"UPSTREAM","message":42}}"#)
        );
        assert!(error
            .problem
            .details
            .as_ref()
            .and_then(|details| details.get("problemParseError"))
            .and_then(|value| value.as_str())
            .is_some_and(|message| message.contains("invalid type")));
    }

    #[test]
    fn unreadable_error_body_preserves_header_context() {
        let error = ApiError::unreadable_http_error_body(
            503,
            Some("req-17".into()),
            Some(3_000),
            "body stream closed",
        );

        assert_eq!(error.problem.code, "HTTP_ERROR_BODY_UNREADABLE");
        assert_eq!(error.problem.status, Some(503));
        assert_eq!(error.problem.request_id.as_deref(), Some("req-17"));
        assert_eq!(error.problem.retry_after_ms, Some(3_000));
        assert_eq!(error.problem.source.as_deref(), Some("frontend-rest"));
        assert!(error.problem.message.contains("body stream closed"));
    }

    #[test]
    fn frontend_scoped_errors_preserve_client_request_id() {
        let error = ApiError::parse("invalid json", Some("web-test-1".to_owned()));

        assert_eq!(error.problem.code, "PARSE");
        assert_eq!(error.problem.request_id.as_deref(), Some("web-test-1"));
        assert!(error.to_string().contains("request_id web-test-1"));
    }

    #[test]
    fn display_carries_status_and_request_id_for_console_logging() {
        let error = ApiError::from_http_response(
            502,
            Some("req-9".into()),
            None,
            r#"{"error":{"code":"UPSTREAM","message":"gate down"}}"#,
        );

        let rendered = error.to_string();

        assert!(rendered.contains("gate down"));
        assert!(rendered.contains("HTTP 502"));
        assert!(rendered.contains("request_id req-9"));
    }
}
