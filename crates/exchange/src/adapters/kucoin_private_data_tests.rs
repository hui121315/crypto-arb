use super::*;
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, MarginMode, OrderSource, TimeInForce};

#[test]
fn parse_position_mode_maps_official_values() {
    let one_way = parse_position_mode(&PositionModeRow { position_mode: 0 }).unwrap();
    let hedge = parse_position_mode(&PositionModeRow { position_mode: 1 }).unwrap();

    assert_eq!(one_way, KucoinPositionMode::OneWay);
    assert_eq!(one_way.as_str(), "one_way");
    assert_eq!(hedge, KucoinPositionMode::Hedge);
    assert_eq!(hedge.as_str(), "hedge");
}

#[test]
fn kucoin_position_mode_parses_official_fixture() {
    let body = include_str!("../../fixtures/kucoin/position_mode_hedge.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<PositionModeRow> =
        serde_json::from_str(body).expect("official position-mode envelope decodes");
    let row = resp
        .into_data("position/getPositionMode")
        .expect("success code yields position-mode row");
    let mode = parse_position_mode(&row).expect("official position-mode row parses");

    assert_eq!(mode, KucoinPositionMode::Hedge);
    assert_eq!(mode.code(), 1);
}

#[test]
fn parse_position_mode_rejects_unknown_value() {
    let err = parse_position_mode(&PositionModeRow { position_mode: 9 }).unwrap_err();
    assert!(matches!(err, crate::ExchangeError::Api { code, .. } if code == "validation"));
}

#[test]
fn position_mode_side_mapping_is_fail_closed() {
    let buy = intent(OrderSide::Buy, false);
    let sell = intent(OrderSide::Sell, false);
    let reduce = intent(OrderSide::Sell, true);

    assert_eq!(
        KucoinPositionMode::OneWay
            .position_side_for_intent(&buy)
            .unwrap(),
        "BOTH"
    );
    assert_eq!(
        KucoinPositionMode::Hedge
            .position_side_for_intent(&buy)
            .unwrap(),
        "LONG"
    );
    assert_eq!(
        KucoinPositionMode::Hedge
            .position_side_for_intent(&sell)
            .unwrap(),
        "SHORT"
    );
    assert!(KucoinPositionMode::Hedge
        .position_side_for_intent(&reduce)
        .is_err());
}

#[test]
fn position_mode_row_rejects_missing_or_non_integer_field() {
    assert!(serde_json::from_value::<PositionModeRow>(serde_json::json!({})).is_err());
    assert!(
        serde_json::from_value::<PositionModeRow>(serde_json::json!({"positionMode": "1"}))
            .is_err()
    );
}

#[test]
fn parse_balance_response_uses_available_margin_for_buying_power() {
    let account = AccountOverview {
        account_equity: Some(num(100.0)),
        unrealised_pnl: Some(num(1.0)),
        available_balance: Some(num(80.0)),
        margin_balance: Some(num(99.0)),
        available_margin: Some(num(70.0)),
        risk_ratio: Some(num(0.25)),
        position_margin: Some(num(10.0)),
        order_margin: Some(num(5.0)),
        frozen_funds: Some(num(2.0)),
        max_withdraw_amount: None,
        currency: "USDT".into(),
    };
    let parsed = parse_balance_response(&account, "USDT").expect("parse balance");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed["USDT"].total, 100.0);
    assert_eq!(parsed["USDT"].available, 70.0);
    assert_eq!(parsed["USDT"].frozen, 17.0);
    assert_eq!(parsed["USDT"].unrealized_pnl, 1.0);
}

#[test]
fn parse_account_read_projects_futures_equity_summary() {
    let account: AccountOverview = serde_json::from_value(serde_json::json!({
        "accountEquity": "100",
        "unrealisedPNL": "1",
        "availableBalance": "80",
        "marginBalance": "99",
        "availableMargin": "70",
        "riskRatio": "0.25",
        "positionMargin": "10",
        "orderMargin": "5",
        "frozenFunds": "2",
        "maxWithdrawAmount": "65",
        "currency": "USDT"
    }))
    .expect("deserialize account");

    let read = parse_account_read(&account, "USDT", 1_700_000_000_000).expect("parse account read");
    let balance = read.balances.first().expect("balance row present");
    let summary = read.summaries.first().expect("account summary present");

    assert_eq!(balance.currency, "USDT");
    assert_eq!(balance.total, 100.0);
    assert_eq!(summary.total_equity_usd, 100.0);
    assert_eq!(summary.total_available_balance_usd, 70.0);
    assert_eq!(summary.total_initial_margin_usd, 15.0);
    assert_eq!(summary.total_maintenance_margin_usd, 25.0);
    assert_eq!(summary.withdrawable_balance_usd, Some(65.0));
    assert_eq!(summary.account_mm_rate, 0.25);
}

