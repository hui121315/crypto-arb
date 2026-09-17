use super::{
    binance_private_data, bitget_uta_private_data, bybit_private_data, gate_private_data,
    hyperliquid_private_data, kucoin_private_data, okx_private_data,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use shared_types::OrderInfo;

#[test]
fn official_private_read_schemas_preserve_order_condition_and_identity() {
    assert_binance();
    assert_okx();
    assert_bybit();
    assert_bitget();
    assert_gate();
    assert_kucoin();
    assert_hyperliquid();
}

fn assert_binance() {
    let rows: Vec<binance_private_data::OpenOrderItem> =
        decode(include_str!("../../fixtures/binance/usdm_open_orders.json"));
    let order = binance_private_data::parse_open_order(&rows[0]).expect("binance order");
    assert_order_semantics(&order, "GTC", Some("binance-cli-1"), false);
}

fn assert_okx() {
    let value = fixture(include_str!("../../fixtures/okx/trade_orders_pending.json"));
    let rows: Vec<okx_private_data::OpenOrderItem> = decode_value(value["data"].clone());
    let order = okx_private_data::parse_open_orders(rows)
        .expect("okx orders")
        .remove(0);
    assert_order_semantics(&order, "limit", Some("okx-cli-1"), false);
}

fn assert_bybit() {
    let value = fixture(include_str!(
        "../../fixtures/bybit/order_realtime_linear_open.json"
    ));
    let rows: Vec<bybit_private_data::OpenOrderRow> = decode_value(value["result"]["list"].clone());
    let order = bybit_private_data::parse_open_order(rows.into_iter().next().expect("bybit row"))
        .expect("bybit order");
    assert_order_semantics(&order, "GTC", None, false);
}

fn assert_bitget() {
    let value = fixture(include_str!(
        "../../fixtures/bitget/uta_unfilled_orders_open.json"
    ));
    let rows: Vec<bitget_uta_private_data::UtaOrderRow> =
        decode_value(value["data"]["list"].clone());
    let order =
        bitget_uta_private_data::parse_open_order(rows.into_iter().next().expect("bitget row"))
            .expect("bitget order");
    assert_order_semantics(&order, "gtc", Some("111111111111111111"), false);
}

fn assert_gate() {
    let rows: Vec<gate_private_data::OpenOrderItem> = decode(include_str!(
        "../../fixtures/gate/futures_usdt_orders_open.json"
    ));
    let order = gate_private_data::parse_open_order(&rows[0]).expect("gate order");
    assert_order_semantics(&order, "gtc", Some("t-cid-1"), false);
}

fn assert_kucoin() {
    let value = fixture(include_str!(
        "../../fixtures/kucoin/get_order_by_client_oid_open.json"
    ));
    let row: kucoin_private_data::OpenOrderItem = decode_value(value["data"].clone());
    let order = kucoin_private_data::parse_open_order(&row).expect("kucoin order");
    assert_order_semantics(&order, "GTC", Some("5c52e11203aa677f33e493fb"), false);
}

fn assert_hyperliquid() {
    let value = fixture(include_str!(
        "../../fixtures/hyperliquid/info_order_status_filled.json"
    ));
    let mut raw = value["order"]["order"].clone();
    raw["sz"] = raw["origSz"].clone();
    let row: hyperliquid_private_data::OpenOrderItem = decode_value(raw);
    let order = hyperliquid_private_data::parse_open_orders(vec![row], None, "hyperliquid")
        .expect("hyperliquid orders")
        .remove(0);
    assert_order_semantics(
        &order,
        "Gtc",
        Some("0x01010101010101010101010101010101"),
        false,
    );
}

fn assert_order_semantics(
    order: &OrderInfo,
    expected_condition: &str,
    expected_client_order_id: Option<&str>,
    expected_reduce_only: bool,
) {
    assert_eq!(
        order.venue_time_in_force.as_deref(),
        Some(expected_condition)
    );
    assert_eq!(order.client_order_id.as_deref(), expected_client_order_id);
    assert_eq!(order.reduce_only, Some(expected_reduce_only));
    assert!(!order.order_id.trim().is_empty());
}

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture JSON")
}

fn decode<T: DeserializeOwned>(text: &str) -> T {
    serde_json::from_str(text).expect("fixture schema")
}

fn decode_value<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("fixture payload schema")
}
