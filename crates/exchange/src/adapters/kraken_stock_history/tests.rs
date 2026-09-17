use super::*;
use crate::{
    adapters::kraken::{KrakenConfig, KrakenCredentials, KrakenSpotCredentials},
    ExchangeAdapter,
};
use wiremock::{
    matchers::{body_partial_json, header_exists, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};
const AT: i64 = 1_700_000_000_000;

fn fixture() -> (StockPeerOrderReceipt, Value, Map<String, Value>) {
    let mut original = crate::adapters::kraken_stock_receipts::tests::pending();
    original.draft.prepared_at_ms = AT;
    original.draft.source_at_ms = AT;
    original.draft.metadata_at_ms = AT;
    let order = json!({"cl_ord_id":original.client_order_id,"status":"closed","opentm":"1700000000.001","closetm":"1700000000.003",
        "descr":{"pair":"MUXUSD","type":"sell","ordertype":"limit","price":"600","leverage":"none"},
        "vol":"1","vol_exec":"1","cost":"600.6","fee":"0.6006","oflags":"fciq","trades":["T-1","T-2"]});
    let trades = json!({
        "T-1":{"ordertxid":"O-STOCK-1","pair":"MUXUSD","type":"sell","ordertype":"limit","margin":"0","time":"1700000000.0019",
            "trade_id":41,"price":"600","vol":"0.4","cost":"240","fee":"0.24"},
        "T-2":{"ordertxid":"O-STOCK-1","pair":"MUXUSD","type":"sell","ordertype":"limit","margin":"0","time":"1700000000.0029",
            "trade_id":42,"price":"601","vol":"0.6","cost":"360.6","fee":"0.3606"}
    }).as_object().unwrap().clone();
    (original, order, trades)
}

#[test]
fn stock_history_full_receipt_merges_ws_by_trade_id_without_double_fills_or_fees() {
    let (original, order, trades) = fixture();
    let history = parse_history(
        &original,
        "O-STOCK-1",
        &order,
        &trades,
        &["MUXUSD"],
        AT + 10,
    )
    .unwrap();
    assert_eq!(history.cash_settlement().unwrap().quote_change, "599.9994");
    assert_eq!(history.updated_at_ms, Some(AT + 3));
    assert!(
        history.submission_ack.is_none(),
        "history is not a fabricated submit ACK"
    );
    let mut ws = history.clone();
    ws.fills.truncate(1);
    ws.fills[0].execution_id = "WS-native-execution-1".into();
    ws.fills[0].fees = None;
    ws.validate_stored().unwrap();
    assert!(!ws.receipt_complete());
    ws.merge_snapshot(&history).unwrap();
    assert_eq!(ws.fills.len(), 2);
    assert!(ws
        .fills
        .iter()
        .any(|f| f.execution_id == "WS-native-execution-1"));
    assert_eq!(ws.cash_settlement(), history.cash_settlement());
    ws.validate_stored().unwrap();
    let saved = serde_json::to_string(&ws).unwrap();
    let mut restored: StockPeerOrderReceipt = serde_json::from_str(&saved).unwrap();
    restored.merge_snapshot(&history).unwrap();
    assert_eq!(restored, ws);
    let mut contradiction = history;
    contradiction.fills[0].fees.as_mut().unwrap()[0].quantity = "1".into();
    assert!(restored.merge_snapshot(&contradiction).is_err());
    assert!(restored.cash_settlement().is_none());
}

#[test]
fn stock_history_rejects_missing_fee_wrong_identity_incomplete_totals_and_duplicate_trades() {
    for case in 0..12 {
        let (original, mut order, mut trades) = fixture();
        match case {
            0 => {
                order["cl_ord_id"] = json!("other");
            }
            1 => {
                order["descr"]["pair"] = json!("SNDKUSD");
            }
            2 => {
                trades
                    .get_mut("T-1")
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("fee");
            }
            3 => {
                trades.get_mut("T-1").unwrap()["ordertxid"] = json!("O-OTHER");
            }
            4 => {
                order["vol_exec"] = json!("0.9");
            }
            5 => {
                order["fee"] = json!("0");
            }
            6 => {
                order["oflags"] = json!("fcib");
            }
            7 => {
                trades.remove("T-2");
            }
            8 => {
                trades.get_mut("T-2").unwrap()["trade_id"] = json!(41);
            }
            9 => {
                order["closetm"] = json!("9999999999");
            }
            10 => {
                order["vol"] = json!("2");
            }
            _ => {
                order["trades"] = json!(["T-1", "T-1"]);
            }
        }
        assert!(
            parse_history(
                &original,
                "O-STOCK-1",
                &order,
                &trades,
                &["MUXUSD"],
                AT + 10
            )
            .is_err(),
            "case {case}"
        );
    }
    let (original, mut order, mut trades) = fixture();
    order["status"] = json!("canceled");
    order["vol_exec"] = json!("0");
    order["cost"] = json!("0");
    order["fee"] = json!("0");
    order["trades"] = json!([]);
    trades.clear();
    let r = parse_history(
        &original,
        "O-STOCK-1",
        &order,
        &trades,
        &["MUXUSD"],
        AT + 10,
    )
    .unwrap();
    assert!(r.receipt_complete());
    assert_eq!(r.cash_settlement().unwrap().quote_change, "0");
    order.as_object_mut().unwrap().remove("fee");
    assert!(parse_history(
        &original,
        "O-STOCK-1",
        &order,
        &trades,
        &["MUXUSD"],
        AT + 10
    )
    .is_err());
}

#[tokio::test]
async fn stock_history_signed_original_order_reads_recover_lost_ack_without_trading() {
    for (lost_ack, conversion) in [(true, false), (false, false), (true, true), (false, true)] {
        let server = MockServer::start().await;
        let (mut original, mut order, mut trades) = fixture();
        let (native, pair, class) = if conversion {
            original.draft.purpose = StockPeerOrderPurpose::CashConversion;
            original.draft.request.selection.native_symbol = "USDC/USD".into();
            original.draft.quantity = "12".into();
            original.draft.limit_price = "1".into();
            order["descr"]["pair"] = json!("USDCUSD");
            order["descr"]["price"] = json!("1");
            order["vol"] = json!("12");
            order["vol_exec"] = json!("12");
            order["cost"] = json!("12");
            order["fee"] = json!("0.024");
            for trade in trades.values_mut() {
                trade["pair"] = json!("USDCUSD");
                trade["price"] = json!("1");
                trade["vol"] = json!("6");
                trade["cost"] = json!("6");
                trade["fee"] = json!("0.012");
            }
            ("USDC/USD", "USDCUSD", "currency")
        } else {
            ("MUx/USD", "MUXUSD", "tokenized_asset")
        };
        if !lost_ack {
            original.order_id = Some("O-STOCK-1".into());
        }
        // Restore requires an ACK or execution when an original order ID is known.
        if !lost_ack {
            original
                .record_submission_ack(
                    StockPeerOrderAck {
                        accepted: true,
                        request_id: 1,
                        received_at_ms: AT + 1,
                        message: "fixture ACK".into(),
                    },
                    Some("O-STOCK-1".into()),
                )
                .unwrap();
        }
        for (key, endpoint, rows) in [
            ("closed", "ClosedOrders", json!({"O-STOCK-1":order})),
            ("open", "OpenOrders", json!({})),
        ] {
            Mock::given(method("POST"))
                .and(path(format!("/0/private/{endpoint}")))
                .and(body_partial_json(
                    json!({"cl_ord_id":"stock-test-1","rebase_multiplier":"rebased"}),
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"error":[],"result":{key:rows,"count":1}})),
                )
                .expect(if lost_ack { 1 } else { 0 })
                .mount(&server)
                .await;
        }
        Mock::given(method("POST")).and(path("/0/private/QueryOrders"))
            .and(header_exists("API-Sign")).and(body_partial_json(json!({"txid":"O-STOCK-1","trades":true,"consolidate_taker":false,"rebase_multiplier":"rebased"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error":[],"result":{"O-STOCK-1":order}}))).expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/0/private/QueryTrades"))
            .and(body_partial_json(
                json!({"txid":"T-1,T-2","rebase_multiplier":"rebased"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"error":[],"result":trades})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/0/public/AssetPairs")).and(query_param("pair",native)).and(query_param("aclass_base",class))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error":[],"result":{pair:{"wsname":native,"altname":pair,"aclass_base":class,"quote":"ZUSD","status":"cancel_only"}}})))
            .expect(1).mount(&server).await;
        let adapter = Kraken::new(KrakenConfig {
            spot_rest_url_override: Some(server.uri()),
            credentials: Some(KrakenCredentials {
                spot: Some(KrakenSpotCredentials {
                    api_key: "local-fixture".into(),
                    api_secret: "c2VjcmV0".into(),
                }),
                futures: None,
            }),
            allow_live_writes: false,
            ..Default::default()
        })
        .unwrap();
        let recovered = adapter
            .reconcile_stock_order(&original)
            .await
            .unwrap()
            .unwrap();
        original.merge_snapshot(&recovered).unwrap();
        assert!(original.receipt_complete());
        assert_eq!(original.cash_settlement().unwrap().quote_asset, "USD");
        if conversion {
            assert_eq!(original.cash_settlement().unwrap().base_asset, "USDC");
            assert_eq!(original.cash_settlement().unwrap().quote_change, "11.976");
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), if lost_ack { 5 } else { 3 });
        for r in requests.iter().filter(|r| r.method == "POST") {
            assert!(!r.url.path().contains("AddOrder"));
            let b: Value = r.body_json().unwrap();
            let sign = crate::signing::kraken::spot_rest_sign(
                "c2VjcmV0",
                r.url.path(),
                &b["nonce"].to_string(),
                std::str::from_utf8(&r.body).unwrap(),
            )
            .unwrap();
            assert_eq!(r.headers.get("API-Sign").unwrap().to_str().unwrap(), sign);
        }
    }
}
