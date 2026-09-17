use super::*;

#[test]
fn margin_evidence_merges_scoped_operation_health_request_and_retry() {
    let mut evidence = MarginBalanceEvidence::default();
    let venues = vec!["hyperliquid:xyz".to_owned()];
    let rows = vec![
        operation_row(
            "hyperliquid:spot",
            "balance",
            Some(400),
            Some(2_000),
            Some("req-hype-balance"),
        ),
        operation_row(
            "okx",
            "balance",
            Some(900),
            Some(9_000),
            Some("req-okx-ignored"),
        ),
    ];

    merge_operation_health_evidence(&mut evidence, &venues, &rows);

    assert_eq!(evidence.freshness_ms, Some(400));
    assert_eq!(evidence.retry_after_ms, Some(2_000));
    assert_eq!(evidence.request_id.as_deref(), Some("req-hype-balance"));
}

#[test]
fn margin_evidence_ignores_non_balance_operation_health() {
    let mut evidence = MarginBalanceEvidence {
        freshness_ms: Some(100),
        retry_after_ms: None,
        request_id: None,
        operation_health: Vec::new(),
        ..MarginBalanceEvidence::default()
    };
    let venues = vec!["binance".to_owned()];
    let rows = vec![operation_row(
        "binance",
        "order_write",
        Some(800),
        Some(2_000),
        Some("req-order-write"),
    )];

    merge_operation_health_evidence(&mut evidence, &venues, &rows);

    assert_eq!(evidence.freshness_ms, Some(100));
    assert_eq!(evidence.retry_after_ms, None);
    assert_eq!(evidence.request_id, None);
}

#[test]
fn final_margin_error_embeds_preflight_outcome_details() {
    let intent = order_intent("binance", ExecutionMode::Live, false);
    let evidence = margin_evidence(
        HedgePreflightStatus::Blocked,
        &[&intent],
        &[],
        MarginBalanceEvidence::default(),
        Some("missing balance".to_owned()),
    );
    let error = with_margin_evidence(missing_balance_error(&["binance".to_owned()]), &evidence);

    assert!(
        matches!(
            error,
            AppError::Domain {
                details: Some(_),
                ..
            }
        ),
        "expected domain details"
    );
    let AppError::Domain {
        details: Some(details),
        ..
    } = error
    else {
        return;
    };

    assert_eq!(details["venues"][0], "binance");
    assert_eq!(details["preflightOutcome"]["status"], "blocked");
    assert_eq!(
        details["preflightOutcome"]["problems"][0]["code"],
        codes::MARGIN_BALANCE_MISSING
    );
    assert!(details["preflightOutcome"]["fieldQuality"]
        .as_array()
        .is_some_and(|rows| rows.iter().any(|row| row["field"] == "available")));
    assert_eq!(
        details["preflightOutcome"]["rowHealth"][0]["subject"]["venue"],
        "binance"
    );
    assert_eq!(
        details["preflightOutcome"]["rowHealth"][0]["subject"]["kind"],
        "account"
    );
    assert_eq!(
        details["preflightOutcome"]["rowHealth"][0]["lastError"]["code"],
        codes::MARGIN_BALANCE_MISSING
    );
    assert_eq!(details["accountState"]["status"], "degraded");
    assert_eq!(
        details["accountState"]["balances"]["problems"][0]["code"],
        codes::MARGIN_BALANCE_MISSING
    );
    assert_eq!(
        details["accountState"]["balances"]["rowHealth"][0]["lastError"]["code"],
        codes::MARGIN_BALANCE_MISSING
    );
    assert!(details["accountState"]["fieldQuality"]
        .as_array()
        .is_some_and(|rows| rows.iter().any(|row| row["field"] == "available")));
}
