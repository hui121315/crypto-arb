use super::*;
use crate::adapters::kraken::{Kraken, KrakenConfig};
use crate::adapters::kraken_stock_receipts::tests::{frame, pending};
use futures_util::{SinkExt, StreamExt};
use shared_types::stocks::{StockCexOrderPhase, StockChainDirection};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn original(client: &str) -> StockPeerOrderReceipt {
    let mut original = pending();
    let now = common::time::now_ms();
    original.client_order_id = client.into();
    original.draft.prepared_at_ms = now;
    original.draft.source_at_ms = now;
    original.draft.metadata_at_ms = now;
    original
}

#[test]
fn stock_submit_compiler_and_ack_keep_validation_freshness_identity_and_fee_boundaries() {
    let original = original("stock-original");
    let now = original.draft.prepared_at_ms;
    let submit = original.kraken_submission("local", 7, now).unwrap();
    assert_eq!(submit["params"]["validate"], false);
    assert_eq!(submit["params"]["cl_ord_id"], "stock-original");
    assert_eq!(submit["params"]["side"], "sell");
    assert_eq!(submit["params"]["margin"], false);
    assert_eq!(submit["params"]["fee_preference"], "quote");
    assert_eq!(submit["params"]["time_in_force"], "fok");
    let validation = original.draft.kraken_validation("local", 7, now).unwrap();
    assert_eq!(validation["params"]["validate"], true);
    let late = original.kraken_submission("local", 8, now + 2_400).unwrap();
    let deadline =
        chrono::DateTime::parse_from_rfc3339(late["params"]["deadline"].as_str().unwrap())
            .unwrap()
            .timestamp_millis();
    assert_eq!(deadline, now + 3_000);
    assert!(original.kraken_submission("local", 9, now + 2_501).is_err());
    let mut buy = original.clone();
    buy.draft.request.direction = StockChainDirection::Sell;
    assert_eq!(
        buy.kraken_submission("local", 9, now).unwrap()["params"]["side"],
        "buy"
    );
    for (source, metadata) in [(now - 3_001, now), (now + 1, now), (now, now - 60_001)] {
        let mut bad = original.clone();
        bad.draft.source_at_ms = source;
        bad.draft.metadata_at_ms = metadata;
        assert!(bad.kraken_submission("local", 1, now).is_err());
    }
    let mut tiny = original.clone();
    tiny.draft.quantity = "1.12345678901234567890123456789".into();
    assert!(tiny.kraken_submission("local", 1, now).is_err());
    for body in [
        json!({"method":"add_order","req_id":8,"success":true,"result":{"order_id":"O1"}}),
        json!({"method":"add_order","req_id":7,"success":true,"result":{}}),
        json!({"method":"add_order","req_id":7,"success":true,"result":{"order_id":"O1","cl_ord_id":"other"}}),
        json!({"method":"add_order","req_id":7,"success":false,"error":"reject","result":{"order_id":"O1"}}),
        json!({"method":"add_order","req_id":7,"success":false,"error":"reject","result":[]}),
    ] {
        assert!(parse_order_ack(&body.to_string(), "stock-original", 7, now).is_err());
    }
    let (ack,id)=parse_order_ack(&json!({"method":"add_order","req_id":7,"success":false,"error":"EOrder:Insufficient funds local-secret"}).to_string(),"stock-original",7,now).unwrap();
    assert!(!ack.message.contains("local-secret"));
    let mut rejected = original.clone();
    rejected.record_submission_ack(ack, id).unwrap();
    assert!(rejected.rejection_proven());
    assert!(rejected.receipt_complete());
    assert_eq!(rejected.cash_settlement().unwrap().quote_change, "0");
    let cache = crate::adapters::kraken_stock_receipts::StockReceipts::default();
    cache.track(rejected.clone()).unwrap();
    assert_eq!(cache.get("stock-original").unwrap(), rejected);
    let mut contradictory = frame(1, false);
    contradictory["data"][0]["cl_ord_id"] = json!("stock-original");
    cache.apply(&contradictory.to_string());
    let conflict = cache.get("stock-original").unwrap();
    assert!(conflict.evidence_conflict);
    assert!(!conflict.receipt_complete());
    let restored = crate::adapters::kraken_stock_receipts::StockReceipts::default();
    restored.track(conflict.clone()).unwrap();
    assert_eq!(restored.get("stock-original").unwrap(), conflict);
    assert!(restored.release(&conflict).is_err());
    let late = crate::adapters::kraken_stock_receipts::StockReceipts::default();
    let mut unknown = original.clone();
    unknown.problem = Some("submit not confirmed".into());
    late.track(unknown).unwrap();
    for mut f in [frame(1, false), frame(2, true)] {
        f["data"][0]["cl_ord_id"] = json!("stock-original");
        late.apply(&f.to_string());
    }
    let known = late.get("stock-original").unwrap();
    assert!(known.receipt_complete());
    assert!(known.problem.is_none());
    let (ack, id) = parse_order_ack(
        &json!({"method":"add_order","req_id":7,"success":true,"result":{"order_id":"O1"}})
            .to_string(),
        "stock-original",
        7,
        now,
    )
    .unwrap();
    assert!(rejected.record_submission_ack(ack, id).is_err());
    assert!(!rejected.receipt_complete());
    let mut known = original.clone();
    known.order_id = Some("O1".into());
    for body in [
        json!({"method":"cancel_order","req_id":7,"success":true,"result":{"order_id":"O1","cl_ord_id":123}}),
        json!({"method":"cancel_order","req_id":7,"success":true,"result":{"order_id":"O2","cl_ord_id":"stock-original"}}),
        json!({"method":"cancel_order","req_id":8,"success":true,"result":{"order_id":"O1"}}),
    ] {
        assert_eq!(parse_cancel_ack(&body.to_string(), &known, 7), None);
    }
}

