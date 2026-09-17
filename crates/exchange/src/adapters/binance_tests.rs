use super::*;
use crate::adapters::binance_market_data::{BookTickerItem, PremiumIndexItem, Ticker24hItem};
use crate::adapters::binance_private_data::{parse_open_order, OpenOrderItem};
use pretty_assertions::assert_eq;
use shared_types::OrderSide;
use shared_types::OrderStatus;
use shared_types::OrderType;

fn binance() -> Binance {
    Binance::new(BinanceConfig::default()).unwrap()
}

#[test]
fn name() {
    assert_eq!(ExchangeAdapter::name(&binance()), "binance");
}

#[test]
fn symbol_round_trip() {
    let b = binance();
    assert_eq!(b.to_exchange_symbol("BTC"), "BTCUSDT");
    assert_eq!(b.to_exchange_symbol("ETH"), "ETHUSDT");
    // 已经是交易所符号也能 idempotent 处理
    assert_eq!(b.to_exchange_symbol("BTCUSDT"), "BTCUSDT");
    assert_eq!(b.to_exchange_symbol("BTCUSDC"), "BTCUSDC");
    assert_eq!(b.to_exchange_symbol("BTC-USDC-SWAP"), "BTCUSDC");
    assert_eq!(b.normalize_symbol("BTCUSDT"), "BTC");
}

#[test]
fn discovery_defaults_to_usdt_but_exact_requests_keep_usdc() {
    assert!(include_discovery_perp("BTCUSDT", None));
    assert!(!include_discovery_perp("BTCUSDC", None));
    let requested = std::collections::HashSet::from(["BTCUSDC".to_owned()]);
    assert!(include_discovery_perp("BTCUSDC", Some(&requested)));
    assert!(!include_discovery_perp("BTCUSDT", Some(&requested)));
}

#[test]
fn symbol_resolution_picks_listed_thousand_prefix() {
    let b = binance();
    // 冷启动（上市集为空）：保持历史行为，取首候选。
    assert_eq!(b.to_exchange_symbol("SHIB"), "SHIBUSDT");
    // REST 全量应答喂入上市集后：SHIB 解析到真实上市的 1000SHIBUSDT。
    b.note_listed_usdm(["1000SHIBUSDT", "BTCUSDT", "1MBABYDOGEUSDT"].into_iter());
    assert_eq!(b.to_exchange_symbol("SHIB"), "1000SHIBUSDT");
    assert_eq!(b.to_exchange_symbol("BABYDOGE"), "1MBABYDOGEUSDT");
    assert_eq!(b.to_exchange_symbol("BTC"), "BTCUSDT");
    // 上市集非空但候选全不在集内：仍取首候选（fail-open，与 REST 行为一致）。
    assert_eq!(b.to_exchange_symbol("NOTLISTED"), "NOTLISTEDUSDT");
}

#[test]
fn parse_premium_funding_8h_default() {
    let item = PremiumIndexItem {
        symbol: "BTCUSDT".into(),
        mark_price: String::new(),
        index_price: String::new(),
        last_funding_rate: "0.0001".into(),
        next_funding_time: 1_700_000_000_000,
        time: 1_699_999_900_000,
    };
    let f = parse_funding(&item, 1_234_567.0, 8).expect("funding parses");
    assert_eq!(f.symbol, "BTC");
    assert_eq!(f.exchange, "binance");
    assert!((f.rate - 0.0001).abs() < 1e-12);
    assert!((f.rate_8h - 0.0001).abs() < 1e-12);
    assert_eq!(f.next_funding_time, 1_700_000_000_000);
    assert_eq!(f.funding_interval, 8);
    assert!((f.volume_24h - 1_234_567.0).abs() < 1e-9);
}