#[test]
fn parse_balance_response_rejects_missing_or_mismatched_currency() {
    let mut account: AccountOverview = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/account_overview_usdt.json"
    ))
    .and_then(|response: serde_json::Value| serde_json::from_value(response["data"].clone()))
    .expect("official account overview data");
    account.currency.clear();
    assert!(matches!(
        parse_balance_response(&account, "USDT"),
        Err(crate::ExchangeError::Parse(message)) if message.contains("missing currency")
    ));

    account.currency = "USDC".into();
    assert!(matches!(
        parse_balance_response(&account, "USDT"),
        Err(crate::ExchangeError::Parse(message)) if message.contains("currency mismatch")
    ));
}

#[test]
fn parse_balance_response_rejects_missing_required_number() {
    let account: AccountOverview = serde_json::from_value(serde_json::json!({
        "unrealisedPNL": 1.0,
        "availableBalance": 80.0,
        "marginBalance": 99.0,
        "availableMargin": 70.0,
        "riskRatio": 0.25,
        "positionMargin": 10.0,
        "orderMargin": 5.0,
        "frozenFunds": 2.0,
        "currency": "USDT"
    }))
    .expect("deserialize account");

    assert!(matches!(
        parse_balance_response(&account, "USDT"),
        Err(crate::ExchangeError::Parse(message)) if message.contains("missing accountEquity")
    ));
}

#[test]
fn parse_balance_response_accepts_numeric_strings() {
    let account: AccountOverview = serde_json::from_value(serde_json::json!({
        "accountEquity": "100.5",
        "unrealisedPNL": "-1.25",
        "availableBalance": "90.0",
        "marginBalance": "99.25",
        "availableMargin": "85.0",
        "riskRatio": "0.125",
        "positionMargin": "7.5",
        "orderMargin": "1.5",
        "frozenFunds": "0",
        "currency": "USDT"
    }))
    .expect("deserialize account");

    let parsed = parse_balance_response(&account, "USDT").expect("parse string balance");
    assert_eq!(parsed["USDT"].total, 100.5);
    assert_eq!(parsed["USDT"].available, 85.0);
    assert_eq!(parsed["USDT"].frozen, 9.0);
    assert_eq!(parsed["USDT"].unrealized_pnl, -1.25);
}

#[test]
fn parse_balance_response_rejects_missing_available_margin() {
    let account: AccountOverview = serde_json::from_value(serde_json::json!({
        "accountEquity": 100.0,
        "unrealisedPNL": 1.0,
        "availableBalance": 80.0,
        "marginBalance": 99.0,
        "riskRatio": 0.25,
        "positionMargin": 10.0,
        "orderMargin": 5.0,
        "frozenFunds": 2.0,
        "currency": "USDT"
    }))
    .expect("deserialize account");

    assert!(matches!(
        parse_balance_response(&account, "USDT"),
        Err(crate::ExchangeError::Parse(message)) if message.contains("missing availableMargin")
    ));
}

#[test]
fn parse_balance_response_rejects_negative_risk_ratio() {
    let account: AccountOverview = serde_json::from_value(serde_json::json!({
        "accountEquity": 100.0,
        "unrealisedPNL": 1.0,
        "availableBalance": 80.0,
        "marginBalance": 99.0,
        "availableMargin": 70.0,
        "riskRatio": -0.01,
        "positionMargin": 10.0,
        "orderMargin": 5.0,
        "frozenFunds": 2.0,
        "currency": "USDT"
    }))
    .expect("deserialize account");

    assert!(matches!(
        parse_balance_response(&account, "USDT"),
        Err(crate::ExchangeError::Parse(message)) if message.contains("negative riskRatio")
    ));
}

