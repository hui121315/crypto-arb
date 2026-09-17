use super::*;
use pretty_assertions::assert_eq;

#[test]
fn parse_balance_response_filters_currency() {
    let parsed = parse_balance_response(
        &account("", "100", "80", "12", "3", "1"),
        Some("USDT"),
        "usdt",
    )
    .unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed["USDT"].total, 100.0);
    assert_eq!(parsed["USDT"].available, 80.0);
    assert_eq!(parsed["USDT"].frozen, 15.0);
    assert_eq!(parsed["USDT"].unrealized_pnl, 1.0);

    let empty = parse_balance_response(
        &account("USDT", "100", "80", "0", "0", "0"),
        Some("BTC"),
        "usdt",
    )
    .unwrap();
    assert!(empty.is_empty());
}

#[test]
fn parse_balance_response_rejects_bad_numeric() {
    assert!(matches!(
        parse_balance_response(&account("USDT", "100", "bad", "0", "0", "0"), None, "usdt"),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid available")
    ));
}

#[test]
fn parse_balance_response_rejects_blank_currency_without_endpoint_settle() {
    assert!(matches!(
        parse_balance_response(&account("", "100", "80", "0", "0", "0"), None, ""),
        Err(crate::ExchangeError::Parse(message))
            if message.contains("endpoint settle unavailable")
    ));
}

#[test]
fn gate_futures_account_balance_parses_official_fixture() {
    let body = include_str!("../../fixtures/gate/futures_usdt_account.json");
    let row: AccountItem = serde_json::from_str(body).expect("official account fixture decodes");
    let parsed = parse_balance_response(&row, Some("USDT"), "usdt").expect("account parses");
    let usdt = parsed.get("USDT").expect("USDT account present");

    assert_eq!(parsed.len(), 1);
    assert_eq!(usdt.total, 4.4516);
    assert_eq!(usdt.available, 4.98);
    assert!((usdt.frozen - 5.2).abs() < 1e-12);
    assert_eq!(usdt.unrealized_pnl, 0.0);
}

#[test]
fn parse_positions_applies_signed_size_and_contract_unit() {
    let rows = vec![
        position("BTC_USDT", 100),
        position("BTC_USDT", -50),
        position("ETH_USDT", 0),
    ];
    let parsed = parse_positions(&rows, Some("BTC_USDT"), |_| Ok(0.0001)).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[1].side, "short");
    assert!((parsed[0].quantity - 0.01).abs() < 1e-12);
    assert!((parsed[1].quantity - 0.005).abs() < 1e-12);
    assert!((parsed[0].maintenance_margin_ratio - 0.005).abs() < 1e-12);
}

#[test]
fn gate_positions_parse_official_fixture() {
    let body = include_str!("../../fixtures/gate/futures_usdt_positions.json");
    let rows: Vec<PositionRow> =
        serde_json::from_str(body).expect("official futures positions fixture decodes");
    let parsed = parse_positions(&rows, None, |_| Ok(0.0001)).expect("positions parse");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].symbol, "BTC");
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[1].symbol, "ETH");
    assert_eq!(parsed[1].side, "short");
    assert!((parsed[0].quantity - 0.01).abs() < 1e-12);
}

#[test]
fn parse_positions_maps_official_maintenance_rate() {
    let mut row = position("BTC_USDT", 100);
    row.maintenance_rate = "0.012".into();

    let parsed = parse_positions(&[row], Some("BTC_USDT"), |_| Ok(0.0001)).unwrap();

    assert!((parsed[0].maintenance_margin_ratio - 0.012).abs() < 1e-12);
}

#[test]
fn parse_positions_keeps_blank_maintenance_rate_as_unknown() {
    let mut row = position("BTC_USDT", 100);
    row.maintenance_rate = String::new();

    let parsed = parse_positions(&[row], Some("BTC_USDT"), |_| Ok(0.0001)).unwrap();

    assert_eq!(parsed[0].maintenance_margin_ratio, 0.0);
}

#[test]
fn parse_positions_rejects_bad_maintenance_rate() {
    let mut row = position("BTC_USDT", 100);
    row.maintenance_rate = "bad".into();

    assert!(matches!(
        parse_positions(&[row], Some("BTC_USDT"), |_| Ok(0.0001)),
        Err(crate::ExchangeError::Parse(message)) if message.contains("maintenance_rate")
    ));
}

