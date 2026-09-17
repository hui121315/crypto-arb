use super::*;

mod evidence;

#[test]
fn runtime_projection_requires_explicit_capability_and_configuration() {
    let mut legacy = operation(VenueOperationStatus::Ok, Some(true), Some(true));
    let ready = VenueRuntimeOperationHealth::from_operation_health(
        VenueRuntimeOperation::PlaceOrder,
        &legacy,
    );
    assert_eq!(ready.capability_status, VenueCapabilityStatus::Supported);
    assert_eq!(
        ready.configuration_status,
        VenueConfigurationStatus::Configured
    );
    assert!(ready.currently_usable);

    legacy.configured = None;
    let unknown_configuration = VenueRuntimeOperationHealth::from_operation_health(
        VenueRuntimeOperation::PlaceOrder,
        &legacy,
    );
    assert_eq!(
        unknown_configuration.configuration_status,
        VenueConfigurationStatus::Unknown
    );
    assert!(!unknown_configuration.currently_usable);

    legacy.configured = Some(true);
    legacy.supported = None;
    let unknown_capability = VenueRuntimeOperationHealth::from_operation_health(
        VenueRuntimeOperation::PlaceOrder,
        &legacy,
    );
    assert_eq!(
        unknown_capability.capability_status,
        VenueCapabilityStatus::Unknown
    );
    assert!(!unknown_capability.currently_usable);
}

#[test]
fn public_operations_do_not_require_credentials_but_still_require_capability() {
    let mut legacy = operation(VenueOperationStatus::Ok, Some(true), None);
    let public = VenueRuntimeOperationHealth::from_operation_health(
        VenueRuntimeOperation::PublicRest,
        &legacy,
    );
    assert_eq!(
        public.configuration_status,
        VenueConfigurationStatus::NotRequired
    );
    assert!(public.currently_usable);

    legacy.supported = None;
    let unknown_capability = VenueRuntimeOperationHealth::from_operation_health(
        VenueRuntimeOperation::PublicRest,
        &legacy,
    );
    assert!(!unknown_capability.currently_usable);
}

#[test]
fn runtime_health_serializes_named_operation_contract() {
    let legacy = operation(VenueOperationStatus::Ok, Some(true), Some(true));
    let place_order = VenueRuntimeOperationHealth::from_operation_health(
        VenueRuntimeOperation::PlaceOrder,
        &legacy,
    );
    let mut health = VenueRuntimeHealth::new("binance", 42);
    health.set_operation(place_order);

    let value = serde_json::to_value(&health);
    assert!(value.is_ok(), "venue runtime health must serialize");
    let Ok(value) = value else {
        return;
    };

    assert_eq!(value["venue"], "binance");
    assert_eq!(value["generatedAtMs"], 42);
    assert_eq!(value["placeOrder"]["operation"], "place_order");
    assert_eq!(value["placeOrder"]["capabilityStatus"], "supported");
    assert_eq!(value["placeOrder"]["configurationStatus"], "configured");
    assert_eq!(value["placeOrder"]["currentlyUsable"], true);
    assert!(value.get("cancelOrder").is_none());
    assert_eq!(
        health
            .operation(VenueRuntimeOperation::PlaceOrder)
            .map(|row| row.currently_usable),
        Some(true)
    );
}

#[test]
fn runtime_health_exposes_every_pr_bx_operation_slot() {
    let operations = [
        (VenueRuntimeOperation::PublicRest, "publicRest"),
        (VenueRuntimeOperation::PublicWs, "publicWs"),
        (VenueRuntimeOperation::PrivateRest, "privateRest"),
        (VenueRuntimeOperation::PrivateWs, "privateWs"),
        (VenueRuntimeOperation::Balance, "balance"),
        (VenueRuntimeOperation::Positions, "positions"),
        (VenueRuntimeOperation::OpenOrders, "openOrders"),
        (VenueRuntimeOperation::PlaceOrder, "placeOrder"),
        (VenueRuntimeOperation::CancelOrder, "cancelOrder"),
        (VenueRuntimeOperation::OrderStream, "orderStream"),
        (VenueRuntimeOperation::Finality, "finality"),
    ];
    let legacy = operation(VenueOperationStatus::Ok, Some(true), Some(true));
    let mut health = VenueRuntimeHealth::new("binance", 42);

    for (operation, _) in operations {
        health.set_operation(VenueRuntimeOperationHealth::from_operation_health(
            operation, &legacy,
        ));
    }

    let value = serde_json::to_value(&health);
    assert!(value.is_ok(), "all runtime operation slots must serialize");
    let Ok(value) = value else {
        return;
    };
    for (operation, field) in operations {
        assert!(
            health.operation(operation).is_some(),
            "missing {field} slot"
        );
        assert!(value.get(field).is_some(), "missing {field} wire field");
    }
}

