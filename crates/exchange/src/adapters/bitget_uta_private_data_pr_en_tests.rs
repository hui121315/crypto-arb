use super::*;

#[test]
fn official_account_fixture_projects_complete_uta_summary() {
    let fixture = include_str!("../../fixtures/bitget/uta_account_assets.json");
    let response: crate::adapters::bitget_response::BitgetObjectResponse<UtaAccountAssetsPayload> =
        serde_json::from_str(fixture).expect("account response");
    let payload = response.into_result("account assets").expect("payload");
    let summary = parse_account_summary(&payload, 1_746_687_063_471).expect("summary");

    assert_eq!(summary.account_type, "uta");
    assert_eq!(summary.total_equity_usd, 11.13919278);
    assert_eq!(summary.total_available_balance_usd, 6.19299777);
    assert_eq!(summary.total_initial_margin_usd, 0.0);
    assert_eq!(summary.total_maintenance_margin_usd, 0.0);
    assert_eq!(summary.account_im_rate, 0.0);
    assert_eq!(summary.account_mm_rate, 0.0);
    assert_eq!(summary.observed_at_ms, 1_746_687_063_471);
    assert!(summary.problem.is_none());
}

#[test]
fn account_summary_rejects_missing_or_non_finite_risk_fields() {
    let fixture = include_str!("../../fixtures/bitget/uta_account_assets.json");
    let mut value: serde_json::Value = serde_json::from_str(fixture).expect("account fixture");
    value["data"].as_object_mut().expect("data").remove("imr");
    assert!(serde_json::from_value::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaAccountAssetsPayload>,
    >(value)
    .is_err());

    let response: crate::adapters::bitget_response::BitgetObjectResponse<UtaAccountAssetsPayload> =
        serde_json::from_str(fixture).expect("account response");
    let mut payload = response.into_result("account assets").expect("payload");
    payload.margin_ratio = "NaN".to_owned();
    assert!(parse_account_summary(&payload, 1).is_err());
}
