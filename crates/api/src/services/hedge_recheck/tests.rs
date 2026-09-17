use super::*;
use shared_types::{
    ExecutionGuard, ExecutionMode, HedgeDepthStatus, HedgeExecutableNotional, HedgeLegRole,
    HedgeSizing, HedgeTicket, LiveOrderState, MarginMode, OrderBookInfo, OrderSide, OrderSource,
    OrderUpdateSource, SpotLegMode, StrategyKind, TimeInForce, VenueOrderIdentity,
};

#[path = "tests/submit_market.rs"]
mod submit_market;

#[test]
fn open_orders_excluding_self_saturates() {
    assert_eq!(open_orders_excluding_self(0), 0);
    assert_eq!(open_orders_excluding_self(1), 0);
    assert_eq!(open_orders_excluding_self(3), 2);
}

#[test]
fn orderbook_read_blocker_requires_fresh_value() {
    let leg = leg_quote(HedgeLegRole::Long, "okx", "BTCUSDT");
    let read = crate::services::market_data::MarketRead {
        value: None,
        quality: crate::services::market_data::MarketQuality::RateLimited,
        freshness_ms: None,
        source: crate::services::market_data::MarketSource::LocalCache,
        retry_after_ms: Some(2_000),
        last_error: None,
    };

    let blocker = orderbook_read_blocker(&leg, &read).unwrap_or_default();

    assert!(blocker.contains("限频退避"));
    assert!(blocker.contains("2000ms后重试"));
}

#[test]
fn orderbook_read_blocker_keeps_unknown_stale_age_unknown() {
    let leg = leg_quote(HedgeLegRole::Long, "okx", "BTCUSDT");
    let read = crate::services::market_data::MarketRead {
        value: None,
        quality: crate::services::market_data::MarketQuality::StaleAllowed,
        freshness_ms: None,
        source: crate::services::market_data::MarketSource::LocalCache,
        retry_after_ms: None,
        last_error: None,
    };

    let blocker = orderbook_read_blocker(&leg, &read).unwrap_or_default();

    assert!(blocker.contains("旧缓存(未知)"), "{blocker}");
    assert!(!blocker.contains("0ms"), "{blocker}");
}

#[test]
fn orderbook_read_blocker_keeps_unknown_retry_after_unknown() {
    let leg = leg_quote(HedgeLegRole::Short, "binance", "BTCUSDT");
    let read = crate::services::market_data::MarketRead {
        value: None,
        quality: crate::services::market_data::MarketQuality::RateLimited,
        freshness_ms: None,
        source: crate::services::market_data::MarketSource::LocalCache,
        retry_after_ms: None,
        last_error: None,
    };

    let blocker = orderbook_read_blocker(&leg, &read).unwrap_or_default();

    assert!(blocker.contains("限频退避，未知时间后重试"), "{blocker}");
    assert!(!blocker.contains("0ms"), "{blocker}");
}

