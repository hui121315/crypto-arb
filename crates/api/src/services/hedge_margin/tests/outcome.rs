use super::*;

#[test]
fn required_margin_venues_dedupes_live_non_reduce_only() {
    let live = order_intent("binance", ExecutionMode::Live, false);
    let reduce_only = order_intent("okx", ExecutionMode::Live, true);
    let paper = order_intent("gate", ExecutionMode::DryRun, false);
    let duplicate = order_intent("Binance", ExecutionMode::Live, false);

    let venues = required_margin_venues(&[&live, &reduce_only, &paper, &duplicate]);

    assert_eq!(venues, vec!["binance"]);
}

#[test]
fn missing_required_margin_balance_returns_typed_problem() {
    let intent = order_intent("binance", ExecutionMode::Live, false);
    let result = ensure_required_venue_balances(&[], &[&intent]);

    assert!(matches!(
        result,
        Err(AppError::Domain { code, .. }) if code == codes::MARGIN_BALANCE_MISSING
    ));
}

#[test]
fn hyperliquid_spot_balance_can_cover_builder_margin_check() {
    let intent = order_intent("hyperliquid:xyz", ExecutionMode::Live, false);
    let balances = vec![VenueBalanceInfo {
        venue: "hyperliquid:spot".into(),
        currency: "USDC".into(),
        total: 10.0,
        available: 10.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }];

    assert!(ensure_required_venue_balances(&balances, &[&intent]).is_ok());
}

#[test]
fn hyperliquid_sibling_dex_balance_cannot_cover_builder_margin_check() {
    let intent = order_intent("hyperliquid:xyz", ExecutionMode::Live, false);
    let balances = vec![VenueBalanceInfo {
        venue: "hyperliquid:abc".into(),
        currency: "USDC".into(),
        total: 10.0,
        available: 10.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }];

    let result = ensure_required_venue_balances(&balances, &[&intent]);

    assert!(matches!(
        result,
        Err(AppError::Domain { code, .. }) if code == codes::MARGIN_BALANCE_MISSING
    ));
}

#[test]
fn margin_outcome_records_scope_and_observed_venues() {
    let binance = order_intent("binance", ExecutionMode::Live, false);
    let paper = order_intent("okx", ExecutionMode::DryRun, false);
    let balances = vec![VenueBalanceInfo {
        venue: "binance".into(),
        currency: "USDT".into(),
        total: 100.0,
        available: 100.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }];

    let outcome = margin_outcome(
        HedgePreflightStatus::Passed,
        &[&binance, &paper],
        &balances,
        MarginBalanceEvidence {
            operation_health: vec![current_balance_operation_row("binance")],
            ..MarginBalanceEvidence::default()
        },
        None,
    );

    assert_eq!(outcome.status, HedgePreflightStatus::Passed);
    assert_margin_scope(&outcome);
    assert_eq!(outcome.observed_venues, vec!["binance"]);
    assert_eq!(
        outcome.source.as_deref(),
        Some("account_state.margin_facts")
    );
    assert!(outcome.problems.is_empty());
    assert_margin_balance_row(&outcome);
    assert_margin_field_quality(&outcome);
    assert_margin_row_health(&outcome);
}

#[test]
fn margin_outcome_carries_scoped_balance_evidence() {
    let intent = order_intent("binance", ExecutionMode::Live, false);
    let evidence = MarginBalanceEvidence {
        freshness_ms: Some(250),
        retry_after_ms: None,
        request_id: Some("margin-rid-1".to_owned()),
        operation_health: Vec::new(),
        ..MarginBalanceEvidence::default()
    };

    let outcome = margin_outcome(
        HedgePreflightStatus::Passed,
        &[&intent],
        &[],
        evidence,
        None,
    );

    assert_eq!(outcome.freshness_ms, Some(250));
    assert_eq!(outcome.retry_after_ms, None);
    assert_eq!(outcome.request_id.as_deref(), Some("margin-rid-1"));
}

