use super::{identity_contract_fields, identity_evidence_constraints};

pub(super) fn bitget_identity_constraints(
    canonical_symbol: &str,
    native_symbol: &str,
    settle_asset: &str,
) -> Vec<String> {
    let mut constraints = identity_contract_fields(canonical_symbol, native_symbol, settle_asset);
    constraints.extend(identity_evidence_constraints(
        "metadata",
        "bitget-uta-v3-instrument-identity",
        "/api/v3/market/instruments official fixture registry",
        "bitget/uta_instruments_identity_matrix.json",
        "official_identity_matrix_covers_usdt_usdc_coin_and_reality_boundaries",
    ));
    constraints.extend(identity_evidence_constraints(
        "user_stream",
        "bitget-uta-v3-private-order-stream",
        "Bitget UTA V3 private order channel",
        "bitget/uta_ws_order_filled.json",
        "order_and_fill_fixtures_preserve_terminal_finality_and_fees",
    ));
    constraints.extend(identity_evidence_constraints(
        "order_finality",
        "bitget-uta-v3-private-order-finality",
        "private order channel with signed REST order fallback",
        "bitget/uta_ws_order_filled.json",
        "bitget_private_fill_and_order_finality_project_once",
    ));
    constraints.extend(identity_evidence_constraints(
        "fee",
        "bitget-uta-v3-private-fill-fee",
        "Bitget UTA V3 private fill channel",
        "bitget/uta_ws_fill.json",
        "bitget_private_fill_and_order_finality_project_once",
    ));
    constraints
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{FeeProduct, OrderIdentityEvidenceKind, OrderIdentityPlan};

    fn bitget_plan(constraints: Vec<String>) -> OrderIdentityPlan {
        let mut policy = exchange::client_order_id_policy("bitget", "xl-order-1");
        policy.constraints.extend(constraints);
        OrderIdentityPlan::from_compile_contract("bitget", "BTC", FeeProduct::Perp, &policy)
    }

    #[test]
    fn bitget_identity_constraints_bind_native_finality_and_fee_fixtures() {
        let plan = bitget_plan(bitget_identity_constraints("BTC", "BTCUSDT", "USDT"));

        assert!(plan.is_execution_ready(), "{:?}", plan.blockers);
        assert_eq!(plan.native_symbol.as_deref(), Some("BTCUSDT"));
        for kind in [
            OrderIdentityEvidenceKind::Metadata,
            OrderIdentityEvidenceKind::UserStream,
            OrderIdentityEvidenceKind::OrderFinality,
            OrderIdentityEvidenceKind::Fee,
        ] {
            assert!(plan
                .evidence_for(kind)
                .is_some_and(shared_types::OrderIdentityEvidence::is_verified));
        }
        assert!(plan
            .evidence_for(OrderIdentityEvidenceKind::OrderFinality)
            .is_some_and(|evidence| {
                evidence.fixture_id.as_deref() == Some("bitget/uta_ws_order_filled.json")
                    && evidence.parser_test.as_deref()
                        == Some("bitget_private_fill_and_order_finality_project_once")
            }));
        assert!(plan
            .evidence_for(OrderIdentityEvidenceKind::Fee)
            .is_some_and(|evidence| {
                evidence.fixture_id.as_deref() == Some("bitget/uta_ws_fill.json")
                    && evidence.parser_test.as_deref()
                        == Some("bitget_private_fill_and_order_finality_project_once")
            }));
    }

    #[test]
    fn bitget_identity_plan_fails_closed_without_private_finality_evidence() {
        let constraints = bitget_identity_constraints("BTC", "BTCUSDT", "USDT")
            .into_iter()
            .filter(|value| !value.contains("identity.evidence.order_finality"))
            .collect();
        let plan = bitget_plan(constraints);

        assert!(plan
            .blockers
            .contains(&"ORDER_IDENTITY_ORDER_FINALITY_EVIDENCE_UNAVAILABLE".to_owned()));
    }
}