#[tokio::test]
async fn orderbook_recheck_blocks_missing_perp_leg() -> anyhow::Result<()> {
    let state = test_state().await?;
    let ticket = ticket(Some(StrategyKind::PerpCross), None);
    state.market_data().store_orderbook(
        orderbook("okx", "BTCUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );

    let rejection =
        orderbook_freshness_rejection(&state, &ticket, "执行前盘口复检未通过").unwrap_or_default();

    assert!(rejection.contains("binance BTCUSDT 执行前盘口暂无 fresh 数据"));
    Ok(())
}

#[tokio::test]
async fn spot_perp_recheck_uses_spot_book_for_long_leg() -> anyhow::Result<()> {
    let state = test_state().await?;
    let ticket = ticket(Some(StrategyKind::SpotPerp), Some(SpotLegMode::BuySpot));
    state.market_data().store_orderbook(
        orderbook("okx", "BTCUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );
    state.market_data().store_orderbook(
        orderbook("binance", "BTCUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );

    let missing_spot =
        orderbook_freshness_rejection(&state, &ticket, "执行前盘口复检未通过").unwrap_or_default();

    assert!(missing_spot.contains("okx BTCUSDT 执行前盘口暂无 fresh 数据"));
    state.market_data().store_spot_orderbook(
        "okx",
        orderbook("okx", "BTCUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );
    assert!(orderbook_freshness_rejection(&state, &ticket, "执行前盘口复检未通过").is_none());
    Ok(())
}

#[tokio::test]
async fn spot_cross_recheck_uses_spot_books_for_both_legs() -> anyhow::Result<()> {
    let state = test_state().await?;
    let ticket = ticket(Some(StrategyKind::SpotCross), None);
    for leg in [&ticket.long_leg, &ticket.short_leg] {
        state.market_data().store_orderbook(
            orderbook(&leg.exchange, &leg.symbol),
            crate::services::market_data::MarketSource::WsPush,
        );
    }

    let missing_spot =
        orderbook_freshness_rejection(&state, &ticket, "执行前盘口复检未通过").unwrap_or_default();
    assert!(missing_spot.contains("okx BTCUSDT"));
    assert!(missing_spot.contains("binance BTCUSDT"));

    for leg in [&ticket.long_leg, &ticket.short_leg] {
        state.market_data().store_spot_orderbook(
            &leg.exchange,
            orderbook(&leg.exchange, &leg.symbol),
            crate::services::market_data::MarketSource::WsPush,
        );
    }
    assert!(orderbook_freshness_rejection(&state, &ticket, "执行前盘口复检未通过").is_none());
    Ok(())
}

async fn test_state() -> anyhow::Result<AppState> {
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    AppState::new(config).await
}

#[tokio::test]
async fn spot_perp_inventory_sale_recheck_uses_spot_book_for_short_leg() -> anyhow::Result<()> {
    let state = test_state().await?;
    let ticket = ticket(
        Some(StrategyKind::CrossSpotPerp),
        Some(SpotLegMode::SellInventory),
    );
    state.market_data().store_orderbook(
        orderbook("okx", "BTCUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );
    state.market_data().store_spot_orderbook(
        "binance",
        orderbook("binance", "BTCUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );

    assert!(orderbook_freshness_rejection(&state, &ticket, "执行前盘口复检未通过").is_none());
    Ok(())
}

#[tokio::test]
async fn spot_perp_recheck_blocks_missing_leg_orientation() -> anyhow::Result<()> {
    let state = test_state().await?;
    let ticket = ticket(Some(StrategyKind::SpotPerp), None);

    let rejection =
        orderbook_freshness_rejection(&state, &ticket, "执行前盘口复检未通过").unwrap_or_default();

    assert!(rejection.contains("执行产品未解析"));
    assert!(rejection.contains("现货腿方向证据"));
    Ok(())
}

fn ticket(strategy: Option<StrategyKind>, spot_leg_mode: Option<SpotLegMode>) -> HedgeTicket {
    HedgeTicket {
        ticket_id: "ticket-test".to_owned(),
        opportunity_id: "opp-test".to_owned(),
        strategy,
        spot_leg_mode,
        symbol: "BTCUSDT".to_owned(),
        created_at_ms: common::time::now_ms(),
        market_checked_at_ms: common::time::now_ms(),
        expires_at_ms: common::time::now_ms().saturating_add(60_000),
        long_leg: leg_quote(HedgeLegRole::Long, "okx", "BTCUSDT"),
        short_leg: leg_quote(HedgeLegRole::Short, "binance", "BTCUSDT"),
        cost: None,
        fee_snapshots: Vec::new(),
        sizing: HedgeSizing {
            requested_capital_usd: 100.0,
            leverage: 1.0,
            target_notional_usd: 100.0,
            long_notional_cap_usd: 100.0,
            short_notional_cap_usd: 100.0,
            target_base_quantity: None,
            max_executable_notional: HedgeExecutableNotional {
                status: HedgeDepthStatus::Available,
                amount_usd: Some(100.0),
                ..HedgeExecutableNotional::default()
            },
        },
        guards: vec![ExecutionGuard {
            key: "depth".to_owned(),
            label: "深度".to_owned(),
            passed: true,
            detail: "通过".to_owned(),
            preflight_outcome: None,
        }],
        blockers: Vec::new(),
    }
}

fn leg_quote(role: HedgeLegRole, exchange: &str, symbol: &str) -> HedgeLegQuote {
    HedgeLegQuote {
        role,
        exchange: exchange.to_owned(),
        symbol: symbol.to_owned(),
        side: match role {
            HedgeLegRole::Long => OrderSide::Buy,
            HedgeLegRole::Short => OrderSide::Sell,
        },
        reference_price: Some(100.0),
        bid: Some(99.9),
        ask: Some(100.1),
        mid: Some(100.0),
        open_vwap_price: Some(100.1),
        open_slippage_bps: Some(0.0),
        close_vwap_price: Some(99.9),
        close_slippage_bps: Some(0.0),
        depth_usd_5bps: Some(1_000.0),
        depth_usd_10bps: Some(1_500.0),
        depth_usd_20bps: Some(2_000.0),
        max_notional_usd: Some(1_000.0),
        market_evidence: None,
        depth_health: None,
        depth_reason: None,
        funding_bps: Some(0.0),
        next_funding_time: 0,
        funding_interval_hours: 0,
        market_timestamp_ms: Some(common::time::now_ms()),
        blockers: Vec::new(),
    }
}

fn order_intent(side: OrderSide, price: Option<f64>) -> OrderIntent {
    OrderIntent {
        id: "order-test".to_owned(),
        source: OrderSource::ArbitragePreview,
        strategy: Some(StrategyKind::PerpCross),
        mode: ExecutionMode::DryRun,
        exchange: "okx".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        side,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price,
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-test".to_owned(),
        client_order_id_policy: None,
        created_at_ms: common::time::now_ms(),
    }
}

fn orderbook(exchange: &str, symbol: &str) -> OrderBookInfo {
    OrderBookInfo {
        symbol: symbol.to_owned(),
        exchange: exchange.to_owned(),
        bids: vec![[99.9, 1.0]],
        asks: vec![[100.1, 1.0]],
        timestamp: common::time::now_ms(),
    }
}
