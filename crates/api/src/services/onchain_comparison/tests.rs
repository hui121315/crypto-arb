use super::*;
use crate::services::market_data::{MarketQuality, MarketSource};
use onchain_monitor::ProviderQuote;

mod provider_contracts;

#[tokio::test]
async fn direct_symbol_change_maps_known_tokens_without_reusing_old_mints() -> anyhow::Result<()> {
    let current = OnchainComparisonConfig::default();
    let mut patch = OnchainComparisonConfigPatch {
        base_token: Some("USDC".to_owned()),
        quote_token: Some("SOL".to_owned()),
        base_mint: Some(current.base_mint.clone()),
        quote_mint: Some(current.quote_mint.clone()),
        ..OnchainComparisonConfigPatch::default()
    };

    resolve_changed_token_identity(&current, &mut patch, "USDC", "SOL").await?;

    assert_eq!(
        patch.base_mint.as_deref(),
        Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
    );
    assert_eq!(
        patch.quote_mint.as_deref(),
        Some("So11111111111111111111111111111111111111112")
    );
    assert_eq!(patch.base_decimals, Some(6));
    assert_eq!(patch.quote_decimals, Some(9));
    Ok(())
}

#[tokio::test]
async fn unknown_evm_symbol_requires_explicit_contract_identity() {
    let current = OnchainComparisonConfig::default();
    let mut patch = OnchainComparisonConfigPatch {
        chain: Some("base".to_owned()),
        base_mint: Some(current.base_mint.clone()),
        quote_mint: Some(current.quote_mint.clone()),
        ..OnchainComparisonConfigPatch::default()
    };

    let result = resolve_changed_token_identity(&current, &mut patch, "WETH", "USDC").await;

    assert!(result
        .map_err(|error| error.to_string())
        .is_err_and(|error| error.contains("缺少官方合约地址")));
}

#[test]
fn full_snapshot_is_fresh_profitable_and_strictly_read_only() {
    let snapshot = project_snapshot(
        config(),
        &quotes(),
        &market_read(Some(book(100.0, 101.0)), 10),
        1_000,
    );

    assert_eq!(snapshot.quality, OnchainComparisonQuality::Fresh);
    assert!(snapshot.read_only);
    assert_eq!(snapshot.quote_evidence.len(), 2);
    assert_eq!(snapshot.onchain_freshness_ms, Some(10));
    assert!(snapshot.comparisons.iter().all(|row| !row.executable));
    assert!(snapshot
        .quote_evidence
        .iter()
        .all(|row| !row.transaction_requested));
}

#[test]
fn batch_summary_selects_the_direction_for_the_configured_alert_mode() {
    let mut snapshot = OnchainComparisonSnapshot {
        config: config(),
        quality: OnchainComparisonQuality::Fresh,
        comparisons: vec![
            shared_types::OnchainCexComparison {
                direction: shared_types::OnchainComparisonDirection::BuyOnchainSellCex,
                onchain_price: 100.0,
                cex_price: 102.0,
                gross_spread_bps: 200.0,
                cex_fee_bps: 10.0,
                quote_conversion_fee_bps: 0.0,
                slippage_bps: 5.0,
                gas_usd: 0.0,
                gas_bps: 0.0,
                total_cost_bps: 190.0,
                net_spread_bps: 10.0,
                observable_notional_usd: 1_000.0,
                executable: false,
            },
            shared_types::OnchainCexComparison {
                direction: shared_types::OnchainComparisonDirection::BuyCexSellOnchain,
                onchain_price: 101.0,
                cex_price: 100.0,
                gross_spread_bps: 100.0,
                cex_fee_bps: 10.0,
                quote_conversion_fee_bps: 0.0,
                slippage_bps: 5.0,
                gas_usd: 0.0,
                gas_bps: 0.0,
                total_cost_bps: 20.0,
                net_spread_bps: 80.0,
                observable_notional_usd: 900.0,
                executable: false,
            },
        ],
        ..OnchainComparisonSnapshot::default()
    };

    snapshot.config.spread_alert.mode = shared_types::OnchainSpreadAlertMode::RawObservation;
    let raw = super::projection::batch_item_snapshot("raw".to_owned(), &snapshot);
    assert_eq!(
        raw.best_direction,
        Some(shared_types::OnchainComparisonDirection::BuyOnchainSellCex)
    );
    assert_eq!(raw.best_gross_spread_bps, Some(200.0));

    snapshot.config.spread_alert.mode = shared_types::OnchainSpreadAlertMode::VerifiedNet;
    let net = super::projection::batch_item_snapshot("net".to_owned(), &snapshot);
    assert_eq!(
        net.best_direction,
        Some(shared_types::OnchainComparisonDirection::BuyCexSellOnchain)
    );
    assert_eq!(net.best_net_spread_bps, Some(80.0));
}