#[test]
fn runtime_health_defaults_new_separation_fields_fail_closed() {
    let payload = r#"{
        "operation":"private_rest",
        "status":"ok",
        "source":"legacy-runtime",
        "observedAtMs":7
    }"#;

    let health: Result<VenueRuntimeOperationHealth, _> = serde_json::from_str(payload);
    assert!(health.is_ok(), "compatibility payload must deserialize");
    let Ok(health) = health else {
        return;
    };

    assert_eq!(health.capability_status, VenueCapabilityStatus::Unknown);
    assert_eq!(
        health.configuration_status,
        VenueConfigurationStatus::Unknown
    );
    assert!(!health.currently_usable);
}

#[test]
fn legacy_operation_health_wire_contract_is_unchanged() {
    let payload = r#"{
        "venue":"binance",
        "operation":"place_order",
        "status":"ok",
        "source":"legacy",
        "message":"ok",
        "supported":true,
        "configured":true,
        "observedAtMs":9
    }"#;

    let legacy: Result<VenueOperationHealth, _> = serde_json::from_str(payload);
    assert!(legacy.is_ok(), "legacy operation health must deserialize");
    let Ok(legacy) = legacy else {
        return;
    };

    assert_eq!(legacy.supported, Some(true));
    assert_eq!(legacy.configured, Some(true));
    assert!(legacy.is_currently_usable());
}

#[test]
fn snapshot_groups_normalized_venues_into_all_runtime_slots() {
    let rows = [
        row(" Binance ", OP_REST_ORDERBOOKS, VenueOperationStatus::Ok, 1),
        row("BINANCE", OP_WS_TICKER, VenueOperationStatus::Ok, 2),
        row("binance", OP_PRIVATE_READ, VenueOperationStatus::Ok, 3),
        row(
            "binance",
            OP_PRIVATE_WS_SESSION,
            VenueOperationStatus::Ok,
            4,
        ),
        row("binance", OP_BALANCE, VenueOperationStatus::Ok, 5),
        row("binance", OP_POSITIONS, VenueOperationStatus::Ok, 6),
        row(
            "binance",
            "credential_probe:open_orders_read",
            VenueOperationStatus::Ok,
            7,
        ),
        row("binance", OP_ORDER_WRITE, VenueOperationStatus::Ok, 8),
        row(
            "binance",
            OP_PRIVATE_WS_ORDER_STREAM,
            VenueOperationStatus::Ok,
            9,
        ),
        row("binance", OP_ORDER_FINALITY, VenueOperationStatus::Ok, 10),
    ];

    let snapshot = VenueRuntimeHealthSnapshot::from_operation_rows(&rows, 100);
    let _: &crate::VenueRuntimeHealthSnapshot = &snapshot;
    let _: &crate::VenueRuntimeHealth = &snapshot.venues[0];
    let _: Option<&crate::VenueRuntimeOperationHealth> = snapshot.venues[0].public_rest.as_ref();
    let _: crate::VenueRuntimeOperation = VenueRuntimeOperation::PublicRest;
    let _: crate::VenueCapabilityStatus = VenueCapabilityStatus::Supported;
    let _: crate::VenueConfigurationStatus = VenueConfigurationStatus::Configured;

    assert_eq!(snapshot.venue_count, 1);
    assert_eq!(snapshot.operation_count, ALL_RUNTIME_OPERATIONS.len());
    assert_eq!(
        snapshot.currently_usable_count,
        ALL_RUNTIME_OPERATIONS.len()
    );
    assert_eq!(snapshot.attention_count, 0);
    assert_eq!(snapshot.venues[0].venue, "binance");
    for operation in ALL_RUNTIME_OPERATIONS {
        assert!(snapshot.venues[0].operation(operation).is_some());
    }
}

#[test]
fn snapshot_duplicate_raw_operation_prefers_newest_observation() {
    let rows = [
        row(
            "binance",
            OP_REST_ORDERBOOKS,
            VenueOperationStatus::Blocked,
            10,
        ),
        row("BINANCE", OP_REST_ORDERBOOKS, VenueOperationStatus::Ok, 20),
    ];

    let snapshot = VenueRuntimeHealthSnapshot::from_operation_rows(&rows, 30);
    let public_rest = snapshot.venues[0].public_rest.as_ref();
    assert!(public_rest.is_some(), "public REST projection must exist");
    let Some(public_rest) = public_rest else {
        return;
    };

    assert_eq!(public_rest.status, VenueOperationStatus::Ok);
    assert_eq!(public_rest.observed_at_ms, 20);
}

