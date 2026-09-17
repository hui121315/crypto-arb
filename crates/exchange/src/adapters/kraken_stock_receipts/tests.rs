use super::*;
use serde_json::json;
use shared_types::stocks::*;

pub(super) const START: i64 = 1_800_000_000_000;

pub(in crate::adapters) fn pending() -> StockPeerOrderReceipt {
    StockPeerOrderReceipt::pending(
        StockPeerOrderDraft {
            purpose: StockPeerOrderPurpose::Equity,
            request: StockPeerOrderCheckRequest {
                asset: "MU.US".into(),
                selection: StockPeerSelection {
                    venue: "kraken".into(),
                    product: StockPeerProduct::Spot,
                    native_symbol: "MUx/USD".into(),
                },
                direction: StockChainDirection::Buy,
            },
            quantity: "1".into(),
            limit_price: "600".into(),
            quote_asset: "USD".into(),
            prepared_at_ms: START,
            source_at_ms: START,
            metadata_at_ms: START,
        },
        "stock-test-1".into(),
    )
    .unwrap()
}

pub(in crate::adapters) fn frame(sequence: i64, full: bool) -> Value {
    let t = chrono::DateTime::from_timestamp_millis(START + if full { 2 } else { 1 })
        .unwrap()
        .to_rfc3339();
    json!({"channel":"executions","type":if sequence==1 {"snapshot"}else{"update"},"sequence":sequence,"data":[{
        "order_id":"O-STOCK-1","cl_ord_id":"stock-test-1","symbol":"MUx/USD","side":"sell","order_qty":"1",
        "order_type":"limit","time_in_force":"FOK","limit_price":"600","exec_type":"trade",
        "order_status":if full {"filled"}else{"partially_filled"},"exec_id":if full {"F-2"}else{"F-1"},
        "last_qty":if full {"0.6"}else{"0.4"},"last_price":if full {"601"}else{"600"},
        "cost":if full {"360.6"}else{"240"},"cum_qty":if full {"1"}else{"0.4"},
        "cum_cost":if full {"600.6"}else{"240"},"timestamp":t,
        "fees":[{"asset":"USD","qty":if full {"0.3606"}else{"0.24"}}]
    }]})
}

#[test]
fn stock_cash_conversion_receipts_keep_native_fees_and_original_identity() {
    for quote in ["USD", "USDT"] {
        let cache = StockReceipts::default();
        let mut draft = pending().draft;
        draft.purpose = StockPeerOrderPurpose::CashConversion;
        draft.request.selection.native_symbol = format!("USDC/{quote}");
        draft.quote_asset = quote.into();
        draft.quantity = "12".into();
        draft.limit_price = "1".into();
        let original = StockPeerOrderReceipt::pending(draft, "stock-test-1".into()).unwrap();
        let order = original.kraken_submission("local", 1, START).unwrap();
        assert_eq!(order["params"]["symbol"], format!("USDC/{quote}"));
        assert_eq!(order["params"]["side"], "sell");
        assert_eq!(order["params"]["margin"], false);
        cache.track(original).unwrap();
        let mut update = frame(1, true);
        let row = &mut update["data"][0];
        row["symbol"] = json!(format!("USDC/{quote}"));
        for field in ["order_qty", "last_qty", "cum_qty", "cost", "cum_cost"] {
            row[field] = json!("12");
        }
        row["last_price"] = json!("1");
        row["limit_price"] = json!("1");
        row["fees"] = json!([{"asset":quote,"qty":"0.024"}]);
        cache.apply(&update.to_string());
        let settled = cache.get("stock-test-1").unwrap();
        let cash = settled.cash_settlement().unwrap();
        assert_eq!(cash.base_asset, "USDC");
        assert_eq!(cash.equity_shares_change, "-12");
        assert_eq!(cash.quote_change, "11.976");
        assert_eq!(cash.quote_asset, quote);
        cache.apply(&update.to_string());
        assert_eq!(cache.get("stock-test-1").unwrap(), settled);
        update["data"][0]["symbol"] = json!("BTC/USD");
        cache.apply(&update.to_string());
        assert!(cache.get("stock-test-1").unwrap().evidence_conflict);
    }
}

