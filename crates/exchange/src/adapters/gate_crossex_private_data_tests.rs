use super::*;
use shared_types::{
    ExecutionMode, ExecutionSizingPlan, FeeProduct, OrderSide, OrderSource, StrategyKind,
};

fn intent(order_type: OrderType, side: OrderSide) -> OrderIntent {
    OrderIntent {
        id: "internal-1".to_owned(),
        source: OrderSource::Manual,
        strategy: Some(StrategyKind::PerpCross),
        mode: ExecutionMode::Live,
        exchange: "gate_crossex:okx".to_owned(),
        symbol: "ADA".to_owned(),
        side,
        order_type,
        quantity: 10.25,
        price: (order_type != OrderType::Market).then_some(0.65),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: order_type == OrderType::PostOnly,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "Crossline Order/1".to_owned(),
        client_order_id_policy: None,
        created_at_ms: 1_756_434_169_000,
    }
}

#[test]
fn compiles_perp_post_only_without_guessing_units() {
    let route = CrossExRoute::parse("OKX_FUTURE_ADA_USDT").unwrap();
    let compiled =
        compile_order(&intent(OrderType::PostOnly, OrderSide::Sell), None, &route).unwrap();

    assert_eq!(compiled.symbol, "OKX_FUTURE_ADA_USDT");
    assert_eq!(compiled.order_type, "LIMIT");
    assert_eq!(compiled.time_in_force, "POC");
    assert_eq!(compiled.qty.as_deref(), Some("10.25"));
    assert_eq!(compiled.price.as_deref(), Some("0.65"));
    assert!(compiled.text.starts_with("cx-"));
}

#[test]
fn spot_market_buy_requires_ticket_quote_notional() {
    let route = CrossExRoute::parse("BINANCE_SPOT_ADA_USDT").unwrap();
    let intent = intent(OrderType::Market, OrderSide::Buy);
    assert!(compile_order(&intent, None, &route).is_err());

    let context = OrderSubmissionContext {
        product: FeeProduct::Spot,
        sizing_plan: Some(ExecutionSizingPlan {
            actual_notional_usd: 12.5,
            ..Default::default()
        }),
        ..Default::default()
    };
    let compiled = compile_order(&intent, Some(&context), &route).unwrap();
    assert_eq!(compiled.qty, None);
    assert_eq!(compiled.quote_qty.as_deref(), Some("12.5"));
}

#[test]
fn parses_official_order_asset_position_and_fill_frames() {
    let order = parse_private_push(include_str!(
        "../../fixtures/gate_crossex/private_order_update.json"
    ))
    .unwrap();
    let Some(PrivatePush::Order(order)) = order else {
        panic!("missing order")
    };
    assert_eq!(order.exchange, "gate_crossex:okx");
    assert_eq!(order.status, OrderStatus::PartiallyFilled);
    assert_eq!(order.filled_quantity, 5.0);

    let asset = parse_private_push(include_str!(
        "../../fixtures/gate_crossex/private_asset_update.json"
    ))
    .unwrap();
    let Some(PrivatePush::Balance(asset)) = asset else {
        panic!("missing asset")
    };
    assert_eq!(asset.venue, "gate_crossex");
    assert_eq!(asset.currency, "USDT");
    assert_eq!(asset.available, 9940.013209);

    let position = parse_private_push(include_str!(
        "../../fixtures/gate_crossex/private_position_update.json"
    ))
    .unwrap();
    let Some(PrivatePush::Position(position)) = position else {
        panic!("missing position")
    };
    assert_eq!(position.row.unwrap().exchange, "gate_crossex:okx");

    let fill = parse_private_push(include_str!(
        "../../fixtures/gate_crossex/private_fill_update.json"
    ))
    .unwrap();
    let Some(PrivatePush::Fill(fill)) = fill else {
        panic!("missing fill")
    };
    assert_eq!(fill.order_id, "2072784922592768");
    assert_eq!(fill.filled_quantity, 13.36);
}

#[test]
fn parses_ws_api_ack_and_rejects_wrong_request() {
    let place = include_str!("../../fixtures/gate_crossex/place_order_ack.json");
    assert!(is_api_response_for(place, "place_order", "request-1").unwrap());
    let ack = parse_api_ack(place, "place_order", "request-1").unwrap();
    assert_eq!(ack.order_id.as_deref(), Some("2072652940337152"));
    assert!(parse_api_ack(place, "place_order", "wrong").is_err());

    let cancel = include_str!("../../fixtures/gate_crossex/cancel_order_ack.json");
    assert!(is_api_response_for(cancel, "cancel_order", "request-1").unwrap());
    let ack = parse_api_ack(cancel, "cancel_order", "request-1").unwrap();
    assert_eq!(ack.order_id, None);
    assert_eq!(ack.message, "success");
}

#[test]
fn parses_bounded_rest_bootstrap_rows() {
    let order = parse_order_text(include_str!(
        "../../fixtures/gate_crossex/order_detail.json"
    ))
    .unwrap();
    assert_eq!(order.status, OrderStatus::Filled);

    let positions =
        parse_positions_text(include_str!("../../fixtures/gate_crossex/positions.json")).unwrap();
    assert_eq!(positions.len(), 1);

    let (balances, summary) = parse_account_text(
        include_str!("../../fixtures/gate_crossex/account.json"),
        1_756_434_169_000,
    )
    .unwrap();
    assert_eq!(balances.len(), 1);
    assert_eq!(summary.total_equity_usd, 1200.0);
    assert_eq!(summary.equity_scope, AccountEquityScope::Unified);

    let orders =
        parse_orders_text(include_str!("../../fixtures/gate_crossex/open_orders.json")).unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].status, OrderStatus::Open);
}