#[test]
fn snapshot_collapsed_slot_prefers_worst_current_operation() {
    let rows = [
        row(
            "binance",
            OP_REST_ORDERBOOKS,
            VenueOperationStatus::Blocked,
            10,
        ),
        row(
            "binance",
            OP_REST_FUNDING_RATES,
            VenueOperationStatus::Ok,
            20,
        ),
    ];

    let snapshot = VenueRuntimeHealthSnapshot::from_operation_rows(&rows, 30);
    let public_rest = snapshot.venues[0].public_rest.as_ref();
    assert!(public_rest.is_some(), "public REST projection must exist");
    let Some(public_rest) = public_rest else {
        return;
    };

    assert_eq!(public_rest.status, VenueOperationStatus::Blocked);
    assert_eq!(public_rest.observed_at_ms, 10);
    assert!(!public_rest.currently_usable);
}

#[test]
fn snapshot_ties_are_evidence_first_and_input_order_independent() {
    let plain = row(
        "binance",
        OP_ORDER_RECONCILIATION,
        VenueOperationStatus::Warn,
        10,
    );
    let mut evidenced = row("binance", OP_ORDER_FINALITY, VenueOperationStatus::Warn, 10);
    evidenced.error = Some("finality failed".to_owned());
    evidenced.problem = Some(
        ApiProblem::new("FINALITY_FAILED", "finality failed")
            .with_request_id(Some("request-10".to_owned())),
    );

    let forward =
        VenueRuntimeHealthSnapshot::from_operation_rows(&[plain.clone(), evidenced.clone()], 20);
    let reverse = VenueRuntimeHealthSnapshot::from_operation_rows(&[evidenced, plain], 20);

    assert_eq!(forward, reverse);
    let finality = forward.venues[0].finality.as_ref();
    assert!(finality.is_some(), "finality projection must exist");
    let Some(finality) = finality else {
        return;
    };
    assert_eq!(finality.last_error.as_deref(), Some("finality failed"));
    assert_eq!(finality.request_id.as_deref(), Some("request-10"));
    assert_eq!(
        finality
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("FINALITY_FAILED")
    );
}

const ALL_RUNTIME_OPERATIONS: [VenueRuntimeOperation; 11] = [
    VenueRuntimeOperation::PublicRest,
    VenueRuntimeOperation::PublicWs,
    VenueRuntimeOperation::PrivateRest,
    VenueRuntimeOperation::PrivateWs,
    VenueRuntimeOperation::Balance,
    VenueRuntimeOperation::Positions,
    VenueRuntimeOperation::OpenOrders,
    VenueRuntimeOperation::PlaceOrder,
    VenueRuntimeOperation::CancelOrder,
    VenueRuntimeOperation::OrderStream,
    VenueRuntimeOperation::Finality,
];

fn operation(
    status: VenueOperationStatus,
    supported: Option<bool>,
    configured: Option<bool>,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: "binance".to_owned(),
        operation: "place_order".to_owned(),
        status,
        source: "runtime-test".to_owned(),
        message: "runtime test".to_owned(),
        supported,
        configured,
        requested: None,
        rows: None,
        freshness_ms: Some(10),
        retry_after_ms: None,
        latency_ms: Some(5),
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 100,
    }
}

fn row(
    venue: &str,
    operation_name: &str,
    status: VenueOperationStatus,
    observed_at_ms: i64,
) -> VenueOperationHealth {
    let mut row = operation(status, Some(true), Some(true));
    row.venue = venue.to_owned();
    row.operation = operation_name.to_owned();
    row.observed_at_ms = observed_at_ms;
    row.error = (status != VenueOperationStatus::Ok).then(|| "operation requires attention".into());
    row
}

fn evidence(request_id: Option<&str>) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "GET".to_owned(),
        path: "/private".to_owned(),
        checked_at: "2026-07-11".to_owned(),
        doc_version: "test".to_owned(),
        schema_hash: "test".to_owned(),
        fixture_id: "test".to_owned(),
        parser_test: "test".to_owned(),
        request_builder_test: "test".to_owned(),
        auth_kind: "signed".to_owned(),
        request_id: request_id.map(str::to_owned),
        request_context: Vec::new(),
        doc_urls: Vec::new(),
        use_cases: Vec::new(),
        data_kinds: Vec::new(),
        rate_scopes: Vec::new(),
        weight: 1,
    }
}