#[test]
fn missing_cex_bbo_waits_for_the_first_ws_frame() {
    let snapshot = project_snapshot(config(), &quotes(), &market_read(None, 0), 1_000);

    assert_eq!(snapshot.quality, OnchainComparisonQuality::Pending);
    assert!(snapshot.comparisons.is_empty());
    assert!(snapshot.degradation_reasons[0].contains("首个 CEX WS 最优买卖价帧"));
}

#[test]
fn rest_startup_bbo_never_enters_onchain_profit_calculation() {
    let mut read = market_read(Some(book(100.0, 101.0)), 10);
    read.source = MarketSource::RestBaseline;

    let gated = ws_only_cex_bbo(read, &config());

    assert!(gated.value.is_none());
    assert_eq!(gated.quality, MarketQuality::Warming);
    assert_eq!(gated.source, MarketSource::LocalCache);
    assert!(gated
        .last_error
        .as_deref()
        .is_some_and(|problem| problem.contains("不能参与套利收益计算")));
}

#[test]
fn failed_cex_ws_is_an_upstream_problem_not_an_identity_problem() {
    let read = MarketRead {
        quality: MarketQuality::CircuitOpen,
        value: None,
        freshness_ms: None,
        source: MarketSource::LocalCache,
        retry_after_ms: Some(2_000),
        last_error: Some("kraken spot websocket is disconnected".to_owned()),
    };

    let snapshot = project_snapshot(config(), &quotes(), &read, 1_000);

    assert_eq!(
        snapshot.quality,
        OnchainComparisonQuality::UpstreamUnavailable
    );
    assert!(snapshot.degradation_reasons[0].contains("websocket is disconnected"));
    assert!(snapshot.degradation_reasons[0].contains("预计 2000ms 后重试"));
    assert_eq!(
        snapshot.cex_problem.as_deref(),
        Some("kraken spot websocket is disconnected")
    );
    assert_eq!(snapshot.cex_retry_after_ms, Some(2_000));
}

#[test]
fn handshake_timeout_is_not_misreported_as_normal_first_frame_warmup() {
    let problem =
        CexWsRefreshProblem::pending("websocket handshake timed out after 12s".to_owned());
    let mut read = market_read(None, 0);
    apply_cex_refresh_problem(&mut read, problem);

    let snapshot = project_snapshot(config(), &quotes(), &read, 1_000);

    assert_eq!(read.quality, MarketQuality::CircuitOpen);
    assert_eq!(
        snapshot.quality,
        OnchainComparisonQuality::UpstreamUnavailable
    );
    assert!(snapshot.degradation_reasons[0].contains("handshake timed out"));
}

#[test]
fn inbound_idle_timeout_is_not_misreported_as_normal_first_frame_warmup() {
    let problem = CexWsRefreshProblem::pending(
        "Kraken Spot WS 连接失败：inbound idle timeout: no messages for 18415ms (limit 15000ms)"
            .to_owned(),
    );
    let mut read = market_read(None, 0);
    apply_cex_refresh_problem(&mut read, problem);

    let snapshot = project_snapshot(config(), &quotes(), &read, 1_000);

    assert_eq!(read.quality, MarketQuality::CircuitOpen);
    assert_eq!(
        snapshot.quality,
        OnchainComparisonQuality::UpstreamUnavailable
    );
    assert!(snapshot.degradation_reasons[0].contains("inbound idle timeout"));
}

#[test]
fn acknowledged_subscription_wait_remains_a_normal_warmup() {
    let problem = CexWsRefreshProblem::pending("订阅已确认，正在等待首个最优买卖价变化".to_owned());
    let mut read = market_read(None, 0);
    apply_cex_refresh_problem(&mut read, problem);

    let snapshot = project_snapshot(config(), &quotes(), &read, 1_000);

    assert_eq!(read.quality, MarketQuality::Warming);
    assert_eq!(snapshot.quality, OnchainComparisonQuality::Pending);
}