#[test]
fn kucoin_account_overview_parses_official_fixture() {
    let body = include_str!("../../fixtures/kucoin/account_overview_usdt.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<AccountOverview> =
        serde_json::from_str(body).expect("official account-overview envelope decodes");
    let account = resp
        .into_data("account-overview")
        .expect("success code yields account overview");
    let parsed = parse_balance_response(&account, "USDT").expect("account overview parses");
    let usdt = parsed.get("USDT").expect("USDT account present");

    assert_eq!(parsed.len(), 1);
    assert!((usdt.total - 198.733127406).abs() < 1e-12);
    assert!((usdt.available - 198.733127406).abs() < 1e-12);
    assert_eq!(usdt.frozen, 0.0);
    assert_eq!(usdt.unrealized_pnl, 0.0);
}

#[test]
fn kucoin_uta_account_overview_parses_official_fixture() {
    let body = include_str!("../../fixtures/kucoin/uta_account_overview.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<UtaAccountOverview> =
        serde_json::from_str(body).expect("official UTA account-overview envelope decodes");
    let account = resp
        .into_data("uta account-overview")
        .expect("success code yields UTA account overview");
    let parsed = parse_uta_account_overview(&account).expect("UTA overview parses");

    assert_eq!(parsed.account_type, "UNIFIED");
    assert!((parsed.risk_ratio - 0.0085517383).abs() < 1e-12);
    assert!((parsed.equity - 49.9358942670).abs() < 1e-12);
    assert_eq!(parsed.liability, 0.0);
    assert!((parsed.available_margin - 46.0677901003).abs() < 1e-12);
    assert!((parsed.adjusted_equity - 49.9358942670).abs() < 1e-12);
    assert!((parsed.initial_margin - 3.8681041666).abs() < 1e-12);
    assert!((parsed.maintenance_margin - 0.3713380000).abs() < 1e-12);
}

#[test]
fn kucoin_uta_account_overview_rejects_non_unified_scope() {
    let account: UtaAccountOverview = serde_json::from_value(serde_json::json!({
        "accountType": "CONTRACT",
        "riskRatio": "0.01",
        "equity": "10",
        "liability": "0",
        "availableMargin": "9",
        "adjustedEquity": "10",
        "im": "1",
        "mm": "0.1"
    }))
    .expect("deserialize UTA overview");

    assert!(matches!(
        parse_uta_account_overview(&account),
        Err(crate::ExchangeError::Parse(message))
            if message.contains("expected UNIFIED accountType")
    ));
}

#[test]
fn kucoin_uta_account_overview_rejects_missing_available_margin() {
    let account: UtaAccountOverview = serde_json::from_value(serde_json::json!({
        "accountType": "UNIFIED",
        "riskRatio": "0.01",
        "equity": "10",
        "liability": "0",
        "adjustedEquity": "10",
        "im": "1",
        "mm": "0.1"
    }))
    .expect("deserialize UTA overview");

    assert!(matches!(
        parse_uta_account_overview(&account),
        Err(crate::ExchangeError::Parse(message)) if message.contains("missing availableMargin")
    ));
}

#[test]
fn kucoin_uta_currency_assets_parse_wallet_scope_fixture() {
    let body = include_str!("../../fixtures/kucoin/uta_account_currency_assets.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<UtaCurrencyAssets> =
        serde_json::from_str(body).expect("official UTA currency-assets envelope decodes");
    let payload = resp
        .into_data("uta currency-assets")
        .expect("success code yields UTA currency assets");
    let parsed = parse_uta_currency_assets(&payload).expect("UTA currency assets parse");
    let usdt = parsed.first().expect("USDT asset present");

    assert_eq!(parsed.len(), 1);
    assert_eq!(usdt.currency, "USDT");
    assert!((usdt.equity - 3054.8568208133).abs() < 1e-12);
    assert_eq!(usdt.hold, 0.0);
    assert!((usdt.balance - 3052.9434208133).abs() < 1e-12);
    assert!((usdt.available_wallet_balance - 3054.8568208133).abs() < 1e-12);
    assert_eq!(usdt.liability, 0.0);
}

#[test]
fn kucoin_uta_currency_assets_rejects_empty_currencies() {
    let payload: UtaCurrencyAssets = serde_json::from_value(serde_json::json!({
        "accountType": "UNIFIED",
        "accounts": [{"currencies": []}]
    }))
    .expect("deserialize UTA currency assets");

    assert!(matches!(
        parse_uta_currency_assets(&payload),
        Err(crate::ExchangeError::Parse(message)) if message.contains("missing currencies")
    ));
}

#[test]
fn parse_positions_filters_open_target_and_signed_side() {
    let rows = vec![
        position("XBTUSDTM", 3.0, true),
        position("XBTUSDTM", -2.0, true),
        position("ETHUSDTM", 4.0, true),
        position("XBTUSDTM", 0.0, true),
        position("XBTUSDTM", 1.0, false),
    ];
    let parsed = parse_positions(&rows, Some("XBTUSDTM")).expect("parse positions");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].symbol, "BTC");
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[1].side, "short");
}

#[test]
fn kucoin_positions_parse_official_fixture() {
    let body = include_str!("../../fixtures/kucoin/positions_open.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<Vec<PositionRow>> =
        serde_json::from_str(body).expect("official positions envelope decodes");
    let rows = resp
        .into_data("positions")
        .expect("success code yields positions");
    let parsed = parse_positions(&rows, None).expect("positions parse");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].symbol, "BTC");
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[1].symbol, "ETH");
    assert_eq!(parsed[1].side, "short");
    assert!((parsed[0].maintenance_margin_ratio - 0.012).abs() < 1e-12);
}

#[test]
fn parse_positions_projects_margin_mode_evidence() {
    let mut isolated = position("XBTUSDTM", 2.0, true);
    isolated.margin_mode = Some("ISOLATED".to_owned());
    let mut cross = position("ETHUSDTM", 2.0, true);
    cross.margin_mode = Some("CROSS".to_owned());

    let parsed = parse_positions(&[isolated, cross], None).expect("parse positions");

    assert_eq!(parsed[0].margin_mode.as_deref(), Some("isolated"));
    assert_eq!(parsed[1].margin_mode.as_deref(), Some("cross"));
}

