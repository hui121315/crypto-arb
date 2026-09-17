use super::audit_context::resource_kind;
use super::audit_log::audit_action;
use crate::middleware::audit::{self, AuditCorrelation, AuditEvent, AuditEventContext};
use crate::route_specs::{self, RouteEndpointSpec};
use axum::http::{Method, StatusCode};
use common::AppError;
use serde_json::json;
use shared_types::{problem::codes, ApiProblem};

const DENIED_ACTOR: &str = "unknown";
const DENIED_OUTCOME: &str = "denied";
const UNAUTHORIZED_STATUS: u16 = 401;

pub(crate) fn record_auth_denial(
    method: &Method,
    request_path: &str,
    problem: &ApiProblem,
) -> Result<(), AppError> {
    let Some(event) = auth_denial_event(method, request_path, problem) else {
        return Ok(());
    };
    audit::record_durable(&event).map_err(|reason| audit_write_error(&event, reason.as_str()))
}

fn auth_denial_event(
    method: &Method,
    request_path: &str,
    problem: &ApiProblem,
) -> Option<AuditEvent<'static>> {
    let endpoint = high_risk_endpoint(method, request_path)?;
    let kind = endpoint.action_run_kind()?;
    let status = problem.status.unwrap_or(UNAUTHORIZED_STATUS);
    let request_id = problem
        .request_id
        .clone()
        .or_else(common::request_id::current);
    let mut context = AuditEventContext::for_actor(DENIED_ACTOR);
    context.method = Some(endpoint.methods().to_owned());
    context.path = Some(endpoint.path().to_owned());
    context.status = Some(status);
    context.action_kind = Some(kind);
    context.resource_kind = Some(resource_kind(kind));
    context.problem_code = normalized(problem.code.as_str());

    Some(
        AuditEvent::now(
            DENIED_ACTOR,
            audit_action(kind),
            endpoint.path(),
            DENIED_OUTCOME,
            json!({
                "requestId": request_id.as_deref(),
                "requestPath": request_path,
                "problem": {
                    "code": problem.code,
                    "status": status,
                    "source": problem.source,
                    "recoveryAction": problem.recovery_action,
                },
            }),
        )
        .with_context(context)
        .with_correlation(AuditCorrelation::request(request_id)),
    )
}

fn high_risk_endpoint(method: &Method, request_path: &str) -> Option<RouteEndpointSpec> {
    route_specs::route_specs()
        .iter()
        .copied()
        .flat_map(|spec| spec.endpoints().iter().copied())
        .find(|endpoint| {
            endpoint.action_run_kind().is_some()
                && endpoint.auth_policy() == "bearer"
                && endpoint.risk() == "high"
                && matches!(endpoint.audit_policy(), "action_run" | "secret_mutation")
                && endpoint
                    .methods()
                    .split(',')
                    .any(|candidate| candidate == method.as_str())
                && route_path_matches(endpoint.path(), request_path)
        })
}

fn route_path_matches(template: &str, actual: &str) -> bool {
    let mut template_segments = template.split('/');
    let mut actual_segments = actual.split('/');
    loop {
        match (template_segments.next(), actual_segments.next()) {
            (None, None) => return true,
            (Some(template), Some(actual)) if template.starts_with(':') && !actual.is_empty() => {}
            (Some(template), Some(actual)) if template == actual => {}
            _ => return false,
        }
    }
}

fn normalized(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn audit_write_error(event: &AuditEvent<'_>, reason: &str) -> AppError {
    AppError::domain(
        StatusCode::SERVICE_UNAVAILABLE,
        codes::AUDIT_STORAGE_WRITE_FAILED,
        "authentication denial could not be durably audited",
    )
    .with_details(json!({
        "action": event.action,
        "outcome": event.outcome,
        "path": event.context.path.as_deref(),
        "requestId": event.correlation.request_id.as_deref(),
        "reason": reason,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use shared_types::ActionRunKind;
    use std::collections::HashSet;

    fn denied_problem() -> ApiProblem {
        ApiProblem::new(
            "UNAUTHORIZED",
            "authentication required; provide a valid Bearer token and retry",
        )
        .with_status(UNAUTHORIZED_STATUS)
        .with_request_id(Some("auth-denied-request".to_owned()))
        .with_source("api.auth")
    }

    #[test]
    fn auth_denial_matrix_covers_every_high_risk_action_route() {
        let problem = denied_problem();
        let mut kinds = HashSet::new();
        let mut route_count = 0;

        for endpoint in route_specs::route_specs()
            .iter()
            .copied()
            .flat_map(|spec| spec.endpoints().iter().copied())
            .filter(|endpoint| endpoint.action_run_kind().is_some())
        {
            let kind = endpoint
                .action_run_kind()
                .unwrap_or_else(|| panic!("missing action kind for {}", endpoint.path()));
            let sample_path = sample_path(endpoint.path());
            for raw_method in endpoint.methods().split(',') {
                let method = Method::from_bytes(raw_method.as_bytes())
                    .unwrap_or_else(|error| panic!("invalid method {raw_method}: {error}"));
                let event = auth_denial_event(&method, sample_path.as_str(), &problem)
                    .unwrap_or_else(|| {
                        panic!("missing denial event for {raw_method} {sample_path}")
                    });

                assert_eq!(event.action, audit_action(kind));
                assert_eq!(event.resource, endpoint.path());
                assert_eq!(event.outcome, DENIED_OUTCOME);
                assert_eq!(event.context.action_kind, Some(kind));
                assert_eq!(event.context.resource_kind, Some(resource_kind(kind)));
                assert_eq!(event.context.status, Some(UNAUTHORIZED_STATUS));
                assert_eq!(
                    event.correlation.request_id.as_deref(),
                    Some("auth-denied-request")
                );
                kinds.insert(kind);
                route_count += 1;
            }
        }

        assert_eq!(route_count, 21);
        assert_eq!(kinds.len(), 21);
        assert!(!kinds.contains(&ActionRunKind::AutomationLiveUnlock));
    }

    #[test]
    fn auth_denial_keeps_dynamic_route_and_safe_problem_correlation() {
        let problem = denied_problem();
        let event = auth_denial_event(
            &Method::POST,
            "/api/trading/orders/order-42/cancel",
            &problem,
        )
        .unwrap_or_else(|| panic!("dynamic high-risk route did not resolve"));
        let value = serde_json::to_value(event)
            .unwrap_or_else(|error| panic!("audit event encode failed: {error}"));

        assert_eq!(value["action"], "trading.order.cancel");
        assert_eq!(value["actionKind"], "trading_order_cancel");
        assert_eq!(value["resourceKind"], "order");
        assert_eq!(value["path"], "/api/trading/orders/:id/cancel");
        assert_eq!(
            value["detail"]["requestPath"],
            "/api/trading/orders/order-42/cancel"
        );
        assert_eq!(value["problemCode"], "UNAUTHORIZED");
        assert_eq!(value["requestId"], "auth-denied-request");
        assert!(!value.to_string().contains("wrong-bearer-secret"));

        assert!(auth_denial_event(
            &Method::GET,
            "/api/trading/orders/order-42/cancel",
            &problem
        )
        .is_none());
        assert!(auth_denial_event(&Method::GET, "/api/trading/status", &problem).is_none());
    }

    fn sample_path(template: &str) -> String {
        template
            .split('/')
            .map(|segment| {
                if segment.starts_with(':') {
                    "auth-denied-fixture"
                } else {
                    segment
                }
            })
            .collect::<Vec<_>>()
            .join("/")
    }
}
