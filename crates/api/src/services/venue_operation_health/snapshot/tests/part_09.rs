#[test]
fn nav_storage_row_maps_recent_io_failure_to_blocked_problem() {
    let mut health = nav_health(true);
    health.append_error_total = 1;
    health.last_error_at_ms = Some(9_000);
    health.last_error = Some("sqlite open failed".to_owned());

    let row = nav_storage_row(&health, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.requested, Some(1));
    assert_eq!(row.rows, Some(0));
    assert_eq!(row.freshness_ms, Some(1_000));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::NAV_STORAGE_IO_FAILED)
    );
    assert!(row
        .problem
        .as_ref()
        .and_then(|problem| problem.details.as_ref())
        .and_then(|details| details.pointer("/storageContract/degradedReasons"))
        .is_some_and(|reasons| reasons.as_array().is_some_and(|reasons| {
            reasons.iter().any(|reason| reason == "write_failed")
        })));
}

#[test]
fn nav_storage_row_warns_on_recovered_error() {
    let mut health = nav_health(true);
    health.load_success_total = 1;
    health.load_error_total = 1;
    health.last_error_at_ms = Some(9_000);
    health.last_success_at_ms = Some(9_500);
    health.sample_count = 4;
    health.last_error = Some("previous sqlite error".to_owned());

    let row = nav_storage_row(&health, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.requested, Some(2));
    assert_eq!(row.rows, Some(4));
    assert_eq!(row.freshness_ms, Some(500));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::NAV_STORAGE_IO_FAILED)
    );
}

#[test]
fn nav_storage_row_maps_success_to_ok_without_problem() {
    let mut health = nav_health(true);
    health.load_success_total = 1;
    health.append_success_total = 2;
    health.last_success_at_ms = Some(9_500);
    health.schema_version = Some(1);
    health.sample_count = 3;
    health.latest_sample_status = Some("ok".to_owned());
    health.latest_sample_source = Some("position_margin".to_owned());

    let row = nav_storage_row(&health, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.requested, Some(3));
    assert_eq!(row.rows, Some(3));
    assert_eq!(row.freshness_ms, Some(500));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "schema_version=1")
    }));
    assert!(
        row.evidence
            .as_ref()
            .is_some_and(|evidence| evidence.schema_hash.starts_with("fnv1a64:"))
    );
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item.starts_with("schema_hash=fnv1a64:"))
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item.starts_with("migration_checksum=fnv1a64:"))
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "migration_id=20260605_portfolio_nav")
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "migration_applied=true")
    }));
    assert!(row.problem.is_none());
    assert!(row.error.is_none());
}

#[test]
fn nav_storage_row_warns_on_unknown_sample_source() {
    let mut health = nav_health(true);
    health.latest_sample_status = Some("unknown".to_owned());
    health.latest_sample_source = Some("no_positions".to_owned());
    health.latest_sample_problem = Some("no position rows; NAV source is unknown".to_owned());
    health.latest_sample_at_ms = Some(9_900);

    let row = nav_storage_row(&health, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert!(row.message.contains("NAV 样本来源未知"));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::NAV_STORAGE_UNAVAILABLE)
    );
}

fn credential_probe(
    kind: &str,
    status: VenueCredentialProbeStatus,
    scope: &str,
    source: &str,
) -> VenueCredentialProbe {
    VenueCredentialProbe {
        kind: kind.to_owned(),
        status,
        scope: scope.to_owned(),
        source: source.to_owned(),
        message: format!("{kind} probe"),
        checked_at_ms: 1_000,
        request_id: None,
    }
}