#[test]
fn position_size_accepts_gate_official_string_schema() {
    let json = r#"{
        "contract":"BTC_USDT",
        "size":"-9440",
        "entry_price":"3779.55",
        "mark_price":"3780.32",
        "unrealised_pnl":"-0.000507486844",
        "leverage":"0",
        "liq_price":"99999999",
        "margin":"4.431548146258",
        "maintenance_rate":"0.005"
    }"#;

    let row: PositionRow = serde_json::from_str(json).expect("official string size parses");
    let parsed = parse_positions(&[row], Some("BTC_USDT"), |_| Ok(0.0001)).unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].side, "short");
    assert!((parsed[0].quantity - 0.944).abs() < 1e-12);
}

#[test]
fn decimal_position_and_order_sizes_parse_current_gate_schema() {
    #[derive(serde::Deserialize)]
    struct Fixture {
        position: PositionRow,
        order: OpenOrderItem,
    }

    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../fixtures/gate/futures_usdt_decimal_sizes_v4_106.json"
    ))
    .expect("current Gate decimal-size fixture decodes");
    let position = parse_positions(&[fixture.position], None, |_| Ok(0.001))
        .expect("decimal position parses")
        .pop()
        .expect("position");
    let order = parse_open_order(&fixture.order).expect("decimal order parses");

    assert_eq!(position.side, "short");
    assert!((position.quantity - 0.0125).abs() < 1e-12);
    assert_eq!(order.quantity, 5.5);
    assert_eq!(order.filled_quantity, 3.25);
}

#[test]
fn parse_positions_rejects_bad_open_position_numeric() {
    let mut row = position("BTC_USDT", 100);
    row.mark_price = "bad".into();

    assert!(matches!(
        parse_positions(&[row], Some("BTC_USDT"), |_| Ok(0.0001)),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid mark_price")
    ));
}

#[test]
fn parse_positions_rejects_bad_contract_unit() {
    let row = position("BTC_USDT", 100);

    assert!(matches!(
        parse_positions(&[row], Some("BTC_USDT"), |_| Ok(0.0)),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid contract unit")
    ));
}

#[test]
fn parse_positions_propagates_missing_contract_metadata() {
    let row = position("BTC_USDT", 1);
    let error = parse_positions(&[row], None, |contract| {
        Err(ExchangeError::Api {
            exchange: "gate".into(),
            code: "instrument_metadata_missing".into(),
            message: format!("missing multiplier for {contract}"),
        })
    })
    .expect_err("missing multiplier must fail closed");

    assert!(error.to_string().contains("instrument_metadata_missing"));
}

#[test]
fn parse_positions_ignores_zero_size_placeholder_with_blank_fields() {
    let row = PositionRow {
        contract: "BTC_USDT".into(),
        size: 0.0,
        entry_price: String::new(),
        mark_price: String::new(),
        unrealised_pnl: String::new(),
        leverage: String::new(),
        liq_price: String::new(),
        margin: String::new(),
        maintenance_rate: String::new(),
        mode: String::new(),
    };

    let parsed = parse_positions(&[row], Some("BTC_USDT"), |_| Ok(0.0001)).unwrap();

    assert!(parsed.is_empty());
}

