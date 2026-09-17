use super::*;

#[test]
fn nav_breakdown_keeps_wallet_position_cash_and_unrealized_quality() {
    let rows = rows_from_positions(vec![position("long")], &[], 0, 15.0, 8.0);
    let summary = summary_from_rows_with_nav(
        &rows,
        &fresh_position_account_state(),
        7,
        AccountNav {
            value: 125.0,
            evidence: PortfolioNavEvidence {
                status: AccountFieldQualityStatus::Actual,
                source: "account_state.account_summaries.total_equity_usd".to_owned(),
                ..PortfolioNavEvidence::default()
            },
        },
        Some(125.0),
        PortfolioPnlToday::default(),
    );
    let breakdown = &summary.nav_evidence.breakdown;

    assert_eq!(breakdown.wallet_equity.value_usd, Some(125.0));
    assert_eq!(
        breakdown.wallet_equity.status,
        AccountFieldQualityStatus::Actual
    );
    assert_eq!(breakdown.position_equity.value_usd, Some(51.0));
    assert_eq!(
        breakdown.position_equity.status,
        AccountFieldQualityStatus::Actual
    );
    assert_eq!(breakdown.cash.value_usd, Some(74.0));
    assert_eq!(breakdown.cash.status, AccountFieldQualityStatus::Estimated);
    assert_eq!(breakdown.unrealized_pnl.value_usd, Some(1.0));
}

#[test]
fn nav_breakdown_uses_only_valuation_quality_for_position_equity() {
    let rows = rows_from_positions(vec![position("long")], &[], 0, 15.0, 8.0);
    let mut account_state = fresh_position_account_state();
    account_state.field_quality = vec![AccountFieldQuality::new(
        AccountFieldSubject::position("OKX", "BTC", "long"),
        "liquidationDistancePct",
        AccountFieldQualityStatus::Missing,
        "account_position_runtime",
        Some(7),
    )];
    let actual = summary_from_rows_with_nav(
        &rows,
        &account_state,
        7,
        AccountNav {
            value: 125.0,
            evidence: PortfolioNavEvidence {
                status: AccountFieldQualityStatus::Actual,
                ..PortfolioNavEvidence::default()
            },
        },
        Some(125.0),
        PortfolioPnlToday::default(),
    );
    assert_eq!(
        actual.nav_evidence.breakdown.position_equity.status,
        AccountFieldQualityStatus::Actual
    );

    account_state.field_quality.push(AccountFieldQuality::new(
        AccountFieldSubject::position("OKX", "BTC", "long"),
        "margin",
        AccountFieldQualityStatus::Estimated,
        "position_notional_over_leverage_estimate",
        Some(7),
    ));
    let summary = summary_from_rows_with_nav(
        &rows,
        &account_state,
        7,
        AccountNav {
            value: 125.0,
            evidence: PortfolioNavEvidence {
                status: AccountFieldQualityStatus::Actual,
                ..PortfolioNavEvidence::default()
            },
        },
        Some(125.0),
        PortfolioPnlToday::default(),
    );

    assert_eq!(
        summary.nav_evidence.breakdown.position_equity.status,
        AccountFieldQualityStatus::Estimated
    );
    assert_eq!(
        summary.nav_evidence.breakdown.position_equity.value_usd,
        Some(51.0)
    );
}

#[test]
fn nav_history_samples_every_five_minutes() {
    let rows = vec![(1_000, 100.0)];

    assert!(!should_sample_nav(&rows, 1_000 + NAV_SAMPLE_MS - 1, 101.0));
    assert!(should_sample_nav(&rows, 1_000 + NAV_SAMPLE_MS, 101.0));
    assert!(!should_sample_nav(&rows, 1_000 + NAV_SAMPLE_MS, f64::NAN));
}

#[test]
fn recovered_account_equity_persists_once_before_normal_interval() {
    let rows = vec![(1_000, 100.0)];
    let plan = NavSamplePlan {
        should_persist: true,
        skipped_source: nav_persist::NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY,
        skipped_problem: "",
    };

    assert!(nav_sample_due(&rows, 2_000, 101.0, plan, true));
    assert!(!nav_sample_due(&rows, 2_000, 101.0, plan, false));
    assert!(!nav_sample_due(&rows, 2_000, f64::NAN, plan, true));
}

#[test]
fn nav_sample_plan_skips_missing_account_equity() {
    let nav = AccountNav {
        value: 0.0,
        evidence: PortfolioNavEvidence::default(),
    };
    let plan = nav_sample_plan(&nav);

    assert!(!plan.should_persist);
    assert_eq!(
        plan.skipped_source,
        nav_persist::NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY_MISSING
    );
}

#[test]
fn nav_sample_plan_persists_actual_account_equity() {
    let nav = AccountNav {
        value: 100.0,
        evidence: PortfolioNavEvidence {
            status: AccountFieldQualityStatus::Actual,
            ..PortfolioNavEvidence::default()
        },
    };
    let plan = nav_sample_plan(&nav);

    assert!(plan.should_persist);
    assert_eq!(
        plan.skipped_source,
        nav_persist::NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY
    );
}