#[test]
fn parse_positions_rejects_unknown_margin_mode() {
    let mut row = position("XBTUSDTM", 2.0, true);
    row.margin_mode = Some("LEGACY".to_owned());

    assert!(matches!(
        parse_positions(&[row], None),
        Err(crate::ExchangeError::Parse(message))
            if message.contains("unknown marginMode")
    ));
}

#[test]
fn parse_positions_rejects_missing_open_current_qty() {
    let row: PositionRow = serde_json::from_value(serde_json::json!({
        "symbol": "XBTUSDTM",
        "avgEntryPrice": 30000.0,
        "markPrice": 30100.0,
        "unrealisedPnl": 0.5,
        "leverage": 10.0,
        "liquidationPrice": 27000.0,
        "posMargin": 10.0,
        "isOpen": true
    }))
    .expect("deserialize position");

    assert!(matches!(
        parse_positions(&[row], None),
        Err(crate::ExchangeError::Parse(message)) if message.contains("missing currentQty")
    ));
}

#[test]
fn parse_positions_rejects_bad_open_mark_price() {
    let row: PositionRow = serde_json::from_value(serde_json::json!({
        "symbol": "XBTUSDTM",
        "currentQty": 1.0,
        "avgEntryPrice": 30000.0,
        "markPrice": "bad",
        "unrealisedPnl": 0.5,
        "leverage": 10.0,
        "liquidationPrice": 27000.0,
        "posMargin": 10.0,
        "isOpen": true
    }))
    .expect("deserialize position");

    assert!(matches!(
        parse_positions(&[row], None),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid markPrice")
    ));
}

#[test]
fn parse_open_order_maps_post_only_limit_order() {
    let order = parse_open_order(&open_order(true, "limit", "buy", "open", 0.0, "0")).unwrap();
    assert!(matches!(order.order_type, OrderType::PostOnly));
    assert!(matches!(order.status, OrderStatus::Open));
}

#[test]
fn parse_open_order_rejects_post_only_market_order() {
    let order = open_order(true, "market", "buy", "open", 0.0, "0");

    assert!(matches!(
        parse_open_order(&order),
        Err(crate::ExchangeError::Parse(message)) if message.contains("postOnly market")
    ));
}

#[test]
fn parse_open_order_maps_partial_and_average_price() {
    let row = open_order(false, "limit", "sell", "open", 2.0, "60");
    let order = parse_order_with_fills(&row, &[fill("1", 2.0, 30.0, 0.02)]).unwrap();
    assert!(matches!(order.side, OrderSide::Sell));
    assert!(matches!(order.order_type, OrderType::Limit));
    assert!(matches!(order.status, OrderStatus::PartiallyFilled));
    assert_eq!(order.filled_price, 30.0);
    assert_eq!(order.fees, 0.02);
}

#[test]
fn parse_open_order_surfaces_client_order_id_and_reduce_only() {
    let mut row = open_order(false, "limit", "buy", "open", 0.0, "0");
    row.client_oid = "kucoin-cli-1".into();
    row.reduce_only = true;
    let order = parse_open_order(&row).unwrap();
    assert_eq!(order.client_order_id.as_deref(), Some("kucoin-cli-1"));
    assert_eq!(order.reduce_only, Some(true));
}

#[test]
fn parse_open_order_collapses_blank_client_order_id_but_preserves_reduce_only() {
    let order = parse_open_order(&open_order(false, "limit", "buy", "open", 0.0, "0")).unwrap();
    assert_eq!(order.client_order_id, None);
    assert_eq!(order.reduce_only, Some(false));
}

#[test]
fn parse_open_order_maps_market_and_done_status() {
    let row = open_order(false, "market", "buy", "done", 1.0, "31");
    let order = parse_order_with_fills(&row, &[fill("1", 1.0, 31.0, 0.01)]).unwrap();
    assert!(matches!(order.order_type, OrderType::Market));
    assert!(matches!(order.status, OrderStatus::Filled));
}

#[test]
fn parse_open_order_uses_cancel_exist_for_done_cancel() {
    let mut row = open_order(false, "limit", "buy", "done", 0.0, "0");
    row.cancel_exist = true;
    let order = parse_open_order(&row).unwrap();

    assert!(matches!(order.status, OrderStatus::Canceled));
}

#[test]
fn parse_open_order_rejects_bad_price_instead_of_zero() {
    let mut row = open_order(false, "limit", "buy", "open", 0.0, "0");
    row.price = "not-a-price".into();

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid price")
    ));
}

#[test]
fn parse_open_order_rejects_bad_filled_value_instead_of_zero() {
    let row = open_order(false, "limit", "buy", "open", 2.0, "bad");

    assert!(matches!(
        parse_order_with_fills(&row, &[fill("1", 2.0, 30.0, 0.01)]),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid filledValue")
    ));
}