#[test]
fn open_order_create_time_requires_positive_integer_milliseconds() {
    let int_json = r#"{"id":1,"contract":"BTC_USDT","status":"open","size":1,"left":1,"price":"30000","fill_price":"0","tif":"gtc","is_reduce_only":false,"create_time_ms":1700028800123}"#;
    let whole_float_json = r#"{"id":2,"contract":"BTC_USDT","status":"open","size":1,"left":1,"price":"30000","fill_price":"0","tif":"gtc","is_reduce_only":false,"create_time_ms":1700028800123.0}"#;
    let string_json = r#"{"id":3,"contract":"BTC_USDT","status":"open","size":1,"left":1,"price":"30000","fill_price":"0","tif":"gtc","is_reduce_only":false,"create_time_ms":"1700028800123"}"#;
    let fractional_json = r#"{"id":4,"contract":"BTC_USDT","status":"open","size":1,"left":1,"price":"30000","fill_price":"0","tif":"gtc","is_reduce_only":false,"create_time_ms":1700028800123.4}"#;
    let null_json = r#"{"id":5,"contract":"BTC_USDT","status":"open","size":1,"left":1,"price":"30000","fill_price":"0","tif":"gtc","is_reduce_only":false,"create_time_ms":null}"#;
    let missing_json = r#"{"id":6,"contract":"BTC_USDT","status":"open","size":1,"left":1,"price":"30000","fill_price":"0","tif":"gtc","is_reduce_only":false}"#;
    let zero_json = r#"{"id":7,"contract":"BTC_USDT","status":"open","size":1,"left":1,"price":"30000","fill_price":"0","tif":"gtc","is_reduce_only":false,"create_time_ms":0}"#;

    let int_row: OpenOrderItem = serde_json::from_str(int_json).expect("int ms parses");
    let whole_float_row: OpenOrderItem =
        serde_json::from_str(whole_float_json).expect("whole float ms parses");
    let string_row: OpenOrderItem = serde_json::from_str(string_json).expect("string ms parses");
    let missing_row: OpenOrderItem =
        serde_json::from_str(missing_json).expect("missing ms decodes");
    let zero_row: OpenOrderItem = serde_json::from_str(zero_json).expect("zero ms decodes");

    assert_eq!(int_row.create_time_ms, 1_700_028_800_123);
    assert_eq!(whole_float_row.create_time_ms, 1_700_028_800_123);
    assert_eq!(string_row.create_time_ms, 1_700_028_800_123);
    assert!(serde_json::from_str::<OpenOrderItem>(fractional_json).is_err());
    assert!(serde_json::from_str::<OpenOrderItem>(null_json).is_err());
    assert!(parse_open_order(&missing_row).is_err());
    assert!(parse_open_order(&zero_row).is_err());
}

#[test]
fn open_order_falls_back_to_official_create_time_seconds() {
    let json = r#"{
        "id":36028834089796976,
        "contract":"BTC_USDT",
        "status":"finished",
        "finish_as":"filled",
        "size":"1",
        "left":"0",
        "price":"62922",
        "fill_price":"62909.4",
        "create_time":1681195121.754,
        "text":"t-xl-gt-o-test",
        "tif":"fok",
        "is_reduce_only":false
    }"#;
    let row: OpenOrderItem = serde_json::from_str(json).expect("live Gate order decodes");
    let order = parse_open_order(&row).expect("create_time fallback projects");

    assert_eq!(order.created_at.timestamp_millis(), 1_681_195_121_754);
    assert!(matches!(order.status, OrderStatus::Filled));
}

#[test]
fn parse_open_order_maps_post_only_side_and_open_status() {
    let post_only = parse_open_order(&open_order("poc", "open", -5, 5)).unwrap();
    assert_eq!(post_only.symbol, "BTC");
    assert!(matches!(post_only.side, OrderSide::Sell));
    assert!(matches!(post_only.order_type, OrderType::PostOnly));
    assert!(matches!(post_only.status, OrderStatus::Open));
}

#[test]
fn parse_open_order_maps_open_left_to_partially_filled() {
    let mut row = open_order("gtc", "open", 5, 2);
    row.fill_price = "29990".into();

    let order = parse_open_order(&row).unwrap();

    assert!(matches!(order.status, OrderStatus::PartiallyFilled));
    assert_eq!(order.quantity, 5.0);
    assert_eq!(order.filled_quantity, 3.0);
    assert_eq!(order.filled_price, 29990.0);
}

#[test]
fn parse_open_order_maps_market_and_finished_status() {
    let mut row = open_order("ioc", "finished", 5, 0);
    row.price = "0".into();
    let market = parse_open_order(&row).unwrap();
    assert!(matches!(market.order_type, OrderType::Market));
    assert!(matches!(market.status, OrderStatus::Filled));
}

