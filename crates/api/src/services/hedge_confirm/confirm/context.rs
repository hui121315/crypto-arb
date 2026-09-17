use common::AppError;
use shared_types::{
    ExecutionEnvironment, ExecutionMode, HedgeConfirmContext, HedgeConfirmRequest,
    HedgePreviewResponse,
};

pub(super) fn confirm_request_context(
    opportunity_id: &str,
    request: &HedgeConfirmRequest,
) -> HedgeConfirmContext {
    HedgeConfirmContext {
        opportunity_id: opportunity_id.to_owned(),
        idempotency_key: request.idempotency_key.clone(),
        ticket_id: request.ticket_id.clone(),
        ..HedgeConfirmContext::default()
    }
}

pub(super) fn confirm_preview_context(
    preview: &HedgePreviewResponse,
    idempotency_key: &str,
) -> HedgeConfirmContext {
    let environment = if preview.long_leg.mode == ExecutionMode::Live
        || preview.short_leg.mode == ExecutionMode::Live
    {
        ExecutionEnvironment::Live
    } else {
        ExecutionEnvironment::Paper
    };
    HedgeConfirmContext {
        opportunity_id: preview.opportunity_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        ticket_id: Some(preview.ticket.ticket_id.clone()),
        environment: Some(environment),
        long_venue: Some(preview.long_leg.exchange.clone()),
        short_venue: Some(preview.short_leg.exchange.clone()),
        ..HedgeConfirmContext::default()
    }
}

pub(super) fn with_confirm_context(error: AppError, context: &HedgeConfirmContext) -> AppError {
    match error {
        AppError::Domain {
            status,
            code,
            message,
            details,
        } => {
            let mut object = match details {
                Some(serde_json::Value::Object(object)) => object,
                Some(value) => serde_json::Map::from_iter([("originalDetails".to_owned(), value)]),
                None => serde_json::Map::new(),
            };
            object.insert("confirmContext".into(), serde_json::json!(context));
            AppError::Domain {
                status,
                code,
                message,
                details: Some(serde_json::Value::Object(object)),
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn with_confirm_context_preserves_non_object_details() {
        let context = HedgeConfirmContext {
            opportunity_id: "opp-test".into(),
            ..HedgeConfirmContext::default()
        };
        let error = AppError::domain(StatusCode::CONFLICT, "UPSTREAM", "route unavailable")
            .with_details(serde_json::json!("venue offline"));

        let details = match with_confirm_context(error, &context) {
            AppError::Domain { details, .. } => details,
            _ => None,
        };

        assert_eq!(
            details
                .as_ref()
                .and_then(|value| value.get("originalDetails")),
            Some(&serde_json::json!("venue offline"))
        );
        assert_eq!(
            details
                .as_ref()
                .and_then(|value| value.get("confirmContext"))
                .and_then(|value| value.get("opportunityId")),
            Some(&serde_json::json!("opp-test"))
        );
    }
}
