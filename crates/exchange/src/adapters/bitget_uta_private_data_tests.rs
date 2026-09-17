use super::*;
use pretty_assertions::assert_eq;

#[test]
fn parse_balances_picks_unified_total_for_usdt() {
    // Unified Trading Account assets payload sample (modelled on
    // <https://www.bitget.com/api-doc/uta/account/Get-Account-Assets>).
    let payload = UtaAccountAssetsPayload {
        account_equity: "0".into(),
        effective_equity: "0".into(),
        imr: "0".into(),
        mmr: "0".into(),
        margin_ratio: "0".into(),
        position_margin_ratio: "0".into(),
        usdt_unrealized_pl: "1.5".into(),
        assets: vec![
            UtaAccountAssetItem {
                coin: "USDT".into(),
                available: "100".into(),
                locked: "5".into(),
                equity: "105".into(),
            },
            UtaAccountAssetItem {
                coin: "BTC".into(),
                available: "0.5".into(),
                locked: "0.1".into(),
                equity: "0.6".into(),
            },
        ],
    };
    let parsed = parse_account_balances(payload, None).expect("parse balances");
    assert_eq!(parsed.len(), 2);
    assert!((parsed["USDT"].total - 105.0).abs() < 1e-9);
    assert!((parsed["USDT"].available - 100.0).abs() < 1e-9);
    assert!((parsed["USDT"].frozen - 5.0).abs() < 1e-9);
    assert!((parsed["USDT"].unrealized_pnl - 1.5).abs() < 1e-9);
    // Non-USDT row uses the documented equity field directly.
    assert!((parsed["BTC"].total - 0.6).abs() < 1e-9);
}

#[test]
fn parse_balances_filters_currency_case_insensitive() {
    let payload = UtaAccountAssetsPayload {
        account_equity: "0".into(),
        effective_equity: "0".into(),
        imr: "0".into(),
        mmr: "0".into(),
        margin_ratio: "0".into(),
        position_margin_ratio: "0".into(),
        usdt_unrealized_pl: "0".into(),
        assets: vec![
            UtaAccountAssetItem {
                coin: "USDT".into(),
                available: "1".into(),
                locked: "0".into(),
                equity: "1".into(),
            },
            UtaAccountAssetItem {
                coin: "btc".into(),
                available: "2".into(),
                locked: "0".into(),
                equity: "2".into(),
            },
        ],
    };
    let parsed = parse_account_balances(payload, Some("BTC")).expect("parse filtered balances");
    assert_eq!(parsed.len(), 1);
    assert!(parsed.contains_key("btc"));
}

#[test]
fn parse_balances_preserves_negative_account_unrealized_pnl() {
    let payload = UtaAccountAssetsPayload {
        account_equity: "0".into(),
        effective_equity: "0".into(),
        imr: "0".into(),
        mmr: "0".into(),
        margin_ratio: "0".into(),
        position_margin_ratio: "0".into(),
        usdt_unrealized_pl: "-3.5".into(),
        assets: vec![UtaAccountAssetItem {
            coin: "USDT".into(),
            available: "10".into(),
            locked: "0".into(),
            equity: "10".into(),
        }],
    };

    let parsed = parse_account_balances(payload, None).expect("parse balances");

    assert!((parsed["USDT"].unrealized_pnl + 3.5).abs() < 1e-9);
}

#[test]
fn parse_balances_rejects_missing_total() {
    let payload = UtaAccountAssetsPayload {
        account_equity: "0".into(),
        effective_equity: "0".into(),
        imr: "0".into(),
        mmr: "0".into(),
        margin_ratio: "0".into(),
        position_margin_ratio: "0".into(),
        usdt_unrealized_pl: "0".into(),
        assets: vec![UtaAccountAssetItem {
            coin: "ETH".into(),
            available: "1.0".into(),
            locked: "0".into(),
            equity: String::new(),
        }],
    };
    let error = parse_account_balances(payload, None).expect_err("missing total must fail");
    assert!(error.to_string().contains("equity"));
}

