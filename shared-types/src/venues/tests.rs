use super::*;

fn op_health(
    status: VenueOperationStatus,
    supported: Option<bool>,
    configured: Option<bool>,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: "binance".to_owned(),
        operation: "place_order".to_owned(),
        status,
        source: String::new(),
        message: String::new(),
        supported,
        configured,
        requested: None,
        rows: None,
        freshness_ms: None,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 0,
    }
}
#[test]
fn ok_supported_configured_is_currently_usable() {
    assert!(op_health(VenueOperationStatus::Ok, Some(true), Some(true)).is_currently_usable());
    assert!(op_health(VenueOperationStatus::Ok, None, None).is_currently_usable());
}

#[test]
fn non_ok_status_is_not_currently_usable() {
    for status in [
        VenueOperationStatus::Warn,
        VenueOperationStatus::Blocked,
        VenueOperationStatus::Unknown,
        VenueOperationStatus::Unsupported,
    ] {
        assert!(!op_health(status, Some(true), Some(true)).is_currently_usable());
    }
}

#[test]
fn configured_but_unsupported_is_not_currently_usable() {
    assert!(!op_health(VenueOperationStatus::Ok, Some(false), Some(true)).is_currently_usable());
}

#[test]
fn supported_but_unconfigured_is_not_currently_usable() {
    assert!(!op_health(VenueOperationStatus::Ok, Some(true), Some(false)).is_currently_usable());
}

#[test]
fn capability_supported_reflects_only_static_flag() {
    assert!(op_health(VenueOperationStatus::Blocked, Some(true), None).capability_supported());
    assert!(op_health(VenueOperationStatus::Ok, None, None).capability_supported());
    assert!(!op_health(VenueOperationStatus::Ok, Some(false), None).capability_supported());
}

#[test]
fn detects_hyperliquid_builder_venue() {
    assert!(is_hyperliquid_builder_venue("hyperliquid:xyz"));
    assert!(is_hyperliquid_builder_venue(" Hyperliquid:XYZ "));
    assert!(is_hyperliquid_builder_venue("hyperliquid:km"));
    assert!(!is_hyperliquid_builder_venue("hyperliquid"));
    assert!(!is_hyperliquid_builder_venue("hyperliquid: "));
    assert!(!is_hyperliquid_builder_venue("binance:xyz"));
}

#[test]
fn venue_name_mapping_covers_builder_dex() {
    assert_eq!(
        VenueId::from_exchange_name("hyperliquid:xyz"),
        Some(VenueId::Hyperliquid)
    );
    assert_eq!(
        VenueId::from_exchange_name(" Hyperliquid:XYZ "),
        Some(VenueId::Hyperliquid)
    );
    assert_eq!(VenueId::from_exchange_name("okx-live"), Some(VenueId::Okx));
    assert_eq!(VenueId::from_exchange_name("okx_live"), Some(VenueId::Okx));
    assert_eq!(VenueId::from_exchange_name("OKX-LIVE"), Some(VenueId::Okx));
    assert_eq!(VenueId::from_exchange_name("kraken"), Some(VenueId::Kraken));
    assert_eq!(
        VenueId::from_exchange_name("gate-crossex"),
        Some(VenueId::GateCrossEx)
    );
    assert_eq!(
        VenueId::from_exchange_name("crossex"),
        Some(VenueId::GateCrossEx)
    );
    assert_eq!(VenueId::from_exchange_name("unknown"), None);
}

#[test]
fn normalizes_venue_names_for_routing_keys() {
    assert_eq!(
        normalized_venue_name(" Hyperliquid:XYZ "),
        "hyperliquid:xyz"
    );
    assert!(venue_names_equal(" Hyperliquid:XYZ ", "hyperliquid:xyz"));
    assert!(!venue_names_equal("hyperliquid:xyz", "hyperliquid:km"));
    assert_eq!(venue_family(" Hyperliquid:XYZ "), "Hyperliquid");
    assert_eq!(
        venue_family_id(" Hyperliquid:XYZ "),
        Some(VenueId::Hyperliquid)
    );
}

#[test]
fn venue_defaults_are_non_zero() {
    for venue in [
        VenueId::Binance,
        VenueId::Okx,
        VenueId::Bybit,
        VenueId::Bitget,
        VenueId::Gate,
        VenueId::GateCrossEx,
        VenueId::Htx,
        VenueId::Kraken,
        VenueId::Kucoin,
        VenueId::Hyperliquid,
    ] {
        let defaults = venue.defaults();
        assert!(defaults.qps > 0);
        assert!(defaults.timeout_secs > 0);
        assert!(defaults.fanout_timeout_secs > 0);
    }
}

#[test]
fn hyperliquid_fanout_budget_covers_shared_builder_dex_rate_limit_waits() {
    assert_eq!(VenueId::Hyperliquid.defaults().fanout_timeout_secs, 15);
}

#[test]
fn venue_quality_envelope_counts_sampled_rows() {
    let rows = vec![
        quality("binance", VenueQualitySampleStatus::NoSample),
        quality("okx", VenueQualitySampleStatus::WarmingUp),
        quality("gate", VenueQualitySampleStatus::Ready),
    ];

    let envelope = VenueQualityEnvelope::new(rows, 10, VenueQualitySource::RuntimeSamples);

    assert_eq!(envelope.row_count, 3);
    assert_eq!(envelope.sampled_count, 2);
}