#[test]
fn parse_open_order_rejects_filled_quantity_without_fee_evidence() {
    let row = open_order(false, "limit", "buy", "open", 1.0, "30");

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("no /api/v1/fills fee evidence")
    ));
}

#[test]
fn parse_open_order_rejects_negative_filled_size() {
    let row = open_order(false, "limit", "buy", "open", -1.0, "0");

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("negative filledSize")
    ));
}

#[test]
fn parse_open_order_rejects_unknown_status_instead_of_pending() {
    let row = open_order(false, "limit", "buy", "mystery", 0.0, "0");

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("unknown status")
    ));
}

#[test]
fn parse_open_order_rejects_unknown_side_instead_of_buy() {
    let row = open_order(false, "limit", "both", "open", 0.0, "0");

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("unknown side")
    ));
}

#[test]
fn parse_open_order_rejects_unknown_type_instead_of_limit() {
    let row = open_order(false, "icebergish", "buy", "open", 0.0, "0");

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("unknown type")
    ));
}

#[test]
fn parse_open_orders_propagates_row_parse_problem() {
    let mut bad = open_order(false, "limit", "buy", "open", 0.0, "0");
    bad.price = "bad".into();
    let page = PaginatedOrders {
        items: vec![open_order(false, "limit", "buy", "open", 0.0, "0"), bad],
    };

    assert!(matches!(
        parse_open_orders(&page),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid price")
    ));
}

#[test]
fn paginated_orders_requires_items_field() {
    assert!(serde_json::from_value::<PaginatedOrders>(serde_json::json!({})).is_err());
}

#[test]
fn open_order_requires_official_core_fields() {
    let missing_filled_size = serde_json::json!({
        "id": "1",
        "symbol": "XBTUSDTM",
        "side": "buy",
        "type": "limit",
        "status": "open",
        "price": "30000",
        "size": 1,
        "filledValue": "0",
        "createdAt": 1
    });

    assert!(serde_json::from_value::<OpenOrderItem>(missing_filled_size).is_err());

    let missing_status = serde_json::json!({
        "id": "1",
        "symbol": "XBTUSDTM",
        "side": "buy",
        "type": "limit",
        "price": "30000",
        "size": 1,
        "filledSize": 0,
        "filledValue": "0",
        "createdAt": 1
    });
    assert!(serde_json::from_value::<OpenOrderItem>(missing_status).is_err());
}

#[test]
fn parse_open_order_rejects_invalid_created_at_instead_of_now() {
    let mut row = open_order(false, "limit", "buy", "open", 0.0, "0");
    row.created_at = i64::MAX;

    assert!(matches!(
        parse_open_order(&row),
        Err(crate::ExchangeError::Parse(message)) if message.contains("invalid createdAt")
    ));
}

#[test]
fn parse_positions_maps_official_maintenance_margin_req() {
    let mut row = position("XBTUSDTM", 2.0, true);
    row.maint_margin_req = Some(num(0.012));

    let parsed = parse_positions(&[row], None).expect("parse positions");

    assert!((parsed[0].maintenance_margin_ratio - 0.012).abs() < 1e-12);
}

#[test]
fn parse_positions_rejects_missing_maintenance_margin_req() {
    let mut row = position("XBTUSDTM", 2.0, true);
    row.maint_margin_req = None;

    assert!(matches!(
        parse_positions(&[row], None),
        Err(crate::ExchangeError::Parse(message)) if message.contains("missing maintMarginReq")
    ));
}

#[test]
fn parse_positions_rejects_zero_leverage_instead_of_coercing_to_one() {
    let mut row = position("XBTUSDTM", 2.0, true);
    row.leverage = Some(num(0.0));

    assert!(matches!(
        parse_positions(&[row], None),
        Err(crate::ExchangeError::Parse(message)) if message.contains("non-positive leverage")
    ));
}

#[test]
fn parse_positions_rejects_negative_maintenance_margin_req() {
    let mut row = position("XBTUSDTM", 2.0, true);
    row.maint_margin_req = Some(num(-0.01));

    assert!(matches!(
        parse_positions(&[row], None),
        Err(crate::ExchangeError::Parse(message)) if message.contains("maintMarginReq")
    ));
}