#[test]
fn exchange_rate_limit_keeps_retry_evidence() {
    let error = exchange::ExchangeError::RateLimited {
        retry_after_secs: 3,
    };
    let problem = CexWsRefreshProblem::exchange(&error);
    let mut read = market_read(None, 0);
    apply_cex_refresh_problem(&mut read, problem);

    assert_eq!(read.quality, MarketQuality::RateLimited);
    assert_eq!(read.retry_after_ms, Some(3_000));
}

#[test]
fn short_cex_ws_gap_retains_last_official_prices_as_non_executable() {
    let config = config();
    let mut current = project_snapshot(
        config.clone(),
        &quotes(),
        &market_read(Some(book(100.0, 101.0)), 10),
        1_000,
    );
    current.execution_readiness.directions = vec![shared_types::OnchainDirectionReadiness {
        direction: shared_types::OnchainComparisonDirection::BuyOnchainSellCex,
        path: Default::default(),
        inventory: Vec::new(),
        cex_instrument: shared_types::OnchainCexInstrumentEvidence::default(),
        build_ready: true,
        submit_ready: true,
        blockers: Vec::new(),
    }];
    let unavailable = MarketRead {
        quality: MarketQuality::StaleAllowed,
        value: None,
        freshness_ms: None,
        source: MarketSource::LocalCache,
        retry_after_ms: Some(1_000),
        last_error: Some("binance SOL/USDC spot WS BBO expired".to_owned()),
    };

    let retained = retain_last_official_cex_projection(&current, &config, &unavailable, 7_000)
        .expect("a bounded WS gap should retain the last official projection");

    assert_eq!(retained.quality, OnchainComparisonQuality::Stale);
    assert_eq!(retained.comparisons.len(), current.comparisons.len());
    assert!(retained.comparisons.iter().all(|row| !row.executable));
    assert_eq!(retained.cex_freshness_ms, Some(6_010));
    assert!(!retained.execution_readiness.directions[0].build_ready);
    assert!(!retained.execution_readiness.directions[0].submit_ready);
    assert!(retained.degradation_reasons[0].contains("仅保留上次结果"));
}

#[test]
fn last_official_prices_are_dropped_after_the_market_cache_stale_window() {
    let config = config();
    let current = project_snapshot(
        config.clone(),
        &quotes(),
        &market_read(Some(book(100.0, 101.0)), 10),
        1_000,
    );
    let unavailable = MarketRead {
        quality: MarketQuality::Missing,
        value: None,
        freshness_ms: None,
        source: MarketSource::LocalCache,
        retry_after_ms: None,
        last_error: Some("spot WS BBO expired".to_owned()),
    };

    assert!(retain_last_official_cex_projection(&current, &config, &unavailable, 61_001).is_none());
}

#[test]
fn fresh_official_registry_rejection_is_a_mapping_problem_not_an_endless_wait() {
    let read = MarketRead {
        quality: MarketQuality::Unsupported,
        value: None,
        freshness_ms: None,
        source: MarketSource::LocalCache,
        retry_after_ms: None,
        last_error: Some(
            "binance 最新官方 Spot instrument registry 没有 PUPS/USDC 精确交易对".to_owned(),
        ),
    };

    let snapshot = project_snapshot(config(), &quotes(), &read, 1_000);

    assert_eq!(snapshot.quality, OnchainComparisonQuality::MappingInvalid);
    assert!(snapshot.degradation_reasons[0].contains("没有 PUPS/USDC"));
}

#[test]
fn mismatched_quote_and_orderbook_identities_fail_closed() {
    let bad_quotes = quote_pair(
        provider_quote("wrong-base", "quote", "1000000000", "105000000"),
        provider_quote("quote", "base", "100000000", "1000000000"),
    );
    let mismatched_quote = project_snapshot(
        config(),
        &bad_quotes,
        &market_read(Some(book(100.0, 101.0)), 10),
        1_000,
    );
    assert_eq!(
        mismatched_quote.quality,
        OnchainComparisonQuality::MappingInvalid
    );
    assert!(mismatched_quote.degradation_reasons[0].contains("quote identity"));

    let mut wrong_book = book(100.0, 101.0);
    wrong_book.symbol = "SOLUSDT".to_owned();
    let mismatched_book = projected_with(config(), &market_read(Some(wrong_book), 10));
    assert_eq!(
        mismatched_book.quality,
        OnchainComparisonQuality::MappingInvalid
    );
    assert!(mismatched_book.degradation_reasons[0].contains("orderbook identity"));
}