#[test]
fn stock_receipts_exact_replay_restore_and_compare_release() {
    let cache = StockReceipts::default();
    let original = pending();
    cache.track(original.clone()).unwrap();
    let mut updates = cache.subscribe();
    cache.apply(&frame(1, false).to_string());
    let partial = cache.get("stock-test-1").unwrap();
    assert!(!partial.receipt_complete());
    cache.apply(&frame(2, true).to_string());
    let settled = cache.get("stock-test-1").unwrap();
    assert!(settled.receipt_complete());
    let cash = settled.cash_settlement().unwrap();
    assert_eq!(cash.equity_shares_change, "-1");
    assert_eq!(cash.quote_change, "599.9994");
    assert_eq!(cash.quote_asset, "USD");
    cache.apply(&frame(1, false).to_string());
    cache.apply(&frame(2, true).to_string());
    cache.track(original).unwrap();
    assert_eq!(cache.get("stock-test-1").unwrap(), settled);
    assert!(updates.try_recv().is_ok());
    assert!(updates.try_recv().is_ok());
    assert!(updates.try_recv().is_err());
    let persisted = serde_json::to_string(&settled).unwrap();
    let restored = StockReceipts::default();
    restored
        .track(serde_json::from_str(&persisted).unwrap())
        .unwrap();
    restored.apply(&frame(1, false).to_string());
    restored.apply(&frame(2, true).to_string());
    assert_eq!(restored.get("stock-test-1").unwrap(), settled);
    assert!(restored.release(&partial).is_err());
    restored.release(&settled).unwrap();
    assert!(restored.get("stock-test-1").is_none());
    let mut missing_history = settled.clone();
    missing_history.fills.remove(0);
    let restored = StockReceipts::default();
    restored.track(missing_history).unwrap();
    assert!(!restored.get("stock-test-1").unwrap().receipt_complete());
    restored.apply(&frame(1, false).to_string());
    assert!(restored.get("stock-test-1").unwrap().receipt_complete());
}

#[test]
fn stock_receipts_native_fees_unknowns_precision_and_conflicts() {
    let cache = StockReceipts::default();
    cache.track(pending()).unwrap();
    let mut first = frame(1, false);
    first["data"][0].as_object_mut().unwrap().remove("fees");
    first["data"][0]["fee_usd_equiv"] = json!(999);
    cache.apply(&first.to_string());
    cache.apply(&frame(2, true).to_string());
    assert!(!cache.get("stock-test-1").unwrap().receipt_complete());
    first["data"][0]["fees"] =
        serde_json::from_str(r#"[{"asset":"USD","qty":0},{"asset":"BTC","qty":-1e-20}]"#).unwrap();
    cache.apply(&first.to_string());
    let settled = cache.get("stock-test-1").unwrap();
    assert!(settled.receipt_complete());
    assert!(settled.cash_settlement().is_none());
    let fees = settled.fills[0].fees.as_ref().unwrap();
    assert_eq!(fees[0].asset, "BTC");
    assert_eq!(fees[0].quantity, "-0.00000000000000000001");
    assert_eq!(fees[1].quantity, "0");
    first["data"][0]["fees"][0]["qty"] = json!(1);
    cache.apply(&first.to_string());
    let conflict = cache.get("stock-test-1").unwrap();
    assert!(conflict.evidence_conflict);
    assert!(!conflict.receipt_complete());
    assert_eq!(conflict.fills, settled.fills);
    for invalid in [
        "1e-29",
        "1.12345678901234567890123456789",
        "1.12345678901234567890123456789e0",
        "NaN",
    ] {
        assert!(stock_exact_decimal(invalid).is_err(), "{invalid}");
    }
    let mut precise = pending();
    let mut one = frame(1, true)["data"][0].clone();
    one["last_qty"] = json!("1");
    one["last_price"] = json!("600.6");
    one["cost"] = json!("600.6");
    one["fees"] = json!([{"asset":"USD","qty":"0.0000000000000000000000000001"}]);
    precise.apply(parse_execution(&one).unwrap()).unwrap();
    assert!(precise.receipt_complete());
    assert!(
        precise.cash_settlement().is_none(),
        "must not round away tiny fees"
    );
}

#[test]
fn stock_receipts_partial_cancel_identity_and_bounded_tracking() {
    let cache = StockReceipts::default();
    cache.track(pending()).unwrap();
    cache.apply(&frame(1, false).to_string());
    let mut cancel = frame(2, true);
    let row = &mut cancel["data"][0];
    row["exec_type"] = json!("canceled");
    row["order_status"] = json!("canceled");
    row["cum_qty"] = json!("0.4");
    row["cum_cost"] = json!("240");
    cache.apply(&cancel.to_string());
    let cancelled = cache.get("stock-test-1").unwrap();
    assert_eq!(cancelled.phase, StockCexOrderPhase::Cancelled);
    assert_eq!(cancelled.fills.len(), 1, "terminal event is not a new fill");
    assert!(cancelled.receipt_complete());
    assert_eq!(cancelled.cash_settlement().unwrap().quote_change, "239.76");
    assert!(cache.get("other-order").is_none());
    let mut mismatch = frame(1, false);
    mismatch["data"][0]["symbol"] = json!("SNDKx/USD");
    cache.apply(&mismatch.to_string());
    assert!(cache.get("stock-test-1").unwrap().evidence_conflict);
    let capacity = StockReceipts::default();
    for i in 0..MAX_TRACKED {
        let mut r = pending();
        r.client_order_id = format!("stock-{i}");
        capacity.track(r).unwrap();
    }
    let mut extra = pending();
    extra.client_order_id = "stock-extra".into();
    assert!(capacity.track(extra).is_err());
    assert!(capacity.get("stock-0").is_some());
    assert!(capacity.release(&capacity.get("stock-0").unwrap()).is_err());
}
