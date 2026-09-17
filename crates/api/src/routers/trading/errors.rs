use crate::trading_service::SelectAdapterError;
use axum::http::StatusCode;
use common::AppError;
use shared_types::problem::codes;

pub(super) fn map_select_adapter_error(err: SelectAdapterError) -> AppError {
    let message = err.to_string();
    match err {
        SelectAdapterError::OpenOrders => {
            AppError::domain(StatusCode::BAD_REQUEST, codes::ADAPTER_OPEN_ORDERS, message)
        }
        SelectAdapterError::MissingCredentials => AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::ADAPTER_MISSING_CREDENTIALS,
            message,
        ),
        SelectAdapterError::Unsupported(venue) => {
            AppError::domain(StatusCode::BAD_REQUEST, codes::ADAPTER_UNSUPPORTED, message)
                .with_details(serde_json::json!({ "venue": venue }))
        }
        SelectAdapterError::Exchange(inner) => AppError::from(inner),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn unsupported_adapter_maps_to_domain_code_with_venue() {
        let err = map_select_adapter_error(SelectAdapterError::Unsupported("kraken".into()));
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert_eq!(err.code(), "ADAPTER_UNSUPPORTED");
        let text = body_string(err.into_response()).await;
        assert!(
            text.contains("\"code\":\"ADAPTER_UNSUPPORTED\""),
            "body: {text}"
        );
        assert!(text.contains("\"venue\":\"kraken\""), "body: {text}");
    }

    #[test]
    fn open_orders_and_missing_credentials_map_to_distinct_codes() {
        assert_eq!(
            map_select_adapter_error(SelectAdapterError::OpenOrders).code(),
            "ADAPTER_OPEN_ORDERS"
        );
        assert_eq!(
            map_select_adapter_error(SelectAdapterError::MissingCredentials).code(),
            "ADAPTER_MISSING_CREDENTIALS"
        );
    }

    async fn body_string(response: axum::response::Response) -> String {
        let body = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(body) => body,
            Err(error) => panic!("body bytes failed: {error}"),
        };
        match String::from_utf8(body.to_vec()) {
            Ok(text) => text,
            Err(error) => panic!("utf8 body failed: {error}"),
        }
    }
}