#[test]
fn cross_quote_pair_is_explicitly_observation_only() {
    let mut cross_quote = config();
    cross_quote.cex_symbol = "SOL/USDT".to_owned();
    let mut cross_quote_book = book(100.0, 101.0);
    cross_quote_book.symbol = "SOL/USDT".to_owned();

    let snapshot = project_snapshot(
        cross_quote,
        &quotes(),
        &market_read(Some(cross_quote_book), 10),
        1_000,
    );

    assert_eq!(snapshot.quality, OnchainComparisonQuality::RawCrossQuote);
    assert_eq!(snapshot.comparisons.len(), 2);
    assert_eq!(snapshot.comparisons[0].cex_price, 100.0);
    assert_eq!(snapshot.comparisons[1].cex_price, 101.0);
    assert!(snapshot.degradation_reasons[0].contains("不判断净收益"));
}

#[test]
fn cross_quote_uses_directional_ws_conversion_and_charges_the_extra_trade() {
    let mut cross_quote = config();
    cross_quote.cex_symbol = "SOL/USD".to_owned();
    let mut cross_quote_book = book(100.0, 101.0);
    cross_quote_book.symbol = "SOL/USD".to_owned();
    let conversion = shared_types::OnchainQuoteConversionEvidence {
        venue: "kraken".to_owned(),
        symbol: "USDC/USD".to_owned(),
        source: "ws_push".to_owned(),
        cex_quote: "USD".to_owned(),
        onchain_quote: "USDC".to_owned(),
        source_bid: 0.999,
        source_ask: 1.001,
        cex_to_onchain_bid: 1.0 / 1.001,
        cex_to_onchain_ask: 1.0 / 0.999,
        cex_to_onchain_capacity: 10_000.0,
        onchain_to_cex_capacity: 10_000.0,
        freshness_ms: 5,
        observed_at_ms: 995,
    };

    let snapshot = project_snapshot_with_conversion(
        cross_quote,
        &quotes(),
        &market_read(Some(cross_quote_book), 10),
        Ok(Some(conversion)),
        Ok(usd_valuation::fixture("USDC", 1.0, 1_000)),
        1_000,
    );

    assert_eq!(snapshot.quality, OnchainComparisonQuality::Fresh);
    assert_eq!(
        snapshot.quote_conversion.as_ref().unwrap().symbol,
        "USDC/USD"
    );
    assert!((snapshot.comparisons[0].cex_price - (100.0 / 1.001)).abs() < 1e-9);
    assert!((snapshot.comparisons[1].cex_price - (101.0 / 0.999)).abs() < 1e-9);
    assert!(snapshot
        .comparisons
        .iter()
        .all(|row| row.quote_conversion_fee_bps == 1.0));
}

#[test]
fn usd_valuation_scales_notional_before_deducting_dollar_gas_and_reaches_batch() {
    let config = config();
    let read = market_read(Some(book(103.0, 104.0)), 10);
    let normal = project_snapshot(config.clone(), &quotes(), &read, 1_000);
    let valued = project_snapshot_with_conversion(
        config.clone(),
        &quotes(),
        &read,
        Ok(None),
        Ok(usd_valuation::fixture("USDC", 0.5, 1_000)),
        1_000,
    );
    for (before, after) in normal.comparisons.iter().zip(&valued.comparisons) {
        assert_eq!(
            after.observable_notional_usd,
            before.observable_notional_usd * 0.5
        );
        assert_eq!(after.gas_bps, before.gas_bps * 2.0);
        let old_profit = before.observable_notional_usd * before.net_spread_bps / 10_000.0;
        let new_profit = after.observable_notional_usd * after.net_spread_bps / 10_000.0;
        assert!((new_profit - ((old_profit + config.gas_usd) * 0.5 - config.gas_usd)).abs() < 1e-9);
    }
    assert_eq!(valued.comparisons.len(), 2);
    let batch = projection::batch_item_snapshot("valuation-fixture".to_owned(), &valued);
    assert_eq!(batch.quote_usd_valuation, valued.quote_usd_valuation);
    assert!(batch.observable_notional_usd.is_some());
    let missing = project_snapshot_with_conversion(
        config.clone(),
        &quotes(),
        &read,
        Ok(None),
        Err("USDC/USD WS unavailable".to_owned()),
        1_000,
    );
    assert_eq!(missing.quality, OnchainComparisonQuality::ValuationPending);
    assert!(missing.comparisons.is_empty());
    assert!(missing.cex_freshness_ms.is_some());
    assert!(missing.degradation_reasons[0].contains("USDC/USD"));
    let wrong_asset = project_snapshot_with_conversion(
        config,
        &quotes(),
        &read,
        Ok(None),
        Ok(usd_valuation::fixture("USDT", 1.0, 1_000)),
        1_000,
    );
    assert_eq!(
        wrong_asset.quality,
        OnchainComparisonQuality::ValuationPending
    );
}