#[test]
fn parse_premium_funding_4h_doubles_rate_8h() {
    // 修复 P1 1.1：4h 合约的单期 0.01% 等价 8h 0.02%。
    let item = PremiumIndexItem {
        symbol: "DOGEUSDT".into(),
        mark_price: String::new(),
        index_price: String::new(),
        last_funding_rate: "0.0001".into(),
        next_funding_time: 1_700_000_000_000,
        time: 0,
    };
    let f = parse_funding(&item, 0.0, 4).expect("funding parses");
    assert_eq!(f.funding_interval, 4);
    assert!((f.rate - 0.0001).abs() < 1e-12);
    assert!(
        (f.rate_8h - 0.0002).abs() < 1e-12,
        "rate_8h should be doubled for 4h interval, got {}",
        f.rate_8h
    );
}

#[test]
fn parse_premium_funding_1h_multiplies_rate_8h_by_8() {
    let item = PremiumIndexItem {
        symbol: "VOLATILEUSDT".into(),
        mark_price: String::new(),
        index_price: String::new(),
        last_funding_rate: "0.0001".into(),
        next_funding_time: 1_700_000_000_000,
        time: 0,
    };
    let f = parse_funding(&item, 0.0, 1).expect("funding parses");
    assert_eq!(f.funding_interval, 1);
    assert!((f.rate_8h - 0.0008).abs() < 1e-12);
}

#[test]
fn parse_premium_funding_clamps_invalid_interval() {
    // interval=0 是数据异常，应 clamp 到 1 避免除零
    let item = PremiumIndexItem {
        symbol: "BTCUSDT".into(),
        mark_price: String::new(),
        index_price: String::new(),
        last_funding_rate: "0.0001".into(),
        next_funding_time: 1_700_000_000_000,
        time: 0,
    };
    let f = parse_funding(&item, 0.0, 0).expect("funding parses");
    assert_eq!(f.funding_interval, 1); // clamped
    assert!((f.rate_8h - 0.0008).abs() < 1e-12);
}

#[test]
fn parse_premium_funding_rejects_missing_settlement_time() {
    let item = PremiumIndexItem {
        symbol: "BTCUSDT".into(),
        mark_price: String::new(),
        index_price: String::new(),
        last_funding_rate: "0.0001".into(),
        next_funding_time: 0,
        time: 0,
    };
    assert!(parse_funding(&item, 0.0, 8).is_none());
}

#[test]
fn parse_ticker_field_mapping() {
    let t = Ticker24hItem {
        symbol: "ETHUSDT".into(),
        last_price: "2000.5".into(),
        bid_price: "2000.0".into(),
        ask_price: "2001.0".into(),
        quote_volume: "9999.99".into(),
        close_time: 1_699_999_999_000,
    };
    let info = parse_ticker(&t, "ETH", None).expect("ticker parses");
    assert_eq!(info.symbol, "ETH");
    assert_eq!(info.exchange, "binance");
    assert!((info.bid - 2000.0).abs() < 1e-9);
    assert!((info.ask - 2001.0).abs() < 1e-9);
    assert!((info.last - 2000.5).abs() < 1e-9);
    assert!((info.volume_24h - 9999.99).abs() < 1e-9);
    assert_eq!(info.timestamp, 1_699_999_999_000);
}

#[test]
fn parse_ticker_uses_book_ticker_bid_ask() {
    let t = Ticker24hItem {
        symbol: "PROMPTUSDT".into(),
        last_price: "0.0468".into(),
        bid_price: String::new(),
        ask_price: String::new(),
        quote_volume: "124128782.8".into(),
        close_time: 1_779_238_646_010,
    };
    let book = BookTickerItem {
        symbol: "PROMPTUSDT".into(),
        bid_price: "0.04687".into(),
        ask_price: "0.04688".into(),
        time: 1_779_238_646_050,
    };

    let info = parse_ticker(&t, "PROMPT", Some(&book)).expect("ticker parses");

    assert!((info.bid - 0.04687).abs() < 1e-12);
    assert!((info.ask - 0.04688).abs() < 1e-12);
    assert!((info.last - 0.0468).abs() < 1e-12);
    assert_eq!(info.timestamp, 1_779_238_646_050);
}