#[test]
fn parse_positions_skips_zero_qty_and_pairs_hedge_mode() {
    let rows = vec![
        position("BTCUSDT", "long", "1", "hedge_mode"),
        position("BTCUSDT", "short", "2", "hedge_mode"),
        position("ETHUSDT", "long", "0", "one_way_mode"),
    ];
    let parsed = parse_positions(&rows, None).expect("parse positions");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].paired_with.as_deref(), Some("bitget:BTC:short"));
    assert_eq!(parsed[1].paired_with.as_deref(), Some("bitget:BTC:long"));
}

#[test]
fn parse_positions_accepts_official_v3_total() {
    let raw = r#"{
        "symbol": "BTCUSDT",
        "posSide": "long",
        "holdMode": "one_way_mode",
        "total": "1.5",
        "avgPrice": "30000",
        "markPrice": "31000",
        "unrealisedPnl": "10",
        "leverage": "5",
        "liquidationPrice": "20000",
        "positionBalance": "100",
        "mmr": "0.004"
    }"#;
    let row: UtaPositionRow = serde_json::from_str(raw).expect("parse position");
    let parsed = parse_positions(&[row], None).expect("parse position alias");
    assert_eq!(parsed.len(), 1);
    assert!((parsed[0].quantity - 1.5).abs() < 1e-9);
    assert!((parsed[0].leverage - 5.0).abs() < 1e-9);
    assert!((parsed[0].maintenance_margin_ratio - 0.004).abs() < 1e-9);
}

#[test]
fn bitget_current_position_parses_official_fixture() {
    let body = include_str!("../../fixtures/bitget/uta_current_position_btcusdt.json");
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<
        UtaListPayload<UtaPositionRow>,
    > = serde_json::from_str(body).expect("official current-position envelope decodes");
    let payload = resp
        .into_result("current position")
        .expect("success code yields current-position list");
    let parsed = parse_positions(&payload.list, Some("BTCUSDT")).expect("positions parse");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[1].side, "short");
    assert_eq!(parsed[0].paired_with.as_deref(), Some("bitget:BTC:short"));
    assert_eq!(parsed[1].paired_with.as_deref(), Some("bitget:BTC:long"));
    assert!((parsed[0].maintenance_margin_ratio - 0.004).abs() < 1e-12);
}

#[test]
fn bitget_empty_current_position_accepts_live_null_list() {
    let body = include_str!("../../fixtures/bitget/uta_current_position_empty.json");
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<
        UtaListPayload<UtaPositionRow>,
    > = serde_json::from_str(body).expect("live empty current-position envelope decodes");
    let payload = resp
        .into_result("current position")
        .expect("success code yields an empty current-position list");

    assert!(payload.list.is_empty());
    assert!(parse_positions(&payload.list, None)
        .expect("empty positions parse")
        .is_empty());
}

#[test]
fn parse_open_order_uses_time_in_force_for_post_only_and_status() {
    let order =
        parse_open_order(open_order("post_only", "limit", "sell", "live")).expect("parse order");
    assert_eq!(order.symbol, "BTC");
    assert!(matches!(order.side, OrderSide::Sell));
    assert!(matches!(order.order_type, OrderType::PostOnly));
    assert!(matches!(order.status, OrderStatus::Open));
}

#[test]
fn parse_open_order_accepts_official_cancelled_status() {
    let order = parse_open_order(open_order("gtc", "limit", "sell", "cancelled"))
        .expect("parse official cancelled order");

    assert!(matches!(order.status, OrderStatus::Canceled));
}

#[test]
fn parse_open_order_surfaces_client_order_id_and_reduce_only() {
    let mut row = open_order("gtc", "limit", "buy", "live");
    row.client_oid = "bitget-cli-1".into();
    row.reduce_only = "YES".into();

    let order = parse_open_order(row).expect("parse order");

    assert_eq!(order.client_order_id.as_deref(), Some("bitget-cli-1"));
    assert_eq!(order.reduce_only, Some(true));
}

