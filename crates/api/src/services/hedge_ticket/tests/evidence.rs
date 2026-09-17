use super::*;
use shared_types::{MarketDataQuality, MarketDataSourceKind, SpotLegMode, StrategyKind};

mod market;

#[tokio::test]
async fn transfer_guard_is_explicit_for_spot_strategies() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let mut same_venue = opportunity_with_prices(Some(100.0), Some(101.0));
    same_venue.strategy_kind = Some(StrategyKind::SpotPerp);
    same_venue.spot_leg_mode = Some(SpotLegMode::BuySpot);
    same_venue.short_exchange = same_venue.long_exchange.clone();

    let mobility_pending = transfer_route_guard(&state, &same_venue, 1);

    assert!(!mobility_pending.passed);
    assert_eq!(mobility_pending.key, TRANSFER_ROUTE_EVIDENCE_KEY);
    assert!(mobility_pending.detail.contains("充提状态"));

    let mut cross_venue = same_venue;
    cross_venue.strategy_kind = Some(StrategyKind::SpotCross);
    let unavailable = transfer_route_guard(&state, &cross_venue, 1);

    assert!(!unavailable.passed);
    assert!(!unavailable.detail.trim().is_empty());
    Ok(())
}

#[test]
fn sizing_keeps_unknown_depth_reason_structured() {
    let params = HedgeExecutionParams::default();
    let mut long = empty_leg();
    long.blockers = vec!["hyperliquid:xyz SNDK orderbook 触发限频退避，2000ms 后重试".into()];
    let mut short = empty_leg();
    short.exchange = "gate".into();
    short.depth_usd_5bps = Some(1_000.0);
    short.max_notional_usd = Some(1_000.0);
    short.depth_usd_20bps = Some(1_000.0);

    let sizing = sizing(&params, 375.0, [375.0, 375.0], None, &long, &short);

    assert_eq!(
        sizing.max_executable_notional.status,
        HedgeDepthStatus::Unknown
    );
    assert_eq!(sizing.max_executable_notional.amount_usd, None);
    assert_eq!(
        sizing.max_executable_notional.short_leg_depth_usd,
        Some(1_000.0)
    );
    assert!(sizing
        .max_executable_notional
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("限频退避")));
}

#[test]
fn sizing_marks_depth_insufficient_without_erasing_amount() {
    let params = HedgeExecutionParams::default();
    let mut long = empty_leg();
    long.depth_usd_5bps = Some(250.0);
    long.max_notional_usd = Some(250.0);
    long.depth_usd_20bps = Some(250.0);
    let mut short = empty_leg();
    short.exchange = "gate".into();
    short.depth_usd_5bps = Some(500.0);
    short.max_notional_usd = Some(500.0);
    short.depth_usd_20bps = Some(500.0);

    let sizing = sizing(&params, 375.0, [375.0, 375.0], None, &long, &short);

    assert_eq!(
        sizing.max_executable_notional.status,
        HedgeDepthStatus::Insufficient
    );
    assert_eq!(sizing.max_executable_notional.amount_usd, Some(250.0));
    assert!(sizing
        .max_executable_notional
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("低于目标")));
}

#[test]
fn failed_guard_blockers_skip_meta_no_blockers_guard() {
    let mut blockers = vec!["真实根因".to_owned()];
    let guards = vec![
        guard("no_blockers", "无硬阻断", false, "票据存在硬阻断"),
        guard("depth", "深度", false, "深度不足"),
    ];

    append_failed_guard_blockers(&mut blockers, &guards);

    assert_eq!(blockers, vec!["真实根因", "深度不足"]);
}