#[test]
fn strict_account_fixture_rejects_defaulting() {
    let mut missing_available_margin: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/account_overview_usdt.json"
    ))
    .expect("account fixture json");
    missing_available_margin["data"]
        .as_object_mut()
        .expect("account data")
        .remove("availableMargin");
    let account: AccountOverview =
        serde_json::from_value(missing_available_margin["data"].clone()).expect("account data");
    assert!(parse_balance_response(&account, "USDT").is_err());

    let mut non_finite: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/account_overview_usdt.json"
    ))
    .expect("account fixture json");
    non_finite["data"]["accountEquity"] = serde_json::json!("NaN");
    let account: AccountOverview =
        serde_json::from_value(non_finite["data"].clone()).expect("account data");
    assert!(parse_balance_response(&account, "USDT").is_err());

    let mut legacy_status: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/get_order_by_client_oid_open.json"
    ))
    .expect("order fixture json");
    legacy_status["data"]["status"] = serde_json::json!("active");
    let order = serde_json::from_value::<
        crate::adapters::kucoin_response::KucoinResponse<OpenOrderItem>,
    >(legacy_status)
    .expect("order envelope")
    .into_data("get order")
    .expect("order data");
    assert!(parse_open_order(&order).is_err());

    let mut missing_timestamp: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/get_order_by_client_oid_open.json"
    ))
    .expect("order fixture json");
    missing_timestamp["data"]
        .as_object_mut()
        .expect("order data")
        .remove("createdAt");
    assert!(serde_json::from_value::<
        crate::adapters::kucoin_response::KucoinResponse<OpenOrderItem>,
    >(missing_timestamp)
    .is_err());

    let mut invalid_timestamp: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/get_order_by_client_oid_open.json"
    ))
    .expect("order fixture json");
    invalid_timestamp["data"]["createdAt"] = serde_json::json!(0);
    let order = serde_json::from_value::<
        crate::adapters::kucoin_response::KucoinResponse<OpenOrderItem>,
    >(invalid_timestamp)
    .expect("order envelope")
    .into_data("get order")
    .expect("order data");
    assert!(parse_open_order(&order).is_err());

    assert!(parse_open_order(&open_order(true, "market", "buy", "open", 0.0, "0")).is_err());
}

fn position(symbol: &str, current_qty: f64, is_open: bool) -> PositionRow {
    PositionRow {
        symbol: symbol.into(),
        current_qty: Some(num(current_qty)),
        avg_entry_price: Some(num(30_000.0)),
        mark_price: Some(num(31_000.0)),
        unrealised_pnl: Some(num(5.0)),
        leverage: Some(num(2.0)),
        margin_mode: None,
        liquidation_price: Some(num(20_000.0)),
        pos_margin: Some(num(100.0)),
        maint_margin_req: Some(num(0.008)),
        is_open: Some(is_open),
    }
}

fn num(value: f64) -> serde_json::Value {
    serde_json::json!(value)
}

fn open_order(
    post_only: bool,
    order_type: &str,
    side: &str,
    status: &str,
    filled_size: f64,
    filled_value: &str,
) -> OpenOrderItem {
    OpenOrderItem {
        id: "1".into(),
        symbol: "XBTUSDTM".into(),
        side: side.into(),
        order_type: order_type.into(),
        status: status.into(),
        price: "30000".into(),
        size: 3.0,
        filled_size,
        filled_value: filled_value.into(),
        cancel_exist: false,
        created_at: 1,
        post_only,
        time_in_force: Some("GTC".to_owned()),
        client_oid: String::new(),
        reduce_only: false,
    }
}

fn fill(order_id: &str, size: f64, price: f64, fee: f64) -> FillEvidence {
    FillEvidence {
        symbol: "XBTUSDTM".into(),
        trade_id: "trade-1".into(),
        order_id: order_id.into(),
        side: OrderSide::Buy,
        liquidity: KucoinLiquidity::Taker,
        price,
        size,
        value: size * price,
        fee,
        fee_rate: 0.0006,
        fee_currency: "USDT".into(),
        settle_currency: "USDT".into(),
        occurred_at_ms: 1_700_000_000_000,
        source_url: KUCOIN_FILLS_DOC_URL,
    }
}