#[test]
fn parse_open_order_maps_reduce_only_no_and_rejects_blank() {
    let mut row = open_order("gtc", "limit", "buy", "live");
    row.reduce_only = "NO".into();
    assert_eq!(
        parse_open_order(row).expect("parse").reduce_only,
        Some(false)
    );

    let mut blank = open_order("gtc", "limit", "buy", "live");
    blank.reduce_only.clear();
    let error = parse_open_order(blank).expect_err("missing reduceOnly must fail");
    assert!(error.to_string().contains("reduceOnly"));
}

#[test]
fn parse_open_order_rejects_unknown_reduce_only() {
    let mut row = open_order("gtc", "limit", "buy", "live");
    row.reduce_only = "maybe".into();

    let error = parse_open_order(row).expect_err("unknown reduceOnly must fail");

    assert!(error.to_string().contains("reduceOnly"));
}

#[test]
fn parse_open_order_rejects_legacy_size_and_force_aliases() {
    let raw = r#"{
        "symbol": "BTCUSDT",
        "orderId": "1",
        "clientOid": "c",
        "status": "live",
        "orderType": "limit",
        "force": "gtc",
        "side": "buy",
        "price": "30000",
        "size": "0.5",
        "baseVolume": "0",
        "priceAvg": "0",
        "cTime": "1700000000000"
    }"#;
    assert!(serde_json::from_str::<UtaOrderRow>(raw).is_err());
}

#[test]
fn list_payload_requires_list_field() {
    let raw = r#"{}"#;
    assert!(serde_json::from_str::<UtaListPayload<UtaOrderRow>>(raw).is_err());
}

#[test]
fn parse_balances_rejects_invalid_available() {
    let payload = UtaAccountAssetsPayload {
        account_equity: "0".into(),
        effective_equity: "0".into(),
        imr: "0".into(),
        mmr: "0".into(),
        margin_ratio: "0".into(),
        position_margin_ratio: "0".into(),
        usdt_unrealized_pl: "0".into(),
        assets: vec![UtaAccountAssetItem {
            coin: "USDT".into(),
            available: "bad".into(),
            locked: "0".into(),
            equity: "1".into(),
        }],
    };

    let error = parse_account_balances(payload, None).expect_err("invalid available must fail");

    assert!(error.to_string().contains("available"));
    assert!(error.to_string().contains("bad"));
}

#[test]
fn parse_positions_rejects_invalid_mark_price() {
    let mut row = position("BTCUSDT", "long", "1", "one_way_mode");
    row.mark_price = "bad".into();

    let error = parse_positions(&[row], None).expect_err("invalid mark must fail");

    assert!(error.to_string().contains("markPrice"));
    assert!(error.to_string().contains("bad"));
}

#[test]
fn parse_open_order_rejects_invalid_created_time() {
    let mut row = open_order("gtc", "limit", "buy", "live");
    row.created_time = "not-ms".into();

    let error = parse_open_order(row).expect_err("invalid createdTime must fail");

    assert!(error.to_string().contains("createdTime"));
    assert!(error.to_string().contains("not-ms"));
}

#[test]
fn parse_open_order_rejects_unknown_side_type_status_or_tif() {
    let side = open_order("gtc", "limit", "hold", "live");
    let order_type = open_order("gtc", "iceberg", "buy", "live");
    let status = open_order("gtc", "limit", "buy", "waiting");
    let tif = open_order("maker_only", "limit", "buy", "live");

    assert!(parse_open_order(side)
        .expect_err("unknown side must fail")
        .to_string()
        .contains("side"));
    assert!(parse_open_order(order_type)
        .expect_err("unknown type must fail")
        .to_string()
        .contains("orderType"));
    assert!(parse_open_order(status)
        .expect_err("unknown status must fail")
        .to_string()
        .contains("orderStatus"));
    assert!(parse_open_order(tif)
        .expect_err("unknown tif must fail")
        .to_string()
        .contains("timeInForce"));
}

