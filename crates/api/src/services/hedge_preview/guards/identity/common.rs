pub(super) fn identity_contract_fields(
    canonical_symbol: &str,
    native_symbol: &str,
    settle_asset: &str,
) -> Vec<String> {
    vec![
        format!("identity.canonical_symbol={canonical_symbol}"),
        format!("identity.native_symbol={native_symbol}"),
        format!("identity.settle_asset={settle_asset}"),
        format!("identity.quote_asset={settle_asset}"),
        "identity.product=perp".to_owned(),
        "identity.exchange_order_id_finality_source=private_user_stream_with_rest_fallback"
            .to_owned(),
    ]
}

pub(super) fn identity_evidence_constraints(
    kind: &str,
    evidence_id: &str,
    source: &str,
    fixture_id: &str,
    parser_test: &str,
) -> Vec<String> {
    vec![
        format!("identity.evidence.{kind}.status=verified"),
        format!("identity.evidence.{kind}.evidence_id={evidence_id}"),
        format!("identity.evidence.{kind}.source={source}"),
        format!("identity.evidence.{kind}.fixture_id={fixture_id}"),
        format!("identity.evidence.{kind}.parser_test={parser_test}"),
    ]
}
