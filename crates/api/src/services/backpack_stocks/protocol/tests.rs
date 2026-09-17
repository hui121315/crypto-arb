use super::*;
use serde_json::json;

pub(crate) fn securities() -> Vec<u8> {
    serde_json::to_vec(&json!([
        {"asset":"AAPL.US","name":"Apple Inc.","cusip":"037833100","sessions":[{"name":"US_EQUITIES_REGULAR","minQuantity":"0.01","maxQuantity":"10000","stepSize":"0.00001"}]},
        {"asset":"BRK.B.US","name":"Berkshire Hathaway Class B","cusip":null,"sessions":[]}
    ])).unwrap()
}
pub(crate) fn markets() -> Vec<u8> {
    serde_json::to_vec(&json!([
        {"symbol":"AAPL.US_USDC","baseSymbol":"AAPL.US","quoteSymbol":"USDC","marketType":"SPOT","rwaMarketType":"STOCK","orderBookState":"Open","filters":{"price":{"tickSize":"0.01"},"quantity":{"minQuantity":"0.01","stepSize":"0.01"}}},
        {"symbol":"AAPL_USDC_PERP","baseSymbol":"AAPL.US","quoteSymbol":"USDC","marketType":"PERP","rwaMarketType":"STOCK","orderBookState":"Open","filters":{"price":{"tickSize":"0.01"},"quantity":{"minQuantity":"0.01","stepSize":"0.01"}}}
    ])).unwrap()
}
pub(crate) fn assets() -> Vec<u8> {
    serde_json::to_vec(&json!([{"symbol":"AAPL.US","tokens":[{"blockchain":"Solana","contractAddress":"AAPLEDt8RpzPgXyhvFzkMBofvFSQw9gpeMCoUdPdLnB8","nativeDecimals":6,"depositEnabled":false,"withdrawEnabled":false,"minimumDeposit":"0.0016","minimumWithdrawal":"0.0032","maximumWithdrawal":null,"withdrawalFee":"0.0016"}]}])).unwrap()
}

#[test]
fn backpack_stock_catalog_keeps_rfq_only_and_native_contract_identity() {
    let c = catalog(&securities(), &markets(), 100).unwrap();
    assert_eq!(c.rows.len(), 2);
    assert_eq!(c.rows[0].order_books.len(), 1);
    assert!(c.rows[1].order_books.is_empty());
    assert_eq!(c.rows[1].ticker, "BRK.B");
    assert!(c.rows[1].cusip.is_none());
    assert!(streams(&c.rows[1]).contains("stockPrice.BRK.B"));
    assert_eq!(c.rows[1].rfq_symbol, "BRK.B.US_USDC_RFQ");
    let token = tokens(&assets(), "AAPL.US").unwrap().remove(0);
    assert_eq!(token.deposit_enabled, Some(false));
    assert_eq!(token.native_decimals, Some(6));
    assert_eq!(token.withdrawal_fee.as_deref(), Some("0.0016"));
    assert!(tokens(&assets(), "AAPL").unwrap().is_empty());
    let mut nullable: serde_json::Value = serde_json::from_slice(&assets()).unwrap();
    nullable[0]["tokens"][0]["contractAddress"] = serde_json::Value::Null;
    nullable[0]["tokens"][0]["nativeDecimals"] = serde_json::Value::Null;
    let nullable = tokens(&serde_json::to_vec(&nullable).unwrap(), "AAPL.US").unwrap();
    assert!(nullable[0].contract_address.is_none());
    assert!(nullable[0].native_decimals.is_none());
}

pub(crate) fn frame(now: i64, reference: bool) -> String {
    if reference {
        json!({"stream":"stockPrice.AAPL","data":{"e":"stockPrice","symbol":"AAPL","bid":null,"ask":null,"mid":"207.2","timestamp":now,"session":null}}).to_string()
    } else {
        json!({"stream":"bookTicker.AAPL.US_USDC","data":{"e":"bookTicker","s":"AAPL.US_USDC","a":"207.3","A":"1.25","b":"207.1","B":"2.5","u":"111063070525358080","T":now*1000,"E":now*1000}}).to_string()
    }
}