#[test]
fn venue_quality_envelope_defaults_new_runtime_fields_for_old_payloads() {
    let payload = r#"{
        "rows": [],
        "generatedAtMs": 10,
        "source": "neutral_no_sample",
        "rowCount": 0,
        "sampledCount": 0
    }"#;

    let envelope: VenueQualityEnvelope =
        serde_json::from_str(payload).expect("deserialize legacy venue quality envelope");

    assert_eq!(envelope.operation_count, 0);
    assert_eq!(envelope.attention_count, 0);
    assert_eq!(envelope.retry_after_ms, None);
    assert_eq!(envelope.request_id, None);
}

#[test]
fn operation_health_snapshot_counts_attention_rows() {
    let rows = vec![
        operation("binance", VenueOperationStatus::Ok),
        operation("okx", VenueOperationStatus::Unknown),
        operation("gate", VenueOperationStatus::Blocked),
    ];

    let snapshot = VenueOperationHealthSnapshot::new(rows, 10);

    assert_eq!(snapshot.row_count, 3);
    assert_eq!(snapshot.attention_count, 2);
}

#[test]
fn operation_health_snapshot_promotes_max_retry_after_ms() {
    let mut runtime_retry = operation("binance", VenueOperationStatus::Warn);
    runtime_retry.retry_after_ms = Some(1_500);
    let mut problem_retry = operation("okx", VenueOperationStatus::Blocked);
    problem_retry.problem =
        Some(ApiProblem::new("RATE_LIMITED", "rate limited").with_retry_after_ms(Some(2_500)));
    let mut both = operation("gate", VenueOperationStatus::Blocked);
    both.retry_after_ms = Some(3_000);
    both.problem =
        Some(ApiProblem::new("GATE_RATE_LIMITED", "gate limited").with_retry_after_ms(Some(2_000)));

    let snapshot = VenueOperationHealthSnapshot::new(vec![runtime_retry, problem_retry, both], 10);
    let text = serde_json::to_string(&snapshot).expect("serialize operation health snapshot");

    assert_eq!(snapshot.retry_after_ms, Some(3_000));
    assert!(text.contains("\"retryAfterMs\":3000"));
}

#[test]
fn operation_health_snapshot_defaults_missing_retry_after_ms() {
    let payload = r#"{"rows":[],"generatedAtMs":10,"rowCount":0,"attentionCount":0}"#;

    let snapshot: VenueOperationHealthSnapshot =
        serde_json::from_str(payload).expect("deserialize legacy operation health snapshot");

    assert_eq!(snapshot.retry_after_ms, None);
}

#[test]
fn operation_health_serializes_structured_latency() {
    let mut row = operation("binance", VenueOperationStatus::Ok);
    row.latency_ms = Some(8);
    row.latency_p95_ms = Some(25);

    let text = serde_json::to_string(&row).expect("serialize operation health");

    assert!(text.contains("\"latencyMs\":8"));
    assert!(text.contains("\"latencyP95Ms\":25"));
}
#[test]
fn secret_storage_status_defaults_to_runtime_only_for_old_payloads() {
    let status = SecretStorageStatus::default();

    assert_eq!(status.mode, SecretStorageMode::RuntimeOnly);
    assert!(!status.persistent);
    assert!(!status.encrypted);
}

#[test]
fn env_file_secret_storage_does_not_claim_encryption() {
    let status = SecretStorageStatus::env_file_atomic(Some("/tmp/.env".to_owned()));

    assert_eq!(status.mode, SecretStorageMode::EnvFileAtomic);
    assert!(status.persistent);
    assert!(status.atomic_write);
    assert!(!status.encrypted);
    assert_eq!(status.path.as_deref(), Some("/tmp/.env"));
    assert!(status.warning.is_some());
}

#[test]
fn keychain_secret_storage_claims_encrypted_persistence() {
    let status = SecretStorageStatus::keychain("com.crossline.test");

    assert_eq!(status.mode, SecretStorageMode::Keychain);
    assert!(status.persistent);
    assert!(status.encrypted);
    assert!(status.atomic_write);
    assert_eq!(status.path.as_deref(), Some("service:com.crossline.test"));
    assert!(status.warning.is_none());
}

fn quality(venue: &str, sample_status: VenueQualitySampleStatus) -> VenueQuality {
    VenueQuality {
        venue: venue.to_owned(),
        source: "test".to_owned(),
        sample_status,
        avg_rest_latency_ms: 0,
        rest_latency_samples: 0,
        ws_jitter_p99_ms: 0,
        ws_jitter_samples: 0,
        fill_rate_pct: 0.0,
        fill_window_samples: 0,
        avg_slippage_bps: 0.0,
        slippage_samples: 0,
        uptime_window_pct: 0.0,
        uptime_window_samples: 0,
        sample_window: VenueQualitySampleWindow::default(),
        operation_health: Vec::new(),
        retry_after_ms: None,
        last_problem: None,
    }
}

fn operation(venue: &str, status: VenueOperationStatus) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: "rest_orderbooks".to_owned(),
        status,
        source: "test".to_owned(),
        message: "test".to_owned(),
        supported: None,
        configured: None,
        requested: None,
        rows: None,
        freshness_ms: None,
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 10,
    }
}