#[test]
fn account_nav_uses_complete_account_summary_equity() {
    let mut account_state = AccountStateSnapshot::default();
    account_state.balances.account_summaries = vec![shared_types::VenueAccountSummary {
        venue: "bybit".to_owned(),
        account_type: "UNIFIED".to_owned(),
        equity_scope: AccountEquityScope::Unified,
        total_equity_usd: 125.0,
        total_available_balance_usd: 100.0,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: 20.0,
        total_maintenance_margin_usd: 5.0,
        account_im_rate: 0.16,
        account_mm_rate: 0.04,
        source: "bybit wallet".to_owned(),
        observed_at_ms: 7,
        freshness_ms: Some(0),
        problem: None,
    }];
    let nav = account_nav(&account_state, 8);

    assert_eq!(nav.value, 125.0);
    assert_eq!(nav.evidence.status, AccountFieldQualityStatus::Actual);
    assert_eq!(nav.evidence.covered_venues, vec!["bybit"]);
    assert!(nav.evidence.problem.is_none());
}

#[test]
fn account_nav_fails_closed_when_spot_equity_is_uncovered() {
    let account_state = AccountStateSnapshot {
        field_quality: vec![AccountFieldQuality::new(
            AccountFieldSubject::account("hyperliquid"),
            "equity",
            AccountFieldQualityStatus::Unknown,
            "account_state_runtime",
            Some(7),
        )],
        balances: shared_types::VenueBalanceEnvelope::new(
            Vec::new(),
            ListStatus::Fresh,
            "account_state_runtime",
            7,
            Vec::new(),
            Vec::new(),
        )
        .with_account_summaries(vec![shared_types::VenueAccountSummary {
            venue: "hyperliquid".to_owned(),
            account_type: "perpetuals".to_owned(),
            equity_scope: AccountEquityScope::Perpetuals,
            total_equity_usd: 125.0,
            total_available_balance_usd: 100.0,
            withdrawable_balance_usd: Some(100.0),
            total_initial_margin_usd: 20.0,
            total_maintenance_margin_usd: 5.0,
            account_im_rate: 0.16,
            account_mm_rate: 0.04,
            source: "hyperliquid clearinghouseState".to_owned(),
            observed_at_ms: 7,
            freshness_ms: Some(0),
            problem: None,
        }]),
        account_bindings: vec![hyperliquid_unified_binding()],
        ..Default::default()
    };
    let nav = account_nav(&account_state, 8);

    assert_eq!(nav.value, 0.0);
    assert_eq!(nav.evidence.status, AccountFieldQualityStatus::Missing);
    assert_eq!(nav.evidence.missing_venues, vec!["hyperliquid"]);
    assert!(nav.evidence.problem.is_some());
}

#[test]
fn account_nav_uses_spot_equity_once_for_unified_hyperliquid() {
    let mut account_state = AccountStateSnapshot {
        account_bindings: vec![hyperliquid_unified_binding()],
        ..Default::default()
    };
    account_state.balances.account_summaries = vec![
        shared_types::VenueAccountSummary {
            venue: "hyperliquid".to_owned(),
            account_type: "perpetuals".to_owned(),
            equity_scope: AccountEquityScope::Perpetuals,
            total_equity_usd: 125.0,
            total_available_balance_usd: 100.0,
            withdrawable_balance_usd: Some(100.0),
            total_initial_margin_usd: 20.0,
            total_maintenance_margin_usd: 5.0,
            account_im_rate: 0.16,
            account_mm_rate: 0.04,
            source: "hyperliquid clearinghouseState".to_owned(),
            observed_at_ms: 7,
            freshness_ms: Some(0),
            problem: None,
        },
        shared_types::VenueAccountSummary {
            venue: "hyperliquid:spot".to_owned(),
            account_type: "spot".to_owned(),
            equity_scope: AccountEquityScope::Spot,
            total_equity_usd: 25.0,
            total_available_balance_usd: 20.0,
            withdrawable_balance_usd: None,
            total_initial_margin_usd: 0.0,
            total_maintenance_margin_usd: 0.0,
            account_im_rate: 0.0,
            account_mm_rate: 0.0,
            source: "hyperliquid spot mark valuation".to_owned(),
            observed_at_ms: 7,
            freshness_ms: Some(0),
            problem: None,
        },
    ];

    let nav = account_nav(&account_state, 8);

    assert_eq!(nav.value, 25.0);
    assert_eq!(nav.evidence.status, AccountFieldQualityStatus::Actual);
    assert_eq!(nav.evidence.covered_venues, vec!["hyperliquid"]);
}

#[test]
fn nav_history_uses_latest_sample_before_lookback() {
    let now = NAV_LOOKBACK_MS + 10_000;
    let rows = vec![(1_000, 100.0), (9_000, 110.0), (11_000, 120.0)];

    assert_eq!(nav_at_lookback(&rows, now), Some(110.0));
}

#[test]
fn nav_history_limit_clamps_to_product_bounds() {
    assert_eq!(nav_history_limit(None), NAV_HISTORY_DEFAULT_LIMIT);
    assert_eq!(nav_history_limit(Some(0)), 1);
    assert_eq!(
        nav_history_limit(Some(NAV_HISTORY_MAX_LIMIT + 1)),
        NAV_HISTORY_MAX_LIMIT
    );
}