#[test]
fn parse_positions_rejects_unknown_side_or_missing_margin() {
    let side = position("BTCUSDT", "both", "1", "one_way_mode");
    let mut margin = position("BTCUSDT", "long", "1", "one_way_mode");
    margin.margin_size.clear();

    assert!(parse_positions(&[side], None)
        .expect_err("unknown position side must fail")
        .to_string()
        .contains("posSide"));
    assert!(parse_positions(&[margin], None)
        .expect_err("missing margin must fail")
        .to_string()
        .contains("positionBalance"));
}

#[test]
fn parse_positions_maps_official_mmr() {
    let parsed = parse_positions(&[position("BTCUSDT", "long", "1", "one_way_mode")], None)
        .expect("parse position");
    assert_eq!(parsed.len(), 1);
    assert!((parsed[0].maintenance_margin_ratio - 0.004).abs() < 1e-9);
}

#[test]
fn parse_positions_rejects_missing_mmr() {
    let mut row = position("BTCUSDT", "long", "1", "one_way_mode");
    row.maintenance_margin_rate.clear();
    let error = parse_positions(&[row], None).expect_err("missing mmr must fail");
    assert!(error.to_string().contains("mmr"));
}

#[test]
fn parse_positions_rejects_negative_mmr() {
    let mut row = position("BTCUSDT", "long", "1", "one_way_mode");
    row.maintenance_margin_rate = "-0.01".into();
    let error = parse_positions(&[row], None).expect_err("negative mmr must fail");
    assert!(error.to_string().contains("mmr"));
}

#[test]
fn strict_account_fixture_rejects_defaulting() {
    let mut missing_equity: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_account_assets.json"
    ))
    .expect("account fixture json");
    missing_equity["data"]["assets"][0]
        .as_object_mut()
        .expect("asset object")
        .remove("equity");
    assert!(serde_json::from_value::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaAccountAssetsPayload>,
    >(missing_equity)
    .is_err());

    let mut non_finite: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_account_assets.json"
    ))
    .expect("account fixture json");
    non_finite["data"]["assets"][0]["available"] = serde_json::json!("NaN");
    let payload = serde_json::from_value::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaAccountAssetsPayload>,
    >(non_finite)
    .expect("account envelope")
    .into_result("account assets")
    .expect("account payload");
    assert!(parse_account_balances(payload, None).is_err());

    let mut unknown_hold_mode: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_current_position_btcusdt.json"
    ))
    .expect("position fixture json");
    unknown_hold_mode["data"]["list"][0]["holdMode"] = serde_json::json!("legacy_hedge");
    let positions = serde_json::from_value::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaListPayload<UtaPositionRow>>,
    >(unknown_hold_mode)
    .expect("position envelope")
    .into_result("current position")
    .expect("position payload");
    assert!(parse_positions(&positions.list, None).is_err());

    let mut missing_timestamp: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_unfilled_orders_open.json"
    ))
    .expect("order fixture json");
    missing_timestamp["data"]["list"][0]
        .as_object_mut()
        .expect("order object")
        .remove("createdTime");
    assert!(serde_json::from_value::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaListPayload<UtaOrderRow>>,
    >(missing_timestamp)
    .is_err());

    let mut invalid_timestamp: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_unfilled_orders_open.json"
    ))
    .expect("order fixture json");
    invalid_timestamp["data"]["list"][0]["createdTime"] = serde_json::json!("0");
    let orders = serde_json::from_value::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaListPayload<UtaOrderRow>>,
    >(invalid_timestamp)
    .expect("order envelope")
    .into_result("unfilled orders")
    .expect("order payload");
    assert!(parse_open_orders(orders.list).is_err());

    let mut legacy_qty: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_unfilled_orders_open.json"
    ))
    .expect("order fixture json");
    let row = legacy_qty["data"]["list"][0]
        .as_object_mut()
        .expect("order object");
    let qty = row.remove("qty").expect("qty field");
    row.insert("size".to_owned(), qty);
    assert!(serde_json::from_value::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaListPayload<UtaOrderRow>>,
    >(legacy_qty)
    .is_err());
}