#[test]
fn missing_cost_evidence_cannot_pass_ticket_guards() {
    let opportunity = opportunity_with_prices(Some(100.0), Some(101.0));
    let mut long = empty_leg();
    long.max_notional_usd = Some(1_000.0);
    let mut short = empty_leg();
    short.max_notional_usd = Some(1_000.0);
    let proof = arbitrage::profit_proof::evaluate_strategy_profit(
        arbitrage::profit_proof::StrategyProfitProofInput {
            strategy: opportunity.strategy_kind,
            spot_leg_mode: opportunity.spot_leg_mode,
            funding: arbitrage::profit_proof::FundingWindowInput {
                long_funding_bps: long.funding_bps,
                short_funding_bps: short.funding_bps,
                long_next_settlement_ms: long.next_funding_time,
                short_next_settlement_ms: short.next_funding_time,
                long_interval_hours: long.funding_interval_hours,
                short_interval_hours: short.funding_interval_hours,
            },
            executable_price: arbitrage::profit_proof::ExecutablePriceInput::default(),
            gross_edge_bps: None,
            total_cost_bps: None,
            mismatch_buffer_bps: None,
            target_buffer_bps: None,
            observed_at_ms: 1,
        },
    );
    let guards = guards(&GuardInputs {
        opp: &opportunity,
        long: &long,
        short: &short,
        long_target_notional: 100.0,
        short_target_notional: 100.0,
        blockers: &[],
        fees_required: false,
        fee_snapshot_count: 0,
        cost: None,
        profit_proof: &proof,
    });

    assert!(guards
        .iter()
        .any(|guard| guard.key == "complete_cost" && !guard.passed));
    assert!(guards
        .iter()
        .any(|guard| guard.key == "positive_edge" && !guard.passed));
    assert!(guards
        .iter()
        .any(|guard| guard.key == "profit_lock" && !guard.passed));
}

#[test]
fn live_ticket_blocks_missing_leg_route() {
    let risk = trading::RiskConfig {
        live_trading_enabled: true,
        allowed_exchanges: BTreeSet::from(["binance".to_owned()]),
        ..trading::RiskConfig::default()
    };
    let spec = leg_spec("kucoin", "BTC");

    let blockers = live_route_blockers_for(&risk, &spec);

    assert_eq!(blockers.len(), 1);
    assert!(blockers[0].contains("未通过实盘路由校验"));
}

#[test]
fn live_ticket_allows_configured_builder_route() {
    let risk = trading::RiskConfig {
        live_trading_enabled: true,
        allowed_exchanges: BTreeSet::from(["hyperliquid:xyz".to_owned()]),
        ..trading::RiskConfig::default()
    };
    let spec = leg_spec("hyperliquid:xyz", "MU");

    assert!(live_route_blockers_for(&risk, &spec).is_empty());
}

#[test]
fn live_fee_guard_blocks_unverified_fee_product() {
    let risk = trading::RiskConfig {
        live_trading_enabled: true,
        ..trading::RiskConfig::default()
    };
    let spec = leg_spec("binance", "BTC");

    let blockers = fee_blockers(&risk, &spec, "交易产品费率类型未验证");

    assert_eq!(blockers.len(), 1);
    assert!(blockers[0].contains("maker/taker/open/close fee"));
}

#[test]
fn spot_perp_fee_product_follows_typed_leg_orientation() {
    let mut opportunity = opportunity_with_prices(Some(100.0), Some(101.0));
    opportunity.strategy_kind = Some(StrategyKind::SpotPerp);
    opportunity.spot_leg_mode = Some(SpotLegMode::SellInventory);
    opportunity.long_action = "任意展示文案".into();
    opportunity.short_action = "任意展示文案".into();

    assert_eq!(
        fee_product_for(&opportunity, HedgeLegRole::Long),
        FeeProduct::Perp
    );
    assert_eq!(
        fee_product_for(&opportunity, HedgeLegRole::Short),
        FeeProduct::Spot
    );

    opportunity.spot_leg_mode = Some(SpotLegMode::BuySpot);
    assert_eq!(
        fee_product_for(&opportunity, HedgeLegRole::Long),
        FeeProduct::Spot
    );
}

#[test]
fn short_leg_spec_does_not_use_long_price_as_fallback() {
    let opp = opportunity_with_prices(Some(100.0), None);

    let spec = LegSpec::from_opp(&opp, HedgeLegRole::Short);

    assert_eq!(spec.fallback_price, None);
}

#[test]
fn resolve_fee_snapshot_rejects_unverified_fallback() {
    let now_ms = 1_000;
    let mut unverified = fee_snapshot("binance", FeeProduct::Perp, 3.0, 4.0);
    unverified.source = TradeFeeSource::OfficialSchedule;
    unverified.evidence = None;

    assert!(resolve_fee_snapshot(None, || Some(unverified.clone()), now_ms).is_none());

    let mut expired = fee_snapshot("binance", FeeProduct::Perp, 3.0, 4.0);
    expired.valid_until_ms = now_ms;
    assert!(resolve_fee_snapshot(Some(expired), || None, now_ms).is_none());

    let verified = fee_snapshot("binance", FeeProduct::Perp, 3.0, 4.0);
    assert!(resolve_fee_snapshot(Some(verified), || None, now_ms).is_some());
}