fn http_outcome(
    outcome: &str,
    retry_after_ms: Option<u64>,
    last_observed_at_ms: i64,
    request_total: u64,
) -> HttpOutcomeMetricSnapshot {
    HttpOutcomeMetricSnapshot {
        exchange: "binance".to_owned(),
        method: "GET".to_owned(),
        path: "/fapi/v1/depth".to_owned(),
        endpoint_evidence: exchange::endpoint_evidence(
            "binance",
            exchange::HttpMethod::Get,
            "/fapi/v1/depth",
        ),
        outcome: outcome.to_owned(),
        status_code: if outcome == "success" {
            Some(200)
        } else {
            Some(429)
        },
        request_total,
        retry_total: if outcome == "success" {
            0
        } else {
            request_total
        },
        latency_ms_total: 100,
        retry_after_ms_total: retry_after_ms.unwrap_or(0),
        latency_buckets: vec![
            exchange::HttpLatencyBucketSnapshot {
                le_ms: 10,
                count: request_total,
            },
            exchange::HttpLatencyBucketSnapshot {
                le_ms: 25,
                count: request_total,
            },
        ],
        latency_p95_ms: Some(25),
        last_latency_ms: 8,
        last_retry_after_ms: retry_after_ms,
        last_request_id: Some(format!("req-{outcome}")),
        last_request_context: vec!["symbol=BTCUSDT".into(), "contract_code=BTC-USDT".into()],
        last_observed_at_ms,
    }
}

fn credential(private_read: bool, configured: bool) -> VenueCredentialStatus {
    VenueCredentialStatus {
        venue: "binance".to_owned(),
        label: "Binance".to_owned(),
        fields: vec![VenueCredentialField {
            key: "api_key".to_owned(),
            label: "API Key".to_owned(),
            env_key: "BINANCE_API_KEY".to_owned(),
            configured,
            secret: true,
            required: true,
            source: if configured {
                shared_types::VenueCredentialFieldSource::Runtime
            } else {
                shared_types::VenueCredentialFieldSource::Missing
            },
        }],
        public_market: true,
        private_read,
        testnet_write: false,
        live_write: private_read,
        note: String::new(),
        missing_fields: if configured { Vec::new() } else { vec!["api_key".to_owned()] },
        validation_evidence: None,
    }
}

fn run_finality_health(venue: &str, status: VenueOperationStatus) -> RunFinalityRuntimeHealth {
    RunFinalityRuntimeHealth {
        venue: venue.to_owned(),
        status,
        message: "订单终态回查完成".to_owned(),
        requested: Some(0),
        rows: Some(0),
        freshness_ms: Some(0),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 10,
        scanned_order_count: 0,
        refreshed_order_count: 0,
        remote_missing_count: 0,
        skipped_terminal_count: 0,
        refresh_failure_count: 0,
        publish_failure_count: 0,
        sample_problem: None,
    }
}

fn live_order_proof_health(
    venue: &str,
    status: VenueOperationStatus,
) -> LiveOrderProofRuntimeHealth {
    let place = live_order_proof_sample(venue, "adapter_ack", 10);
    let cancel = live_order_proof_sample(venue, "order_query", 12);
    LiveOrderProofRuntimeHealth {
        venue: venue.to_owned(),
        status,
        message: "live 下单 ack 与撤单终态远程证明均已闭环".to_owned(),
        request_id: Some("req-live-proof".to_owned()),
        requested: Some(2),
        rows: Some(2),
        freshness_ms: Some(8),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 12,
        place_proof: Some(place),
        cancel_request: Some(live_order_proof_sample(venue, "adapter_ack", 11)),
        cancel_finality: Some(cancel),
        last_problem: None,
        place_ack_count: 1,
        cancel_requested_count: 1,
        cancel_finality_count: 1,
    }
}

fn live_order_proof_sample(venue: &str, source: &str, checked_at_ms: i64) -> LiveOrderProofSample {
    LiveOrderProofSample {
        venue: venue.to_owned(),
        symbol: "BTCUSDT".to_owned(),
        internal_order_id: "internal-1".to_owned(),
        exchange_order_id: Some("exchange-1".to_owned()),
        client_order_id: Some("client-1".to_owned()),
        source: source.to_owned(),
        checked_at_ms,
        request_id: Some("req-live-proof".to_owned()),
        native_transport: Some("hyperliquid_ws_post".to_owned()),
        native_request_id: Some("257".to_owned()),
        native_response_id: Some("257".to_owned()),
    }
}