#[test]
fn gate_get_order_parses_official_fixture() {
    let json = include_str!("../../fixtures/gate/futures_usdt_get_order_filled.json");
    let row: OpenOrderItem = serde_json::from_str(json).expect("official get-order fixture");
    let order = parse_open_order(&row).expect("get-order row parses");

    assert_eq!(order.order_id, "777");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.client_order_id.as_deref(), Some("t-cid-1"));
    assert_eq!(order.reduce_only, Some(false));
    assert!(matches!(order.side, OrderSide::Buy));
    assert!(matches!(order.order_type, OrderType::Limit));
    assert!(matches!(order.status, OrderStatus::Filled));
    assert_eq!(order.quantity, 1.0);
    assert_eq!(order.filled_quantity, 1.0);
    assert_eq!(order.filled_price, 30000.0);
}

#[test]
fn gate_get_order_ioc_fixture_preserves_partial_fill_as_terminal_cancel() {
    let json = include_str!("../../fixtures/gate/futures_usdt_get_order_ioc.json");
    let row: OpenOrderItem = serde_json::from_str(json).expect("official get-order IOC fixture");
    let order = parse_open_order(&row).expect("get-order IOC row parses");

    assert_eq!(order.order_id, "778");
    assert!(matches!(order.status, OrderStatus::Canceled));
    assert_eq!(order.filled_quantity, 3.0);
    assert_eq!(order.filled_price, 30_000.0);
}

#[test]
fn parse_open_order_maps_positive_price_ioc_to_limit() {
    let order = parse_open_order(&open_order("ioc", "open", 5, 5)).unwrap();

    assert!(matches!(order.order_type, OrderType::Limit));
    assert!(matches!(order.status, OrderStatus::Open));
}

#[test]
fn parse_open_order_maps_fok_to_limit() {
    let order = parse_open_order(&open_order("fok", "finished", 5, 0)).unwrap();

    assert!(matches!(order.order_type, OrderType::Limit));
    assert!(matches!(order.status, OrderStatus::Filled));
}

#[test]
fn parse_open_order_maps_limit_and_finished_left_to_canceled() {
    let canceled = parse_open_order(&open_order("gtc", "finished", 5, 1)).unwrap();
    assert!(matches!(canceled.order_type, OrderType::Limit));
    assert!(matches!(canceled.status, OrderStatus::Canceled));
}

#[test]
fn parse_finished_order_uses_official_finish_as_not_left_heuristic() {
    let mut cancelled = open_order("gtc", "finished", 5, 0);
    cancelled.finish_as = "cancelled".into();
    let mut ioc = open_order("ioc", "finished", 5, 2);
    ioc.finish_as = "ioc".into();

    let cancelled = parse_open_order(&cancelled).unwrap();
    let ioc = parse_open_order(&ioc).unwrap();

    assert!(matches!(cancelled.status, OrderStatus::Canceled));
    assert!(matches!(ioc.status, OrderStatus::Canceled));
    assert_eq!(ioc.filled_quantity, 3.0);
}

#[test]
fn parse_finished_order_rejects_missing_or_inconsistent_finish_as() {
    let mut missing = open_order("gtc", "finished", 5, 0);
    missing.finish_as.clear();
    let mut inconsistent = open_order("gtc", "finished", 5, 1);
    inconsistent.finish_as = "filled".into();

    assert!(matches!(
        parse_open_order(&missing),
        Err(crate::ExchangeError::Parse(message)) if message.contains("unknown finish_as")
    ));
    assert!(matches!(
        parse_open_order(&inconsistent),
        Err(crate::ExchangeError::Parse(message)) if message.contains("reports finish_as filled")
    ));
}

#[test]
fn parse_finished_order_maps_all_documented_non_fill_reasons_to_canceled() {
    for finish_as in [
        "cancelled",
        "liquidated",
        "ioc",
        "auto_deleveraged",
        "reduce_only",
        "position_closed",
        "reduce_out",
        "stp",
    ] {
        let mut row = open_order("gtc", "finished", 5, 1);
        row.finish_as = finish_as.into();
        let parsed = parse_open_order(&row).unwrap();
        assert!(
            matches!(parsed.status, OrderStatus::Canceled),
            "finish_as={finish_as}"
        );
    }
}

#[test]
fn parse_open_order_rejects_bad_price_instead_of_market() {
    let mut row = open_order("ioc", "open", 5, 5);
    row.price = "bad".into();

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid price")
    ));
}

#[test]
fn parse_open_order_rejects_bad_fill_price_instead_of_zero() {
    let mut row = open_order("gtc", "open", 5, 5);
    row.fill_price = "bad".into();

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid fill_price")
    ));
}

