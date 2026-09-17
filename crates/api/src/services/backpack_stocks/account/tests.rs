use super::*;
use serde_json::json;
#[test]
fn backpack_stock_account_balance_updates_are_absolute_ordered_and_missing_assets_stay_unknown() {
    let summary = br#"{"spotMakerFee":"-0.5","spotTakerFee":"10","liquidating":false}"#;
    let balances = br#"{"USDC":{"available":"100","locked":"40","staked":"20"}}"#;
    let evidence = parse(summary, balances, "fixture", 1000).unwrap();
    assert_eq!(evidence.spot_taker_fee_bps, "10");
    assert!(!evidence.balances.contains_key("MU.US"));
    let s = BackpackStocks::new().unwrap();
    *s.account.write() = AccountCache {
        fingerprint: "fixture".into(),
        evidence: Some(evidence),
        ..Default::default()
    };
    let frame = json!({"stream":"account.balanceUpdate","data":{"e":"balanceUpdate","a":"USDC","A":"90","L":"50","S":"20","T":1_100_001,"E":1_100_002}});
    assert!(apply_frame(&s, &frame.to_string(), "fixture", 1101).unwrap());
    assert!(!apply_frame(&s, &frame.to_string(), "fixture", 1102).unwrap());
    let mut old = frame.clone();
    old["data"]["T"] = 1_050_000.into();
    old["data"]["A"] = "999".into();
    assert!(!apply_frame(&s, &old.to_string(), "fixture", 1103).unwrap());
    assert_eq!(
        s.account.read().evidence.as_ref().unwrap().balances["USDC"].available,
        "90"
    );
    assert!(!apply_frame(&s, &frame.to_string(), "other", 1103).unwrap());
    let mut same_time = frame.clone();
    same_time["data"]["A"] = "85".into();
    assert!(apply_frame(&s, &same_time.to_string(), "fixture", 1104).unwrap());
    assert_eq!(
        s.account.read().evidence.as_ref().unwrap().balances["USDC"].available,
        "85"
    );
    assert!(!apply_frame(&s, &same_time.to_string(), "fixture", 1105).unwrap());
    s.invalidate_account();
    assert!(s.account.read().evidence.is_none());
    assert!(parse(
        summary,
        br#"{"USDC":{"locked":"1","staked":"0"}}"#,
        "fixture",
        1000
    )
    .is_err());
    assert!(parse(
        br#"{"spotMakerFee":"1","liquidating":false}"#,
        balances,
        "fixture",
        1000
    )
    .is_err());
}
