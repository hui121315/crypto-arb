use super::{identity_contract_fields, identity_evidence_constraints};

pub(super) fn kucoin_identity_constraints(
    canonical_symbol: &str,
    native_symbol: &str,
    settle_asset: &str,
) -> Vec<String> {
    let mut constraints = identity_contract_fields(canonical_symbol, native_symbol, settle_asset);
    constraints.extend(identity_evidence_constraints(
        "metadata",
        "kucoin-futures-native-contract-identity",
        "/api/v1/contracts/active official fixture registry",
        "kucoin/contracts_active_native_matrix.json",
        "official_matrix_maps_usdt_usdc_and_verified_equity_contracts",
    ));
    constraints.extend(identity_evidence_constraints(
        "user_stream",
        "kucoin-classic-private-order-stream",
        "KuCoin Classic Futures tradeOrders private channel",
        "kucoin/classic_ws_trade_orders_match.json",
        "duplicate_match_fixture_keeps_stable_fill_identity",
    ));
    constraints.extend(identity_evidence_constraints(
        "order_finality",
        "kucoin-classic-private-order-finality",
        "Classic tradeOrders with signed REST order fallback",
        "kucoin/classic_ws_trade_orders_filled.json",
        "kucoin_terminal_fill_projects_execution_run_once_after_durable_ack",
    ));
    constraints.extend(identity_evidence_constraints(
        "fee",
        "kucoin-signed-rest-actual-fill-fee",
        "signed GET /api/v1/fills by exchange order id",
        "kucoin/fills_by_order_id.json",
        "kucoin_fills_parse_official_fixture_without_defaulting_fee",
    ));
    constraints
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{FeeProduct, OrderIdentityEvidenceKind, OrderIdentityPlan};

    fn kucoin_plan(constraints: Vec<String>) -> OrderIdentityPlan {
        let mut policy = exchange::client_order_id_policy("kucoin", "xl-order-1");
        policy.constraints.extend(constraints);
        OrderIdentityPlan::from_compile_contract("kucoin", "BTC", FeeProduct::Perp, &policy)
    }

    #[test]
    fn kucoin_identity_constraints_bind_native_finality_and_actual_fee_fixtures() {
        let plan = kucoin_plan(kucoin_identity_constraints("BTC", "XBTUSDTM", "USDT"));

        assert!(plan.is_execution_ready(), "{:?}", plan.blockers);
        assert_eq!(plan.native_symbol.as_deref(), Some("XBTUSDTM"));
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
            .evidence_for(OrderIdentityEvidenceKind::Fee)
            .is_some_and(|evidence| {
                evidence.fixture_id.as_deref() == Some("kucoin/fills_by_order_id.json")
                    && evidence.parser_test.as_deref()
                        == Some("kucoin_fills_parse_official_fixture_without_defaulting_fee")
            }));
    }

    #[test]
    fn kucoin_identity_plan_fails_closed_without_private_finality_evidence() {
        let constraints = kucoin_identity_constraints("BTC", "XBTUSDTM", "USDT")
            .into_iter()
            .filter(|value| !value.contains("identity.evidence.order_finality"))
            .collect();
        let plan = kucoin_plan(constraints);

        assert!(plan
            .blockers
            .contains(&"ORDER_IDENTITY_ORDER_FINALITY_EVIDENCE_UNAVAILABLE".to_owned()));
    }
}