#[test]
fn custom_cex_base_is_monitored_as_two_independent_markets() {
    let mut custom_pair = config();
    custom_pair.cex_symbol = "ETH/USDC".to_owned();
    let mut custom_book = book(3_000.0, 3_001.0);
    custom_book.symbol = "ETH/USDC".to_owned();

    let snapshot = project_snapshot(
        custom_pair,
        &quotes(),
        &market_read(Some(custom_book), 10),
        1_000,
    );

    assert_eq!(snapshot.quality, OnchainComparisonQuality::RawCustomPair);
    assert_eq!(snapshot.comparisons.len(), 2);
    assert!(snapshot.comparisons.iter().all(|row| !row.executable));
    assert!(snapshot.degradation_reasons[0].contains("不同 Base"));
    assert!(snapshot.degradation_reasons[0].contains("不判断净收益"));
}

#[test]
fn precision_only_alias_never_upgrades_matching_cex_text_to_verified_profit() {
    let mut provisional = config();
    provisional.base_identity_resolved = false;

    let snapshot = projected_with(provisional, &market_read(Some(book(100.0, 101.0)), 10));

    assert_eq!(snapshot.quality, OnchainComparisonQuality::RawCustomPair);
    assert_eq!(snapshot.comparisons.len(), 2);
    assert!(snapshot.comparisons.iter().all(|row| !row.executable));
    assert!(snapshot.degradation_reasons[0].contains("符号身份尚未核验"));
}

#[test]
fn stale_low_liquidity_and_no_profit_are_distinct_products_states() {
    let stale = projected_with(config(), &market_read(Some(book(100.0, 101.0)), 6_000));
    assert_eq!(stale.quality, OnchainComparisonQuality::Stale);

    let mut low_config = config();
    low_config.min_liquidity_usd = 1_000.0;
    let low = projected_with(low_config, &market_read(Some(book(100.0, 101.0)), 10));
    assert_eq!(low.quality, OnchainComparisonQuality::LowLiquidity);

    let no_profit_quotes = quote_pair(
        provider_quote("base", "quote", "1000000000", "100000000"),
        provider_quote("quote", "base", "100000000", "990000000"),
    );
    let no_profit = project_snapshot(
        config(),
        &no_profit_quotes,
        &market_read(Some(book(100.0, 100.0)), 10),
        1_000,
    );
    assert_eq!(no_profit.quality, OnchainComparisonQuality::NoNetProfit);

    let mut below_threshold_config = config();
    below_threshold_config.spread_alert.min_net_spread_bps = 10_000.0;
    let below_threshold = projected_with(
        below_threshold_config,
        &market_read(Some(book(100.0, 101.0)), 10),
    );
    assert!(below_threshold
        .comparisons
        .iter()
        .any(|row| row.net_spread_bps > 0.0));
    assert_eq!(
        below_threshold.quality,
        OnchainComparisonQuality::NoNetProfit
    );
    assert!(below_threshold.degradation_reasons[0].contains("低于配置门槛"));
}