#[test]
fn parse_spot_tick_pair_symbol() {
    let t = Ticker24hItem {
        symbol: "BTCUSDT".into(),
        last_price: "30000.5".into(),
        bid_price: "30000.0".into(),
        ask_price: "30001.0".into(),
        quote_volume: "1000000".into(),
        close_time: 1_700_000_000_000,
    };
    let tick = parse_spot_tick(&t).unwrap();
    assert_eq!(tick.venue, "binance");
    assert_eq!(tick.symbol, "BTC/USDT");
    assert_eq!(tick.exchange_ts_ms, Some(1_700_000_000_000));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn parse_open_order_basic() {
    let raw = OpenOrderItem {
        order_id: 12345,
        symbol: "BTCUSDT".into(),
        status: "NEW".into(),
        order_type: "LIMIT".into(),
        side: "BUY".into(),
        price: "30000".into(),
        orig_qty: "0.01".into(),
        executed_qty: "0".into(),
        avg_price: "0".into(),
        time: Some(1_700_000_000_000),
        update_time: None,
        time_in_force: "GTC".into(),
        client_order_id: String::new(),
        reduce_only: false,
    };
    let o = parse_open_order(&raw).expect("open order parses");
    assert_eq!(o.order_id, "12345");
    assert_eq!(o.symbol, "BTC");
    assert_eq!(o.exchange, "binance");
    assert!(matches!(o.side, OrderSide::Buy));
    assert!(matches!(o.order_type, OrderType::Limit));
    assert!(matches!(o.status, OrderStatus::Open));
    assert!((o.quantity - 0.01).abs() < 1e-9);
    assert!((o.price - 30000.0).abs() < 1e-9);
}

#[test]
fn parse_open_order_recognizes_post_only_via_gtx() {
    // 修复 P2 1.8：PostOnly 通过 `type=LIMIT, timeInForce=GTX` 表达。
    let raw = OpenOrderItem {
        order_id: 9999,
        symbol: "ETHUSDT".into(),
        status: "NEW".into(),
        order_type: "LIMIT".into(),
        side: "SELL".into(),
        price: "2000".into(),
        orig_qty: "1".into(),
        executed_qty: "0".into(),
        avg_price: "0".into(),
        time: Some(1_700_000_000_000),
        update_time: None,
        time_in_force: "GTX".into(),
        client_order_id: String::new(),
        reduce_only: false,
    };
    let o = parse_open_order(&raw).expect("post-only order parses");
    assert!(matches!(o.order_type, OrderType::PostOnly));
    assert!(matches!(o.side, OrderSide::Sell));
}

#[test]
fn parse_open_order_market_overrides_tif() {
    let raw = OpenOrderItem {
        order_id: 7777,
        symbol: "BTCUSDT".into(),
        status: "FILLED".into(),
        order_type: "MARKET".into(),
        side: "BUY".into(),
        price: "0".into(),
        orig_qty: "0.5".into(),
        executed_qty: "0.5".into(),
        avg_price: "30000".into(),
        time: Some(1_700_000_000_000),
        update_time: None,
        time_in_force: "IOC".into(), // Market 单内部 TIF 是 IOC
        client_order_id: String::new(),
        reduce_only: false,
    };
    let o = parse_open_order(&raw).expect("market order parses");
    assert!(matches!(o.order_type, OrderType::Market));
}

#[test]
fn require_credentials_errors_when_missing() {
    let b = Binance::new(BinanceConfig::default()).unwrap();
    assert!(b.require_credentials().is_err());
}

fn make_position(symbol: &str, side: &str, qty: f64) -> PositionInfo {
    PositionInfo {
        symbol: symbol.into(),
        exchange: NAME.into(),
        side: side.into(),
        quantity: qty,
        entry_price: 100.0,
        mark_price: 100.0,
        unrealized_pnl: 0.0,
        leverage: 1.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 0.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

#[test]
fn pair_hedge_positions_links_long_and_short_for_same_symbol() {
    // 修复 P2 1.10：Hedge Mode 下同 symbol 的 LONG/SHORT 互相标记。
    // 算法在 `crate::adapter::pair_hedge_positions`，跨 binance/bybit 共享。
    let mut positions = vec![
        make_position("BTC", "long", 0.5),
        make_position("BTC", "short", 0.3),
        make_position("ETH", "long", 1.0), // One-Way Mode，无 peer
    ];
    crate::adapter::pair_hedge_positions(&mut positions);

    let btc_long = positions
        .iter()
        .find(|p| p.symbol == "BTC" && p.side == "long")
        .unwrap();
    let btc_short = positions
        .iter()
        .find(|p| p.symbol == "BTC" && p.side == "short")
        .unwrap();
    let eth = positions.iter().find(|p| p.symbol == "ETH").unwrap();

    assert_eq!(btc_long.paired_with.as_deref(), Some("binance:BTC:short"));
    assert_eq!(btc_short.paired_with.as_deref(), Some("binance:BTC:long"));
    assert!(
        eth.paired_with.is_none(),
        "single position should have no peer"
    );
}

#[test]
fn pair_hedge_positions_no_op_for_empty_or_single() {
    let mut empty: Vec<PositionInfo> = Vec::new();
    crate::adapter::pair_hedge_positions(&mut empty);
    assert!(empty.is_empty());

    let mut single = vec![make_position("BTC", "long", 1.0)];
    crate::adapter::pair_hedge_positions(&mut single);
    assert!(single[0].paired_with.is_none());
}

#[test]
fn snap_binance_depth_maps_to_legal_levels() {
    // 修复 P2 1.7：Binance `/fapi/v1/depth` 仅接受离散值 {5,10,20,50,100,500,1000}。
    // `min_by_key` 在 tie 时返回**第一个**最小元素（按数组顺序）。
    assert_eq!(snap_binance_depth(5), 5);
    assert_eq!(snap_binance_depth(0), 5); // 0 距 5 是 5，最近
    assert_eq!(snap_binance_depth(7), 5); // 7 距 5=2，距 10=3 → 5
    assert_eq!(snap_binance_depth(8), 10); // 8 距 5=3，距 10=2 → 10
    assert_eq!(snap_binance_depth(27), 20); // 27 距 20=7，距 50=23 → 20
    assert_eq!(snap_binance_depth(75), 50); // 75 距 50=25，距 100=25 → tie 取首个 50
    assert_eq!(snap_binance_depth(200), 100); // 200 距 100=100，距 500=300 → 100
    assert_eq!(snap_binance_depth(300), 100); // 300 距 100=200，距 500=200 → tie 取首个 100
    assert_eq!(snap_binance_depth(750), 500); // 750 距 500=250，距 1000=250 → tie 取首个 500
    assert_eq!(snap_binance_depth(1500), 1000); // 上界
}

#[test]
fn parse_ticker_drops_tick_when_no_book_bid_ask() {
    let t = Ticker24hItem {
        symbol: "BTCUSDT".into(),
        last_price: "30000.0".into(),
        bid_price: String::new(),
        ask_price: String::new(),
        quote_volume: "1.0".into(),
        close_time: 1,
    };
    assert!(parse_ticker(&t, "BTC", None).is_none());
}

#[test]
fn parse_ticker_drops_tick_with_unparseable_last() {
    let t = Ticker24hItem {
        symbol: "BTCUSDT".into(),
        last_price: "n/a".into(),
        bid_price: "30000.0".into(),
        ask_price: "30001.0".into(),
        quote_volume: "1.0".into(),
        close_time: 1,
    };
    assert!(parse_ticker(&t, "BTC", None).is_none());
}