#[test]
fn backpack_stock_ws_distinguishes_units_null_quotes_duplicates_and_identity() {
    let security = catalog(&securities(), &markets(), 100)
        .unwrap()
        .rows
        .remove(0);
    let mut snapshot = StockMarketSnapshot {
        security: Some(security),
        ..Default::default()
    };
    assert!(apply(&mut snapshot, &frame(1000, false), 1050).unwrap());
    assert_eq!(snapshot.books[0].source_at_ms, 1000);
    assert!(apply(&mut snapshot, &frame(1000, true), 1050).unwrap());
    assert_eq!(snapshot.reference.as_ref().unwrap().source_at_ms, 1000);
    assert!(snapshot.reference.as_ref().unwrap().bid.is_none());
    assert_eq!(snapshot.books[0].bid.as_deref(), Some("207.1"));
    assert!(!apply(&mut snapshot, &frame(1000, false), 1060).unwrap());
    assert!(!apply(&mut snapshot, &frame(999, true), 1060).unwrap());
    let mut empty: serde_json::Value = serde_json::from_str(&frame(1060, false)).unwrap();
    empty["data"]["a"] = serde_json::Value::Null;
    empty["data"]["A"] = serde_json::Value::Null;
    empty["data"]["u"] = json!("111063070525358081");
    assert!(apply(&mut snapshot, &empty.to_string(), 1060).unwrap());
    assert!(snapshot.books[0].ask.is_none());
    empty["data"]["u"] = json!(111063070525358082u64);
    assert!(apply(&mut snapshot, &empty.to_string(), 1060).unwrap());
    assert_eq!(snapshot.books[0].update_id, 111063070525358082);
    let before = snapshot.clone();
    empty["data"]["s"] = json!("NVDA.US_USDC");
    assert!(apply(&mut snapshot, &empty.to_string(), 1060).is_err());
    assert_eq!(snapshot.books, before.books);
    assert_eq!(snapshot.reference, before.reference);
    assert_eq!(snapshot.problem.as_deref(), Some("股票盘口市场身份不符"));
    assert!(apply(&mut snapshot, &frame(5000, true), 1060).is_err());
}

#[test]
fn backpack_stock_reference_health_cannot_invalidate_or_repair_native_books() {
    let mut snapshot = StockMarketSnapshot {
        security: Some(
            catalog(&securities(), &markets(), 100)
                .unwrap()
                .rows
                .remove(0),
        ),
        ..Default::default()
    };
    apply(&mut snapshot, &frame(1000, false), 1050).unwrap();
    let book = snapshot.books.clone();
    assert!(apply(&mut snapshot, &frame(5000, true), 1050).is_err());
    assert!(snapshot.reference_problem.is_some());
    assert!(snapshot.problem.is_none());
    assert_eq!(snapshot.books, book);
    apply(&mut snapshot, &frame(1000, true), 1050).unwrap();
    assert!(snapshot.reference_problem.is_none());
    let mut bad: serde_json::Value = serde_json::from_str(&frame(1050, false)).unwrap();
    bad["data"]["a"] = json!("-1");
    assert!(apply(&mut snapshot, &bad.to_string(), 1060).is_err());
    let problem = snapshot.problem.clone();
    apply(&mut snapshot, &frame(1060, true), 1070).unwrap();
    assert_eq!(
        snapshot.problem, problem,
        "external reference must not clear invalid native book"
    );
    bad["data"]["a"] = json!("207.3");
    bad["data"]["u"] = json!("111063070525358081");
    apply(&mut snapshot, &bad.to_string(), 1070).unwrap();
    assert!(snapshot.problem.is_none());
    assert_eq!(snapshot.books[0].update_id, 111063070525358081);
}

#[test]
#[ignore = "Requires captured official public JSON; run explicitly with BACKPACK_PUBLIC_CAPTURE_DIR"]
fn backpack_stock_recorded_public_metadata_is_decodable() {
    // Opt-in verification against the public files captured for this development task.
    let root = std::env::var("BACKPACK_PUBLIC_CAPTURE_DIR").expect("set public capture directory");
    let read =
        |name: &str| std::fs::read(format!("{root}/crossline-backpack-{name}.json")).unwrap();
    let c = catalog(&read("securities"), &read("markets"), 1000).unwrap();
    assert!(c.rows.len() > 100);
    let aapl = c.rows.iter().find(|s| s.asset == "AAPL.US").unwrap();
    assert_eq!(aapl.cusip.as_deref(), Some("037833100"));
    assert!(!tokens(&read("assets"), &aapl.asset).unwrap().is_empty());
    println!(
        "public metadata: {} securities, {} order-book markets",
        c.rows.len(),
        c.rows.iter().map(|s| s.order_books.len()).sum::<usize>()
    );
}
