use super::super::actions::{
    api_base_validate_success_message, require_api_version, ApiBaseValidationResult,
};

#[test]
fn api_base_validate_success_message_reports_health_slots() {
    let result = ApiBaseValidationResult {
        health: health_fixture(7, 8, 5, vec!["bitget".to_owned()]),
        ws_ticket_checked: false,
    };

    let message = api_base_validate_success_message("http://10.0.0.2:8000", &result);

    assert!(message.contains("http://10.0.0.2:8000"));
    assert!(message.contains("version 1.2.3-test"));
    assert!(message.contains("API 7/8"));
    assert!(message.contains("频道 5"));
    assert!(message.contains("断开 1"));
    assert!(message.contains("本次不探测 /api/auth/ws-ticket"));
}

#[test]
fn api_base_validate_rejects_health_without_api_version() {
    let health = health_fixture(1, 1, 1, Vec::new());
    let health = shared_types::SystemHealth {
        api_version: String::new(),
        ..health
    };

    let result = require_api_version(&health);

    assert_eq!(
        result
            .as_ref()
            .err()
            .map(|error| error.problem.code.as_str()),
        Some("API_VERSION_MISSING")
    );
    assert!(result
        .as_ref()
        .err()
        .is_some_and(|error| error.problem.message.contains("apiVersion")));
}

#[test]
fn api_base_validate_success_message_reports_ws_ticket_without_secret() {
    let result = ApiBaseValidationResult {
        health: health_fixture(1, 1, 2, Vec::new()),
        ws_ticket_checked: true,
    };

    let message = api_base_validate_success_message("http://10.0.0.2:8000", &result);

    assert!(message.contains("/api/auth/ws-ticket 探测通过"));
    assert!(!message.contains("ticket-"));
    assert!(!message.contains("Bearer"));
}

fn health_fixture(
    healthy: u32,
    total: u32,
    channels: u32,
    disconnected: Vec<String>,
) -> shared_types::SystemHealth {
    shared_types::SystemHealth {
        api_version: "1.2.3-test".to_owned(),
        api: shared_types::ApiHealthSlot {
            healthy,
            total,
            failed_venues: Vec::new(),
        },
        ws: shared_types::WsHealthSlot {
            channels,
            disconnected,
        },
        order_elapsed_ms: None,
        risk: shared_types::RiskStatusSlot::Ok,
        net_delta_usd: 0.0,
        net_delta_pct_of_nav: 0.0,
        next_funding: None,
        updated_at_ms: 0,
        degraded: false,
        problems: Vec::new(),
    }
}