#[test]
fn parse_open_order_rejects_unknown_status_instead_of_pending() {
    let row = open_order("gtc", "mystery", 5, 5);

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("unknown status")
    ));
}

#[test]
fn parse_open_order_rejects_unknown_tif_instead_of_limit() {
    let row = open_order("mystery", "open", 5, 5);

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("unknown tif")
    ));
}

#[test]
fn parse_open_order_rejects_zero_size() {
    let row = open_order("gtc", "open", 0, 0);

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("zero size")
    ));
}

#[test]
fn parse_open_order_rejects_left_larger_than_size() {
    let row = open_order("gtc", "open", 5, 6);

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("exceeds size")
    ));
}

#[test]
fn parse_open_orders_propagates_bad_row() {
    let mut bad = open_order("gtc", "open", 5, 5);
    bad.price = "bad".into();
    let rows = vec![open_order("gtc", "open", 5, 5), bad];

    assert!(matches!(
        parse_open_orders(&rows),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid price")
    ));
}

#[test]
fn gate_open_orders_parses_official_fixture() {
    let body = include_str!("../../fixtures/gate/futures_usdt_orders_open.json");
    let rows: Vec<OpenOrderItem> =
        serde_json::from_str(body).expect("official open-orders list decodes");
    let parsed = parse_open_orders(&rows).expect("official open-orders list parses");
    let order = parsed.first().expect("open order present");

    assert_eq!(parsed.len(), 1);
    assert_eq!(order.order_id, "7");
    assert_eq!(order.symbol, "BTC");
    assert!(matches!(order.side, OrderSide::Buy));
    assert!(matches!(order.status, OrderStatus::PartiallyFilled));
    assert_eq!(order.client_order_id.as_deref(), Some("t-cid-1"));
    assert_eq!(order.quantity, 5.0);
    assert_eq!(order.filled_quantity, 2.0);
    assert_eq!(order.filled_price, 29990.0);
}

#[test]
fn strict_account_fixture_rejects_defaulting() {
    let mut account: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate/futures_usdt_account.json"
    ))
    .unwrap();
    account["total"] = serde_json::json!("NaN");
    let account: AccountItem = serde_json::from_value(account).unwrap();
    assert!(parse_balance_response(&account, Some("USDT"), "usdt").is_err());

    let mut missing_margin: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate/futures_usdt_account.json"
    ))
    .unwrap();
    missing_margin
        .as_object_mut()
        .unwrap()
        .remove("position_margin");
    assert!(serde_json::from_value::<AccountItem>(missing_margin).is_err());

    let mut positions: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate/futures_usdt_positions.json"
    ))
    .unwrap();
    positions[0]["maintenance_rate"] = serde_json::json!("NaN");
    let positions: Vec<PositionRow> = serde_json::from_value(positions).unwrap();
    assert!(parse_positions(&positions, None, |_| Ok(0.0001)).is_err());

    let mut unknown_status: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate/futures_usdt_orders_open.json"
    ))
    .unwrap();
    unknown_status[0]["status"] = serde_json::json!("mystery");
    let orders: Vec<OpenOrderItem> = serde_json::from_value(unknown_status).unwrap();
    assert!(parse_open_orders(&orders).is_err());

    let mut invalid_timestamp: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate/futures_usdt_orders_open.json"
    ))
    .unwrap();
    invalid_timestamp[0]["create_time_ms"] = serde_json::json!(0);
    let orders: Vec<OpenOrderItem> = serde_json::from_value(invalid_timestamp).unwrap();
    assert!(parse_open_orders(&orders).is_err());

    let mut missing_reduce_only: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate/futures_usdt_orders_open.json"
    ))
    .unwrap();
    missing_reduce_only[0]
        .as_object_mut()
        .unwrap()
        .remove("is_reduce_only");
    let orders: Vec<OpenOrderItem> = serde_json::from_value(missing_reduce_only).unwrap();
    let parsed = parse_open_orders(&orders).unwrap();
    assert_eq!(parsed[0].reduce_only, None);

    let mut invalid_identity: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/gate/futures_usdt_orders_open.json"
    ))
    .unwrap();
    invalid_identity[0]["id"] = serde_json::json!(0);
    let orders: Vec<OpenOrderItem> = serde_json::from_value(invalid_identity).unwrap();
    assert!(parse_open_orders(&orders).is_err());
}