fn intent(side: OrderSide, reduce_only: bool) -> OrderIntent {
    OrderIntent {
        id: "internal-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "kucoin".into(),
        symbol: "BTC".into(),
        side,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(30_000.0),
        slippage_tolerance_bps: None,
        reduce_only,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "client-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

// PR-CY: official KuCoin Futures REST response-envelope fixtures, exercised
// end-to-end through KucoinResponse::into_data + parse_open_order /
// ack_from_cancel_row. The struct-level tests above build OpenOrderItem /
// PositionRow directly and so skip the official `{code,data}` envelope, the
// non-success `code` fail-closed path, and the null-`data` guard (KuCoin signals
// a missing order by returning success code with null data, which must
// fail-closed rather than be coerced to a fabricated order).
const KUCOIN_OFFICIAL_ORDER_DATA: &str = r#"{"id":"5c35c02703aa673ceec2a168","symbol":"BTCUSDT","type":"limit","side":"buy","status":"done","price":"30000","size":1,"filledSize":1,"filledValue":"30000","cancelExist":false,"createdAt":1700000000000,"postOnly":false,"clientOid":"kucoin-cli-7","reduceOnly":false}"#;

fn kucoin_envelope(code: &str, data: &str) -> String {
    format!(r#"{{"code":"{code}","data":{data}}}"#)
}

#[test]
fn get_order_official_envelope_parses_filled_order() {
    let text = kucoin_envelope("200000", KUCOIN_OFFICIAL_ORDER_DATA);
    let resp: crate::adapters::kucoin_response::KucoinResponse<OpenOrderItem> =
        serde_json::from_str(&text).expect("envelope decodes");
    let row = resp
        .into_data("get order")
        .expect("success code yields data");
    let fills = [fill("5c35c02703aa673ceec2a168", 1.0, 30000.0, 18.0)];
    let parsed = parse_order_with_fills(&row, &fills).expect("order and fee evidence parse");
    assert_eq!(parsed.order_id, "5c35c02703aa673ceec2a168");
    assert!(matches!(parsed.status, OrderStatus::Filled));
    assert!(matches!(parsed.side, OrderSide::Buy));
    assert_eq!(parsed.client_order_id.as_deref(), Some("kucoin-cli-7"));
    assert_eq!(parsed.filled_price, 30000.0);
    assert_eq!(parsed.fees, 18.0);
}

#[test]
fn kucoin_fills_parse_official_fixture_without_defaulting_fee() {
    let body = include_str!("../../fixtures/kucoin/fills_by_order_id.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<FillPage> =
        serde_json::from_str(body).expect("official fills envelope decodes");
    let page = resp.into_data("fills").expect("success code yields fills");
    let fills = parse_fill_page(&page, "284486580251463680").expect("official fills parse");
    let fill = fills.first().expect("fill present");

    assert_eq!(fill.trade_id, "1828954878212");
    assert_eq!(fill.order_id, "284486580251463680");
    assert_eq!(fill.fee, 0.05176506);
    assert_eq!(fill.fee_currency, "USDT");
    assert_eq!(fill.liquidity, KucoinLiquidity::Taker);
    assert_eq!(fill.source_url, KUCOIN_FILLS_DOC_URL);
}

#[test]
fn kucoin_fill_rejects_missing_fee_and_order_mismatch() {
    let missing_fee: FillPage = serde_json::from_value(serde_json::json!({
        "items": [{
            "symbol": "XBTUSDTM", "tradeId": "t1", "orderId": "1",
            "side": "buy", "liquidity": "taker", "price": "10", "size": 1,
            "value": "10", "feeRate": "0.0006", "feeCurrency": "USDT",
            "settleCurrency": "USDT", "createdAt": 1700000000000_i64
        }]
    }))
    .expect("fill page decodes with absent optional raw fee");
    assert!(matches!(
        parse_fill_page(&missing_fee, "1"),
        Err(crate::ExchangeError::Parse(message)) if message.contains("missing fee")
    ));

    let body = include_str!("../../fixtures/kucoin/fills_by_order_id.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<FillPage> =
        serde_json::from_str(body).expect("official fills envelope decodes");
    assert!(matches!(
        parse_fill_page(&resp.into_data("fills").unwrap(), "999"),
        Err(crate::ExchangeError::Parse(message)) if message.contains("order mismatch")
    ));
}

#[test]
fn kucoin_fee_rate_parses_official_fixture_and_source_url() {
    let body = include_str!("../../fixtures/kucoin/futures_actual_fee_xbtusdtm.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<FeeRateRow> =
        serde_json::from_str(body).expect("official fee-rate envelope decodes");
    let evidence = parse_fee_rate(
        &resp.into_data("trade-fees").expect("fee data"),
        "XBTUSDTM",
        1_700_000_000_000,
    )
    .expect("fee-rate evidence parses");

    assert_eq!(evidence.maker_fee_rate, 0.0002);
    assert_eq!(evidence.taker_fee_rate, 0.0006);
    assert_eq!(evidence.source_url, KUCOIN_FEE_RATE_DOC_URL);
}

#[test]
fn kucoin_get_order_by_client_oid_parses_official_fixture() {
    let body = include_str!("../../fixtures/kucoin/get_order_by_client_oid_open.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<OpenOrderItem> =
        serde_json::from_str(body).expect("official get-order envelope decodes");
    let row = resp
        .into_data("get order")
        .expect("success code yields official order data");
    let parsed = parse_open_order(&row).expect("official get-order row parses");

    assert_eq!(parsed.order_id, "250444645610336256");
    assert_eq!(parsed.symbol, "XRP");
    assert!(matches!(parsed.side, OrderSide::Buy));
    assert!(matches!(parsed.status, OrderStatus::Open));
    assert_eq!(
        parsed.client_order_id.as_deref(),
        Some("5c52e11203aa677f33e493fb")
    );
    assert_eq!(parsed.reduce_only, Some(false));
    assert_eq!(parsed.quantity, 1.0);
    assert_eq!(parsed.price, 0.1);
    assert_eq!(parsed.filled_quantity, 0.0);
    assert_eq!(parsed.filled_price, 0.0);
}

#[test]
fn kucoin_open_orders_parses_official_fixture() {
    let body = include_str!("../../fixtures/kucoin/get_order_list_active.json");
    let resp: crate::adapters::kucoin_response::KucoinResponse<PaginatedOrders> =
        serde_json::from_str(body).expect("official open-orders envelope decodes");
    let page = resp
        .into_data("orders")
        .expect("success code yields open-orders page");
    let parsed = parse_open_orders(&page).expect("official open-orders page parses");
    let order = parsed.first().expect("open order present");

    assert_eq!(parsed.len(), 1);
    assert_eq!(order.order_id, "230181737576050688");
    assert_eq!(order.symbol, "PEOPLE");
    assert!(matches!(order.side, OrderSide::Buy));
    assert!(matches!(order.status, OrderStatus::Open));
    assert_eq!(
        order.client_order_id.as_deref(),
        Some("5a80bd847f1811ef8a7faa665a37b3d7")
    );
    assert_eq!(order.quantity, 10.0);
    assert_eq!(order.price, 0.05);
    assert_eq!(order.filled_quantity, 0.0);
}

#[test]
fn get_order_envelope_rejects_non_success_code() {
    let text = kucoin_envelope("400100", KUCOIN_OFFICIAL_ORDER_DATA);
    let resp: crate::adapters::kucoin_response::KucoinResponse<OpenOrderItem> =
        serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_data("get order").is_err());
}

#[test]
fn get_order_envelope_null_data_is_fail_closed() {
    let text = kucoin_envelope("200000", "null");
    let resp: crate::adapters::kucoin_response::KucoinResponse<OpenOrderItem> =
        serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_data("get order").is_err());
}

#[test]
fn cancel_order_official_envelope_builds_ack() {
    let text = kucoin_envelope(
        "200000",
        r#"{"cancelledOrderIds":["5c35c02703aa673ceec2a168"]}"#,
    );
    let resp: crate::adapters::kucoin_response::KucoinResponse<
        crate::adapters::kucoin_trade_data::KucoinCancelRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    let row = resp
        .into_data("cancel order")
        .expect("success code yields data");
    let ack = crate::adapters::kucoin_trade_data::ack_from_cancel_row(
        "internal-1".to_owned(),
        "public-1".to_owned(),
        None,
        row,
    );
    assert_eq!(
        ack.exchange_order_id.as_deref(),
        Some("5c35c02703aa673ceec2a168")
    );
    assert!(matches!(
        ack.state,
        shared_types::LiveOrderState::CancelRequested
    ));
}

#[test]
fn cancel_order_envelope_rejects_non_success_code() {
    let text = kucoin_envelope("400100", r#"{"cancelledOrderIds":[]}"#);
    let resp: crate::adapters::kucoin_response::KucoinResponse<
        crate::adapters::kucoin_trade_data::KucoinCancelRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_data("cancel order").is_err());
}

// PR-CY: official KuCoin Futures place-order response-envelope fixtures.
// place_order decodes KucoinResponse<KucoinOrderAckRow> via into_data + builds
// the ack via ack_from_order_row, a path distinct from cancel (which carries
// cancelledOrderIds, not the {orderId,clientOid} shape). The cancel/get fixtures
// never exercised this place shape: success -> Accepted ack carrying orderId as
// the exchange order id and the official clientOid, non-success code -> fail
// closed, success code with null data -> fail-closed Parse error.
#[test]
fn place_order_official_envelope_builds_accepted_ack() {
    let text = kucoin_envelope(
        "200000",
        r#"{"orderId":"5c35c02703aa673ceec2a168","clientOid":"kucoin-cli-7"}"#,
    );
    let resp: crate::adapters::kucoin_response::KucoinResponse<
        crate::adapters::kucoin_trade_data::KucoinOrderAckRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    let row = resp
        .into_data("place order")
        .expect("success code yields data");
    let ack = crate::adapters::kucoin_trade_data::ack_from_order_row(
        "internal-1".to_owned(),
        "public-1".to_owned(),
        row,
        shared_types::LiveOrderState::Accepted,
        None,
    );
    assert_eq!(
        ack.exchange_order_id.as_deref(),
        Some("5c35c02703aa673ceec2a168")
    );
    assert!(matches!(ack.state, shared_types::LiveOrderState::Accepted));
    assert!(ack.message.is_none());
}

#[test]
fn place_order_envelope_rejects_non_success_code() {
    let text = kucoin_envelope(
        "400100",
        r#"{"orderId":"5c35c02703aa673ceec2a168","clientOid":"kucoin-cli-7"}"#,
    );
    let resp: crate::adapters::kucoin_response::KucoinResponse<
        crate::adapters::kucoin_trade_data::KucoinOrderAckRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_data("place order").is_err());
}

#[test]
fn place_order_envelope_null_data_is_fail_closed() {
    let text = kucoin_envelope("200000", "null");
    let resp: crate::adapters::kucoin_response::KucoinResponse<
        crate::adapters::kucoin_trade_data::KucoinOrderAckRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_data("place order").is_err());
}