fn position(symbol: &str, hold_side: &str, qty: &str, pos_mode: &str) -> UtaPositionRow {
    UtaPositionRow {
        symbol: symbol.into(),
        hold_side: hold_side.into(),
        hold_mode: pos_mode.into(),
        qty: qty.into(),
        avg_price: "30000".into(),
        mark_price: "31000".into(),
        unrealized_pl: "10".into(),
        leverage: "2".into(),
        liquidation_price: "15000".into(),
        margin_size: "100".into(),
        maintenance_margin_rate: "0.004".into(),
    }
}

fn open_order(force: &str, order_type: &str, side: &str, status: &str) -> UtaOrderRow {
    UtaOrderRow {
        symbol: "BTCUSDT".into(),
        order_id: "123".into(),
        order_status: status.into(),
        order_type: order_type.into(),
        time_in_force: force.into(),
        side: side.into(),
        price: "30000".into(),
        qty: "1".into(),
        cum_exec_qty: "0".into(),
        avg_price: "0".into(),
        created_time: "1700000000000".into(),
        client_oid: String::new(),
        reduce_only: "NO".into(),
        fee_detail: Vec::new(),
        exec_type: String::new(),
        cancel_reason: String::new(),
    }
}

// PR-CX: official Bitget UTA V3 REST response-envelope fixtures, exercised
// end-to-end through BitgetObjectResponse::into_option / into_result +
// parse_open_order / ack_from_row. The struct-level tests above build
// UtaOrderRow / payloads directly and so skip the official `{code,msg,data}`
// envelope, the non-`00000` code fail-closed path, and the success-code +
// null-data "order not found" path that get_order relies on (into_option ->
// None) and which must stay distinct from a fabricated order.
fn bitget_envelope(code: &str, data: &str) -> String {
    format!(r#"{{"code":"{code}","msg":"success","data":{data}}}"#)
}

fn parse_order_fixture(fixture: &str) -> shared_types::OrderInfo {
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<UtaOrderRow> =
        serde_json::from_str(fixture).expect("envelope decodes");
    let row = resp
        .into_option("get order")
        .expect("success code yields data")
        .expect("order present");
    parse_open_order(row).expect("order parses")
}

fn assert_official_filled_order_identity(parsed: &shared_types::OrderInfo) {
    assert_eq!(parsed.order_id, "111111111111111111");
    assert_eq!(parsed.symbol, "ETH");
    assert!(matches!(parsed.status, OrderStatus::Filled));
    assert!(matches!(parsed.side, OrderSide::Buy));
    assert!(matches!(parsed.order_type, OrderType::Market));
    assert_eq!(
        parsed.client_order_id.as_deref(),
        Some("111111111111111111")
    );
    assert_eq!(parsed.reduce_only, Some(false));
}

fn assert_official_filled_order_values(parsed: &shared_types::OrderInfo) {
    assert_eq!(parsed.filled_quantity, 0.0372);
    assert_eq!(parsed.filled_price, 2684.23);
    assert_eq!(parsed.price, 0.0);
    assert!((parsed.fees - 0.00000744).abs() < 1e-12);
    assert_eq!(parsed.execution_style, None);
}

#[test]
fn bitget_get_order_parses_official_fixture() {
    let parsed = parse_order_fixture(include_str!(
        "../../fixtures/bitget/uta_order_info_filled.json"
    ));
    assert_official_filled_order_identity(&parsed);
    assert_official_filled_order_values(&parsed);
}

#[test]
fn bitget_get_order_preserves_exec_type_and_cancel_reason_context() {
    let mut fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bitget/uta_order_info_filled.json"
    ))
    .expect("fixture json");
    fixture["data"]["execType"] = serde_json::json!("liquidation");
    fixture["data"]["cancelReason"] = serde_json::json!("risk_control");
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<UtaOrderRow> =
        serde_json::from_value(fixture).expect("envelope decodes");
    let row = resp
        .into_option("get order")
        .expect("success code yields data")
        .expect("order present");
    let parsed = parse_open_order(row).expect("order parses");

    assert_eq!(
        parsed.execution_style.as_deref(),
        Some("liquidation; cancel_reason=risk_control")
    );
}