fn account(
    currency: &str,
    total: &str,
    available: &str,
    position_margin: &str,
    order_margin: &str,
    unrealised_pnl: &str,
) -> AccountItem {
    AccountItem {
        currency: currency.into(),
        total: total.into(),
        available: available.into(),
        position_margin: position_margin.into(),
        order_margin: order_margin.into(),
        unrealised_pnl: unrealised_pnl.into(),
        cross_maintenance_margin: String::new(),
        position_mode: "single".into(),
    }
}

fn position(contract: &str, size: impl Into<f64>) -> PositionRow {
    PositionRow {
        contract: contract.into(),
        size: size.into(),
        entry_price: "30000".into(),
        mark_price: "30100".into(),
        unrealised_pnl: "0.5".into(),
        leverage: "10".into(),
        liq_price: "27000".into(),
        margin: "10".into(),
        mode: "single".into(),
        maintenance_rate: "0.005".into(),
    }
}

fn open_order(
    tif: &str,
    status: &str,
    size: impl Into<f64>,
    left: impl Into<f64>,
) -> OpenOrderItem {
    let size = size.into();
    let left = left.into();
    OpenOrderItem {
        id: 1,
        contract: "BTC_USDT".into(),
        status: status.into(),
        finish_as: if status.eq_ignore_ascii_case("finished") {
            if left == 0.0 {
                "filled"
            } else {
                "cancelled"
            }
        } else {
            ""
        }
        .into(),
        size,
        left,
        price: "30000".into(),
        fill_price: "0".into(),
        create_time_ms: 1,
        create_time: serde_json::Value::Null,
        text: String::new(),
        tif: tif.into(),
        is_reduce_only: Some(false),
    }
}

#[test]
fn parse_open_order_surfaces_client_order_id_and_reduce_only() {
    let mut row = open_order("gtc", "open", 5, 5);
    row.text = "t-cid-1".into();
    row.is_reduce_only = Some(true);
    let order = parse_open_order(&row).unwrap();
    assert_eq!(order.client_order_id.as_deref(), Some("t-cid-1"));
    assert_eq!(order.reduce_only, Some(true));
}

#[test]
fn parse_open_order_collapses_blank_client_order_id() {
    let order = parse_open_order(&open_order("gtc", "open", 5, 5)).unwrap();
    assert_eq!(order.client_order_id, None);
    assert_eq!(order.reduce_only, Some(false));
}

#[test]
fn parse_open_order_with_contract_unit_projects_base_quantity() {
    let order = parse_open_order_with_contract_unit(&open_order("fok", "finished", -1, 0), 0.0001)
        .expect("official contract multiplier normalizes Gate lots");

    assert_eq!(order.quantity, 0.0001);
    assert_eq!(order.filled_quantity, 0.0001);
}

#[test]
fn parse_open_order_with_contract_unit_rejects_missing_metadata() {
    let row = open_order("gtc", "open", 1, 1);

    assert!(parse_open_order_with_contract_unit(&row, 0.0).is_err());
    assert!(parse_open_order_with_contract_unit(&row, f64::NAN).is_err());
}

#[test]
fn parse_positions_maps_mode_and_margin_mode() {
    let mut cross = position("BTC_USDT", 100);
    cross.mode = "dual_long".into();
    cross.leverage = "0".into();
    let mut unknown = position("ETH_USDT", 100);
    unknown.mode = String::new();

    let parsed = parse_positions(&[cross, unknown], None, |_| Ok(0.0001)).unwrap();

    assert_eq!(parsed[0].position_mode.as_deref(), Some("dual_long"));
    assert_eq!(parsed[0].margin_mode.as_deref(), Some("cross"));
    assert_eq!(parsed[1].position_mode, None);
    assert_eq!(parsed[1].margin_mode.as_deref(), Some("isolated"));
}

#[test]
fn parse_positions_rejects_invalid_mode() {
    let mut row = position("BTC_USDT", 100);
    row.mode = "hedged".into();

    let err = parse_positions(&[row], None, |_| Ok(0.0001)).unwrap_err();

    assert!(err.to_string().contains("invalid mode"));
}