#[test]
fn margin_outcome_carries_row_health_from_operation_health() {
    let intent = order_intent("binance", ExecutionMode::Live, false);
    let balances = vec![VenueBalanceInfo {
        venue: "binance".into(),
        currency: "USDT".into(),
        total: 100.0,
        available: 95.0,
        frozen: 5.0,
        unrealized_pnl: 0.0,
    }];
    let evidence = MarginBalanceEvidence {
        freshness_ms: Some(250),
        retry_after_ms: Some(2_000),
        request_id: Some("margin-rid-1".to_owned()),
        operation_health: vec![operation_row(
            "binance",
            "balance",
            Some(250),
            Some(2_000),
            Some("req-balance-health"),
        )],
        ..MarginBalanceEvidence::default()
    };

    let outcome = margin_outcome(
        HedgePreflightStatus::Passed,
        &[&intent],
        &balances,
        evidence,
        None,
    );

    assert_eq!(outcome.row_health.len(), 1);
    assert_eq!(
        outcome.row_health[0].subject,
        AccountFieldSubject::balance("binance", "USDT")
    );
    assert_eq!(outcome.row_health[0].source, "test");
    assert_eq!(outcome.row_health[0].freshness_ms, Some(250));
    assert_eq!(outcome.row_health[0].retry_after_ms, Some(2_000));
    assert_eq!(
        outcome.row_health[0].request_id.as_deref(),
        Some("req-balance-health")
    );
    assert_eq!(
        outcome.row_health[0]
            .last_error
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::UPSTREAM_HTTP)
    );
}

#[test]
fn margin_outcome_marks_missing_available_balance_field() {
    let intent = order_intent("binance", ExecutionMode::Live, false);

    let outcome = margin_outcome(
        HedgePreflightStatus::Blocked,
        &[&intent],
        &[],
        MarginBalanceEvidence::default(),
        Some("missing balance".to_owned()),
    );

    assert_eq!(outcome.problems.len(), 1);
    assert_eq!(outcome.problems[0].code, codes::MARGIN_BALANCE_MISSING);
    assert!(outcome.field_quality.iter().any(|row| {
        row.field == "available" && row.status == AccountFieldQualityStatus::Missing
    }));
}

#[test]
fn margin_outcome_failed_prefers_read_problem_over_missing_problem() {
    let intent = order_intent("binance", ExecutionMode::Live, false);

    let outcome = margin_outcome(
        HedgePreflightStatus::Failed,
        &[&intent],
        &[],
        MarginBalanceEvidence::default(),
        Some("read failed".to_owned()),
    );

    assert_eq!(outcome.problems.len(), 1);
    assert_eq!(outcome.problems[0].code, codes::MARGIN_BALANCE_READ_FAILED);
    assert!(outcome.field_quality.iter().any(|row| {
        row.field == "available"
            && row.status == AccountFieldQualityStatus::Missing
            && row
                .problem
                .as_ref()
                .is_some_and(|problem| problem.code == codes::MARGIN_BALANCE_READ_FAILED)
    }));
}

#[test]
fn margin_outcome_blocks_stale_scoped_balance_evidence() {
    let intent = order_intent("binance", ExecutionMode::Live, false);
    let balances = vec![VenueBalanceInfo {
        venue: "binance".into(),
        currency: "USDT".into(),
        total: 100.0,
        available: 100.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }];
    let mut stale = current_balance_operation_row("binance");
    stale.status = VenueOperationStatus::Warn;
    stale.message = "账户缓存样本可用但已变旧".into();

    let outcome = margin_outcome(
        HedgePreflightStatus::Blocked,
        &[&intent],
        &balances,
        MarginBalanceEvidence {
            operation_health: vec![stale],
            ..MarginBalanceEvidence::default()
        },
        Some("保证金余额证据已降级: binance".into()),
    );

    assert_eq!(outcome.problems.len(), 1);
    assert_eq!(outcome.problems[0].code, codes::BALANCE_READ_DEGRADED);
    assert_eq!(outcome.status, HedgePreflightStatus::Blocked);
}

#[test]
fn margin_evidence_rejects_sibling_hyperliquid_dex_health() -> Result<(), &'static str> {
    let intent = order_intent("hyperliquid:xyz", ExecutionMode::Live, false);
    let balances = vec![VenueBalanceInfo {
        venue: "hyperliquid:spot".into(),
        currency: "USDC".into(),
        total: 100.0,
        available: 100.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }];
    let evidence = MarginBalanceEvidence {
        operation_health: vec![current_balance_operation_row("hyperliquid:abc")],
        ..MarginBalanceEvidence::default()
    };

    let problem = margin_balance_evidence_problem(&[&intent], &evidence, 1)
        .ok_or("sibling DEX health must not satisfy the requested scope")?;

    assert_eq!(problem.code, codes::BALANCE_EVIDENCE_MISSING);
    assert!(has_margin_balance_for_intent(&balances, &intent));
    Ok(())
}