#[test]
fn bitget_account_assets_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/bitget/uta_account_assets.json");
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<UtaAccountAssetsPayload> =
        serde_json::from_str(fixture).expect("envelope decodes");
    let payload = resp
        .into_result("account assets")
        .expect("success code yields account assets");
    let parsed = parse_account_balances(payload, None).expect("account assets parse");
    let usdt = parsed.get("USDT").expect("USDT asset present");
    let bgb = parsed.get("BGB").expect("BGB asset present");

    assert_eq!(parsed.len(), 2);
    assert!((usdt.total - 6.19300826).abs() < 1e-9);
    assert!((usdt.available - 6.19300826).abs() < 1e-9);
    assert_eq!(usdt.frozen, 0.0);
    assert_eq!(usdt.unrealized_pnl, 0.0);
    assert!((bgb.total - 1.15582129).abs() < 1e-9);
}

#[test]
fn bitget_unfilled_orders_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/bitget/uta_unfilled_orders_open.json");
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<UtaListPayload<UtaOrderRow>> =
        serde_json::from_str(fixture).expect("envelope decodes");
    let payload = resp
        .into_result("unfilled orders")
        .expect("success code yields order list");
    let parsed = parse_open_orders(payload.list).expect("open orders parse");
    let order = parsed.first().expect("open order present");

    assert_eq!(parsed.len(), 1);
    assert_eq!(order.order_id, "111111111111111111");
    assert_eq!(order.symbol, "BTC");
    assert!(matches!(order.side, OrderSide::Buy));
    assert!(matches!(order.order_type, OrderType::Limit));
    assert!(matches!(order.status, OrderStatus::Open));
    assert_eq!(order.client_order_id.as_deref(), Some("111111111111111111"));
    assert_eq!(order.reduce_only, Some(false));
    assert_eq!(order.quantity, 0.01);
    assert_eq!(order.price, 45_000.0);
    assert_eq!(order.filled_quantity, 0.0);
}

#[test]
fn get_order_envelope_rejects_non_success_code() {
    let text = bitget_envelope("40109", "null");
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<UtaOrderRow> =
        serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_option("get order").is_err());
}

#[test]
fn get_order_envelope_null_data_is_not_found() {
    let text = bitget_envelope("00000", "null");
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<UtaOrderRow> =
        serde_json::from_str(&text).expect("envelope decodes");
    let found = resp.into_option("get order").expect("success code");
    assert!(found.is_none());
}

#[test]
fn cancel_order_official_envelope_builds_ack() {
    let text = bitget_envelope(
        "00000",
        r#"{"orderId":"1779020","clientOid":"bitget-cli-7"}"#,
    );
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<
        crate::adapters::bitget_uta_trade_data::UtaOrderAckRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    let row = resp
        .into_result("cancel order")
        .expect("success code yields ack");
    let ack = crate::adapters::bitget_uta_trade_data::ack_from_row(
        "internal-1".to_owned(),
        "public-1".to_owned(),
        row,
        shared_types::LiveOrderState::CancelRequested,
        None,
    );
    assert_eq!(ack.exchange_order_id.as_deref(), Some("1779020"));
    assert!(matches!(
        ack.state,
        shared_types::LiveOrderState::CancelRequested
    ));
}

#[test]
fn cancel_order_envelope_rejects_non_success_code() {
    let text = bitget_envelope("40109", r#"{"orderId":"","clientOid":""}"#);
    let resp: crate::adapters::bitget_response::BitgetObjectResponse<
        crate::adapters::bitget_uta_trade_data::UtaOrderAckRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_result("cancel order").is_err());
}
