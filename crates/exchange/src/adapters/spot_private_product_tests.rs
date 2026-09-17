use super::{bitget_uta_ws_user, bybit_ws_user, hyperliquid_ws_user, okx_ws_user};
use shared_types::OrderStatus;

#[test]
fn okx_any_orders_stream_accepts_spot_terminals() {
    let event = okx_ws_user::parse_user_event(
        r#"{
            "arg":{"channel":"orders","instType":"ANY"},
            "data":[{
                "instType":"SPOT","instId":"SOL-USDT","ordId":"spot-1",
                "clOrdId":"spot-cid-1","px":"150","sz":"0.4","ordType":"limit",
                "side":"sell","accFillSz":"0.4","avgPx":"150.1","state":"filled",
                "fee":"-0.06","feeCcy":"USDT","tradeId":"fill-1",
                "fillPx":"150.1","fillSz":"0.4","fillFee":"-0.06",
                "fillFeeCcy":"USDT","fillTime":"1780000000100",
                "cTime":"1780000000000","uTime":"1780000000101"
            }]
        }"#,
    )
    .expect("spot event")
    .expect("orders topic");
    let okx_ws_user::OkxUserEvent::Order(rows) = event else {
        panic!("expected OKX spot order");
    };
    assert_eq!(rows[0].order.symbol, "SOL");
    assert_eq!(rows[0].order.status, OrderStatus::Filled);
    assert!(rows[0].fill.is_some());
}

#[test]
fn bybit_all_in_one_stream_accepts_spot_terminals() {
    let event = bybit_ws_user::parse_user_event(
        r#"{
            "topic":"order","id":"spot-1","creationTime":1780000000100,
            "data":[{
                "category":"spot","orderId":"spot-1","orderLinkId":"spot-cid-1",
                "symbol":"SOLUSDT","side":"Sell","orderType":"Market",
                "timeInForce":"IOC","orderStatus":"Filled","qty":"0.4","price":"0",
                "avgPrice":"150.1","cumExecQty":"0.4","cumExecFee":"0.06",
                "positionIdx":0,"cancelType":"UNKNOWN","rejectReason":"EC_NoError",
                "leavesQty":"0","reduceOnly":false,"createdTime":"1780000000000"
            }]
        }"#,
    )
    .expect("spot event")
    .expect("orders topic");
    let bybit_ws_user::BybitUserEvent::Order(rows) = event else {
        panic!("expected Bybit spot order");
    };
    assert_eq!(rows[0].order.symbol, "SOL");
    assert_eq!(rows[0].order.status, OrderStatus::Filled);
}

#[test]
fn bitget_uta_stream_accepts_spot_order_and_fill_terminals() {
    let order = bitget_uta_ws_user::parse_user_event(
        r#"{
            "arg":{"instType":"UTA","topic":"order"},"action":"update",
            "data":[{
                "category":"spot","symbol":"SOLUSDT","orderId":"spot-1",
                "clientOid":"spot-cid-1","price":"150","qty":"0.4",
                "holdMode":"","holdSide":"","tradeSide":"","orderType":"limit",
                "timeInForce":"gtc","side":"sell","reduceOnly":"",
                "cumExecQty":"0.4","avgPrice":"150.1","totalProfit":"0",
                "orderStatus":"filled","cancelReason":"","feeDetail":[
                    {"feeCoin":"USDT","fee":"-0.06"}
                ],"createdTime":"1780000000000","updatedTime":"1780000000100"
            }]
        }"#,
    )
    .expect("spot order")
    .expect("orders topic");
    let bitget_uta_ws_user::BitgetUserEvent::Order(rows) = order else {
        panic!("expected Bitget spot order");
    };
    assert_eq!(rows[0].category, "SPOT");
    assert_eq!(rows[0].order.status, OrderStatus::Filled);

    let fill = bitget_uta_ws_user::parse_user_event(
        r#"{
            "arg":{"instType":"UTA","topic":"fill"},"action":"update",
            "data":[{
                "category":"spot","symbol":"SOLUSDT","orderId":"spot-1",
                "clientOid":"spot-cid-1","execId":"fill-1","side":"sell",
                "holdSide":"","tradeSide":"","execPrice":"150.1",
                "execQty":"0.4","execValue":"60.04","execPnl":"0",
                "feeDetail":[{"feeCoin":"USDT","fee":"-0.06"}],
                "execTime":"1780000000100","updatedTime":"1780000000101","isRPI":"no"
            }]
        }"#,
    )
    .expect("spot fill")
    .expect("fill topic");
    let bitget_uta_ws_user::BitgetUserEvent::Fill(rows) = fill else {
        panic!("expected Bitget spot fill");
    };
    assert_eq!(rows[0].category, "SPOT");
    assert_eq!(rows[0].size, 0.4);
}

#[test]
fn hyperliquid_global_stream_accepts_spot_asset_terminals() {
    let event = hyperliquid_ws_user::parse_user_event(
        r#"{
            "channel":"orderUpdates",
            "data":[{
                "order":{
                    "coin":"@107","side":"B","limitPx":"1.25","sz":"0",
                    "oid":107001,"timestamp":1780000000000,"origSz":"8",
                    "cloid":"0x01010101010101010101010101010101"
                },
                "status":"filled","statusTimestamp":1780000000100
            }]
        }"#,
    )
    .expect("spot event")
    .expect("order updates topic");
    let hyperliquid_ws_user::HyperliquidUserWsEvent::Order(rows) = event else {
        panic!("expected Hyperliquid spot order");
    };
    assert_eq!(rows[0].order.symbol, "@107");
    assert_eq!(rows[0].order.status, OrderStatus::Filled);
}
