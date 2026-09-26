use super::super::super::data::{
    PreviewDepth, PreviewLiquidation, PreviewProfitEvidence, PreviewRisk,
};
use super::*;
use shared_types::{
    AccountFieldSubject, HedgePreflightScope, MarginPreflightOutcome, MarketDataSourceKind,
};

mod cases;
mod position_evidence;

fn ready_preview(one_cycle_cost: Option<PreviewOneCycleCost>) -> ExecutionPreview {
    ExecutionPreview {
        opportunity_id: "opp-1".into(),
        opportunity_snapshot_id: "snapshot-1".into(),
        idempotency_key: Some("idem-1".into()),
        ticket_id: Some("ticket-1".into()),
        expires_at_ms: Some(i64::MAX),
        clock: None,
        readiness: PreviewReadiness::Ready,
        source: "后端交易检查",
        estimated_funding_usd: 1.0,
        open_cost_usd: 0.0,
        close_cost_usd: 0.0,
        slippage_cost_usd: 0.0,
        one_cycle_cost,
        max_loss_usd: 1.0,
        used_capital_usd: 100.0,
        liquidation: PreviewLiquidation {
            current_account_pct: None,
            after_hedge_pct: None,
            positions_evidence: None,
        },
        execution_mode_label: "模拟",
        long_allowed: true,
        short_allowed: true,
        long_notional_usd: 100.0,
        short_notional_usd: 100.0,
        long_reference_price: Some(100.0),
        short_reference_price: Some(101.0),
        long_market_evidence: None,
        short_market_evidence: None,
        depth: PreviewDepth {
            long_5bps: None,
            long_10bps: None,
            long_20bps: None,
            short_5bps: None,
            short_10bps: None,
            short_20bps: None,
            executable_status: HedgeDepthStatus::Available,
            executable_amount_usd: Some(100.0),
            executable_reason: None,
            long_reason: None,
            short_reason: None,
            long_depth_health: None,
            short_depth_health: None,
        },
        fee_evidence: Vec::new(),
        profit_evidence: PreviewProfitEvidence {
            one_cycle_net_bps: 0.0,
            fee_evidence_ids: Vec::new(),
            fee_evidence_complete: false,
        },
        order_plans: Vec::new(),
        identity_evidence_required: false,
        risk: PreviewRisk {
            note: "通过".into(),
            guards: Vec::new(),
            blockers: Vec::new(),
        },
    }
}

fn market_health(quality: MarketDataQuality) -> MarketDataHealth {
    MarketDataHealth {
        quality,
        source: MarketDataSourceKind::WsPush,
        freshness_ms: Some(42),
        retry_after_ms: None,
        last_error: None,
        observed_at_ms: 1,
        coverage: None,
        problem: None,
    }
}

fn guard(label: &str, status: HedgePreflightStatus) -> ExecutionGuard {
    ExecutionGuard {
        key: label.into(),
        label: label.into(),
        passed: status == HedgePreflightStatus::Passed,
        detail: format!("{label}检查"),
        preflight_outcome: Some(MarginPreflightOutcome {
            status,
            checked_at_ms: 1,
            scope: HedgePreflightScope {
                venues: vec!["binance".into()],
                symbols: vec!["MUUSDT".into()],
                account_modes: vec!["unified".into()],
                operations: vec![
                    HedgePreflightOperation::MarginBalance,
                    HedgePreflightOperation::Positions,
                    HedgePreflightOperation::OpenOrders,
                    HedgePreflightOperation::PrivateWs,
                ],
            },
            observed_venues: vec!["binance".into()],
            balance_rows: vec![VenueBalanceInfo {
                venue: "binance".into(),
                currency: "USDT".into(),
                total: 120.0,
                available: 100.0,
                frozen: 20.0,
                unrealized_pnl: 0.0,
            }],
            source: Some("balance_cache".into()),
            freshness_ms: Some(42),
            retry_after_ms: None,
            request_id: Some("request-abcdef".into()),
            problems: vec![
                ApiProblem::new("MARGIN_BALANCE_MISSING", "missing margin").with_status(200)
            ],
            field_quality: vec![AccountFieldQuality::new(
                AccountFieldSubject::balance("binance", "USDT"),
                "available",
                AccountFieldQualityStatus::Missing,
                "account_balance_runtime",
                Some(1),
            )],
            row_health: vec![AccountDataHealth {
                subject: AccountFieldSubject::balance("binance", "USDT"),
                source: "account_cache".into(),
                observed_at_ms: 2,
                freshness_ms: Some(500),
                last_success_ms: Some(1),
                last_error: Some(ApiProblem::new("BALANCE_READ_DEGRADED", "rate limited")),
                retry_after_ms: Some(2_000),
                request_id: Some("request-balance-row".into()),
            }],
            error: (status == HedgePreflightStatus::Blocked).then(|| "insufficient".into()),
        }),
    }
}

#[test]
fn ticket_venue_availability_is_scoped_to_live_ticket_preflight() {
    let mut preview = ready_preview(None);
    preview.execution_mode_label = "实盘";
    let mut ticket_guard = guard("live_operation_health", HedgePreflightStatus::Passed);
    assert!(
        ticket_guard.preflight_outcome.is_some(),
        "ticket guard has outcome"
    );
    let Some(outcome) = ticket_guard.preflight_outcome.as_mut() else {
        return;
    };
    outcome.scope.venues = vec!["binance".into(), "okx".into()];
    outcome.observed_venues = vec!["binance".into(), "okx".into()];
    outcome.source = Some("ticket_scoped_runtime".into());
    let mut unrelated = guard("other_venue_guard", HedgePreflightStatus::Blocked);
    assert!(
        unrelated.preflight_outcome.is_some(),
        "unrelated guard has outcome"
    );
    let Some(unrelated_outcome) = unrelated.preflight_outcome.as_mut() else {
        return;
    };
    unrelated_outcome.scope.venues = vec!["bybit".into()];
    preview.risk.guards = vec![unrelated, ticket_guard];

    let summary = ticket_venue_availability_summary(&preview);
    let detail = ticket_venue_availability_detail(&preview);

    assert_eq!(summary, "双腿 binance / okx · 可用 2/2");
    assert!(detail.contains("双腿范围 binance / okx"));
    assert!(detail.contains("checked 1"));
    assert!(detail.contains("source ticket_scoped_runtime"));
    assert!(detail.contains("freshness 42ms"));
    assert!(detail.contains("request request-abcdef"));
    assert!(detail.contains("health binance USDT account_cache"));
    assert!(!detail.contains("bybit"));
}

#[test]
fn keyed_guard_display_reads_the_latest_preview_value() {
    let mut preview = ready_preview(None);
    preview.risk.guards = vec![ExecutionGuard {
        key: "no_blockers".into(),
        label: "无硬阻断".into(),
        passed: false,
        detail: "票据存在硬阻断".into(),
        preflight_outcome: None,
    }];

    assert_eq!(
        current_guard_detail(&preview, "no_blockers"),
        "票据存在硬阻断"
    );
    assert_eq!(
        current_guard_state(&preview, "no_blockers"),
        CheckItemState::Block
    );

    preview.risk.guards[0].passed = true;
    preview.risk.guards[0].detail = "通过".into();

    assert_eq!(current_guard_detail(&preview, "no_blockers"), "通过");
    assert_eq!(
        current_guard_state(&preview, "no_blockers"),
        CheckItemState::Ok
    );
}