#[tokio::test]
async fn stock_submit_live_gate_rejects_before_initializing_private_connection() {
    let adapter = Kraken::new(KrakenConfig::default()).unwrap();
    let original = original("stock-disabled");
    let result = adapter
        .submit_stock_order(original.draft, original.client_order_id)
        .await;
    assert!(matches!(result, Err(ExchangeError::Auth(_))));
    assert!(matches!(
        adapter.cancel_stock_order("stock-disabled").await,
        Err(ExchangeError::Auth(_))
    ));
    assert!(adapter.spot_private_stream.get().is_none());
}

#[tokio::test]
async fn stock_submit_socket_exact_fills_cancel_ack_unknown_restore_and_no_resubmission() {
    let http = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/0/private/GetWebSocketsToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"error":[],"result":{"token":"local-trade-token","expires":900}}),
        ))
        .expect(1)
        .mount(&http)
        .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let (cancel_final, mut cancel_ready) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        let mut commands = Vec::new();
        let mut emit_cancel = true;
        loop {
            tokio::select! {
                _=&mut stopped=>break,
                _=&mut cancel_ready, if emit_cancel=>{
                    emit_cancel=false;let mut cancelled=frame(3,false);
                    let r=&mut cancelled["data"][0];r["cl_ord_id"]=json!("stock-cancel");r["order_id"]=json!("O-CANCEL");
                    r["exec_type"]=json!("canceled");r["order_status"]=json!("canceled");r["cum_qty"]=json!("0");r["cum_cost"]=json!("0");
                    r["timestamp"]=json!(chrono::DateTime::from_timestamp_millis(common::time::now_ms()).unwrap().to_rfc3339());
                    ws.send(Message::Text(cancelled.to_string())).await.unwrap();
                },
                incoming=ws.next()=>{
                    let text=match incoming {
                        Some(Ok(Message::Text(text)))=>text,
                        Some(Ok(Message::Ping(data)))=>{ws.send(Message::Pong(data)).await.unwrap();continue;},
                        Some(Ok(Message::Pong(_)))=>continue,
                        Some(Ok(Message::Close(_)))|None=>break,
                        other=>panic!("unexpected local stock socket message {other:?}"),
                    };
                    let v:Value=serde_json::from_str(&text).unwrap();
                    if v["method"]=="subscribe" {continue;}
                    let id=v["req_id"].as_u64().unwrap();commands.push(v.clone());
                    if v["method"]=="cancel_order" {
                        assert_eq!(v["params"]["order_id"],json!(["O-CANCEL"]));assert!(v["params"].get("cl_ord_id").is_none());
                        ws.send(Message::Text(json!({"method":"cancel_order","req_id":id,"success":true,"result":{"order_id":"O-CANCEL","cl_ord_id":"stock-cancel"}}).to_string())).await.unwrap();continue;
                    }
                    assert_eq!(v["method"],"add_order");assert_eq!(v["params"]["validate"],false);
                    assert_eq!(v["params"]["order_qty"].to_string(),"1");assert_eq!(v["params"]["limit_price"].to_string(),"600");
                    assert_eq!(v["params"]["symbol"],"MUx/USD");assert_eq!(v["params"]["margin"],false);
                    assert_eq!(v["params"]["time_in_force"],"fok");assert_eq!(v["params"]["fee_preference"],"quote");
                    let client=v["params"]["cl_ord_id"].as_str().unwrap();
                    if client=="stock-timeout" {continue;}
                    if client=="stock-rejected" {
                        ws.send(Message::Text(json!({"method":"add_order","req_id":id,"success":false,"error":"EOrder:Insufficient funds local-trade-token"}).to_string())).await.unwrap();continue;
                    }
                    let order=if client=="stock-live" {"O-STOCK-1"}else{"O-CANCEL"};
                    ws.send(Message::Text(json!({"method":"add_order","req_id":id+1,"success":true,"result":{"order_id":"wrong-request"}}).to_string())).await.unwrap();
                    if client=="stock-live" {
                        for mut f in [frame(1,false),frame(2,true)] {
                            f["data"][0]["cl_ord_id"]=json!(client);
                            f["data"][0]["timestamp"]=json!(chrono::DateTime::from_timestamp_millis(common::time::now_ms()).unwrap().to_rfc3339());
                            ws.send(Message::Text(f.to_string())).await.unwrap();
                        }
                    }
                    ws.send(Message::Text(json!({"method":"add_order","req_id":id,"success":true,"result":{"order_id":order,"cl_ord_id":client}}).to_string())).await.unwrap();
                }
            }
        }
        commands
    });
    let credentials = KrakenSpotCredentials {
        api_key: "local-stock-trade".into(),
        api_secret: "c2VjcmV0".into(),
    };
    let client = HttpClient::new("local-stock-trade").unwrap();
    let stream = KrakenSpotPrivateStream::new(&url, &http.uri(), &credentials, &client, 1);
    let live = original("stock-live");
    let (a, b) = tokio::join!(
        stream.submit_stock_order(live.clone()),
        stream.submit_stock_order(live.clone())
    );
    let filled = a.unwrap();
    let duplicate = b.unwrap();
    assert!(filled.receipt_complete());
    assert_eq!(filled.cash_settlement().unwrap().quote_change, "599.9994");
    assert_eq!(filled.order_id.as_deref(), Some("O-STOCK-1"));
    assert!(filled.submission_ack.as_ref().unwrap().accepted);
    assert_eq!(duplicate.client_order_id, "stock-live");
    let cancelling = original("stock-cancel");
    let acknowledged = stream.submit_stock_order(cancelling).await.unwrap();
    assert!(!acknowledged.receipt_complete());
    assert_eq!(acknowledged.phase, StockCexOrderPhase::SubmissionUnknown);
    let cancel_ack = stream.cancel_stock_order("stock-cancel").await.unwrap();
    assert_eq!(cancel_ack.accepted, Some(true));
    assert!(!stream
        .stock_order_receipt("stock-cancel")
        .unwrap()
        .receipt_complete());
    let mut receipts = stream.subscribe_stock_receipts();
    let _ = cancel_final.send(());
    let cancelled = tokio::time::timeout(Duration::from_secs(2), receipts.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.phase, StockCexOrderPhase::Cancelled);
    assert!(cancelled.receipt_complete());
    let unknown = original("stock-timeout");
    let uncertain = stream.submit_stock_order(unknown.clone()).await.unwrap();
    assert_eq!(uncertain.phase, StockCexOrderPhase::SubmissionUnknown);
    assert!(uncertain.submission_ack.is_none());
    assert!(!uncertain
        .problem
        .as_ref()
        .unwrap()
        .contains("local-trade-token"));
    assert_eq!(
        stream.submit_stock_order(unknown.clone()).await.unwrap(),
        uncertain
    );
    let restored = KrakenSpotPrivateStream::new(&url, &http.uri(), &credentials, &client, 1);
    restored
        .track_stock_order(
            serde_json::from_str(&serde_json::to_string(&uncertain).unwrap()).unwrap(),
        )
        .unwrap();
    assert_eq!(
        restored.submit_stock_order(unknown).await.unwrap(),
        uncertain
    );
    let rejected = stream
        .submit_stock_order(original("stock-rejected"))
        .await
        .unwrap();
    assert!(rejected.rejection_proven());
    assert!(rejected.receipt_complete());
    stream.release_stock_receipt(&filled).unwrap();
    assert!(stream.submit_stock_order(live).await.is_err());
    let _ = stop.send(());
    let commands = server.await.unwrap();
    assert_eq!(
        commands
            .iter()
            .filter(|v| v["method"] == "add_order")
            .count(),
        4
    );
    assert_eq!(
        commands
            .iter()
            .filter(|v| v["method"] == "cancel_order")
            .count(),
        1
    );
    assert_eq!(
        http.received_requests().await.unwrap().len(),
        1,
        "restore must not fetch a token or reconnect to resend"
    );
}