#[test]
fn one_small_quote_direction_does_not_hide_a_liquid_profitable_direction() {
    let low_price_quotes = quote_pair(
        provider_quote("base", "quote", "1000000000", "2500000"),
        provider_quote("quote", "base", "100000000", "41000000000"),
    );
    let mut liquid_book = book(2.5, 2.51);
    liquid_book.bids = vec![[2.5, 100.0]];
    liquid_book.asks = vec![[2.51, 100.0]];
    let snapshot = project_snapshot(
        config(),
        &low_price_quotes,
        &market_read(Some(liquid_book), 10),
        1_000,
    );

    assert_eq!(snapshot.quality, OnchainComparisonQuality::Fresh);
    assert_eq!(snapshot.comparisons[0].observable_notional_usd, 100.0);
    assert_eq!(snapshot.comparisons[1].observable_notional_usd, 2.51);
}

#[test]
fn sell_side_capacity_is_expressed_as_entry_capital_and_charges_full_gas() {
    let mut config = config();
    config.cex_taker_fee_bps = 0.0;
    config.slippage_bps = 0.0;
    config.gas_usd = 0.2;
    let mut shallow_book = book(101.0, 101.0);
    shallow_book.bids = vec![[101.0, 0.1]];
    let snapshot = project_snapshot(
        config,
        &quotes(),
        &market_read(Some(shallow_book), 10),
        1_000,
    );
    let row = &snapshot.comparisons[0];
    // Buying 0.1 SOL costs 10 USDC, sells for 10.1, and still pays 0.2 gas.
    assert!((row.observable_notional_usd - 10.0).abs() < 1e-9);
    assert!((row.gas_bps - 200.0).abs() < 1e-9);
    let profit = row.observable_notional_usd * row.net_spread_bps / 10_000.0;
    assert!((profit + 0.1).abs() < 1e-9);
}

#[test]
fn observable_notional_uses_cumulative_cex_depth_inside_slippage_limit() {
    let mut config = config();
    config.slippage_bps = 10.0;
    let mut shallow_book = book(100.0, 101.0);
    shallow_book.asks = vec![[101.0, 0.3], [101.05, 0.5], [102.0, 100.0]];

    let snapshot = project_snapshot(
        config,
        &quotes(),
        &market_read(Some(shallow_book), 10),
        1_000,
    );
    let cex_buy_row = &snapshot.comparisons[1];

    assert!((cex_buy_row.observable_notional_usd - 80.825).abs() < 1e-9);
}

#[test]
fn each_direction_uses_its_own_onchain_base_amount_for_cex_notional() {
    let asymmetric_quotes = quote_pair(
        provider_quote("base", "quote", "2000000000", "210000000"),
        provider_quote("quote", "base", "50000000", "500000000"),
    );
    let mut deep_book = book(100.0, 101.0);
    deep_book.bids = vec![[100.0, 10.0]];
    deep_book.asks = vec![[101.0, 10.0]];

    let snapshot = project_snapshot(
        config(),
        &asymmetric_quotes,
        &market_read(Some(deep_book), 10),
        1_000,
    );

    assert_eq!(snapshot.comparisons[0].observable_notional_usd, 50.0);
    assert_eq!(snapshot.comparisons[1].observable_notional_usd, 202.0);
}

#[test]
fn upstream_failure_snapshot_is_explicit_and_read_only() {
    let snapshot = degraded_without_quotes(
        config(),
        OnchainComparisonQuality::UpstreamUnavailable,
        "Jupiter quote returned HTTP 429",
        1_000,
    );

    assert_eq!(
        snapshot.quality,
        OnchainComparisonQuality::UpstreamUnavailable
    );
    assert!(snapshot.read_only);
    assert!(snapshot.degradation_reasons[0].contains("429"));
}

#[test]
fn pending_snapshot_is_distinct_from_an_upstream_failure() {
    let snapshot = pending_snapshot(config(), "正在等待首轮链上双向报价", 1_000);

    assert_eq!(snapshot.quality, OnchainComparisonQuality::Pending);
    assert!(snapshot.comparisons.is_empty());
    assert!(snapshot.read_only);
    assert!(snapshot.degradation_reasons[0].contains("等待首轮"));
}

#[test]
fn quote_wait_preserves_provider_failure_and_attaches_cex_state() {
    let config = config();
    let current = degraded_without_quotes(
        config.clone(),
        OnchainComparisonQuality::UpstreamUnavailable,
        "Jupiter request timed out",
        1_000,
    );
    let mut next = quote_wait_snapshot(&current, config, 2_000);
    let mut cex = MarketRead {
        quality: MarketQuality::Warming,
        value: None,
        freshness_ms: None,
        source: MarketSource::LocalCache,
        retry_after_ms: Some(5_000),
        last_error: Some("Kraken PUPS/USD 盘口订阅未确认".to_owned()),
    };

    attach_cex_wait_state(&mut next, &cex, 2_000);
    cex.retry_after_ms = Some(2_500);
    attach_cex_wait_state(&mut next, &cex, 2_100);

    assert_eq!(next.quality, OnchainComparisonQuality::UpstreamUnavailable);
    assert!(next
        .degradation_reasons
        .iter()
        .any(|reason| reason.contains("Jupiter request timed out")));
    assert!(next
        .degradation_reasons
        .iter()
        .any(|reason| reason.contains("盘口订阅未确认")));
    assert_eq!(
        next.degradation_reasons
            .iter()
            .filter(|reason| reason.contains("现货 WS 最优买卖价暂不可用"))
            .count(),
        1
    );
    assert_eq!(next.cex_source, "ws_pending");
    assert_eq!(
        next.cex_problem.as_deref(),
        Some("Kraken PUPS/USD 盘口订阅未确认")
    );
    assert_eq!(next.cex_retry_after_ms, Some(2_500));
}

fn projected_with(
    config: OnchainComparisonConfig,
    read: &MarketRead<shared_types::OrderBookInfo>,
) -> OnchainComparisonSnapshot {
    project_snapshot(config, &quotes(), read, 1_000)
}

fn config() -> OnchainComparisonConfig {
    OnchainComparisonConfig {
        enabled: true,
        base_mint: "base".to_owned(),
        quote_mint: "quote".to_owned(),
        base_amount_raw: "1000000000".to_owned(),
        quote_amount_raw: "100000000".to_owned(),
        base_decimals: 9,
        quote_decimals: 6,
        cex_taker_fee_bps: 1.0,
        slippage_bps: 1.0,
        gas_usd: 0.01,
        min_liquidity_usd: 50.0,
        max_age_ms: 5_000,
        ..OnchainComparisonConfig::default()
    }
}

fn quotes() -> OnchainQuotePair {
    quote_pair(
        provider_quote("base", "quote", "1000000000", "105000000"),
        provider_quote("quote", "base", "100000000", "1000000000"),
    )
}

fn quote_pair(forward: ProviderQuote, reverse: ProviderQuote) -> OnchainQuotePair {
    OnchainQuotePair {
        chain: "solana".to_owned(),
        provider: "jupiter_swap_v2".to_owned(),
        endpoint: quote::JUPITER_ORDER_ENDPOINT.to_owned(),
        official_docs_url: JUPITER_ORDER_DOCS.to_owned(),
        base_address: "base".to_owned(),
        quote_address: "quote".to_owned(),
        forward,
        reverse,
        observed_at_ms: 990,
        request_latency_ms: 42,
        quote_interval_ms: 5_000,
    }
}

fn provider_quote(
    input_address: &str,
    output_address: &str,
    input_amount_raw: &str,
    output_amount_raw: &str,
) -> ProviderQuote {
    ProviderQuote {
        input_address: input_address.to_owned(),
        output_address: output_address.to_owned(),
        input_amount_raw: input_amount_raw.to_owned(),
        output_amount_raw: output_amount_raw.to_owned(),
        router: Some("iris".to_owned()),
    }
}

fn book(bid: f64, ask: f64) -> shared_types::OrderBookInfo {
    shared_types::OrderBookInfo {
        symbol: "SOLUSDC".to_owned(),
        exchange: "binance".to_owned(),
        bids: vec![[bid, 10.0]],
        asks: vec![[ask, 10.0]],
        timestamp: 990,
    }
}

fn market_read(
    value: Option<shared_types::OrderBookInfo>,
    freshness_ms: i64,
) -> MarketRead<shared_types::OrderBookInfo> {
    MarketRead {
        quality: if value.is_some() {
            MarketQuality::Fresh
        } else {
            MarketQuality::Missing
        },
        value,
        freshness_ms: Some(freshness_ms),
        source: MarketSource::WsPush,
        retry_after_ms: None,
        last_error: None,
    }
}
