use super::{identity_contract_fields, identity_evidence_constraints};

pub(super) fn gate_crossex_identity_constraints(
    canonical_symbol: &str,
    native_symbol: &str,
    settle_asset: &str,
) -> Vec<String> {
    let mut constraints = identity_contract_fields(canonical_symbol, native_symbol, settle_asset);
    constraints.extend(identity_evidence_constraints(
        "metadata",
        "gate-crossex-symbol-route-identity",
        "Gate CrossEx symbols route registry",
        "gate_crossex/symbols_routes.json",
        "official_symbol_specs_are_execution_constructible",
    ));
    constraints.extend(identity_evidence_constraints(
        "user_stream",
        "gate-crossex-private-order-stream",
        "Gate CrossEx private order channel",
        "gate_crossex/private_order_update.json",
        "parses_official_order_asset_position_and_fill_frames",
    ));
    constraints.extend(identity_evidence_constraints(
        "order_finality",
        "gate-crossex-order-finality",
        "private order stream with bounded CrossEx order query fallback",
        "gate_crossex/order_detail.json",
        "parses_bounded_rest_bootstrap_rows",
    ));
    constraints.extend(identity_evidence_constraints(
        "fee",
        "gate-crossex-private-fill-fee",
        "Gate CrossEx private usertrades channel",
        "gate_crossex/private_fill_update.json",
        "parses_official_order_asset_position_and_fill_frames",
    ));
    constraints
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{FeeProduct, OrderIdentityEvidenceKind, OrderIdentityPlan};

    #[test]
    fn crossex_identity_contract_is_execution_ready() {
        let mut policy = exchange::client_order_id_policy("gate_crossex:okx", "xl-order-1");
        policy.constraints.extend(gate_crossex_identity_constraints(
            "BTC",
            "OKX_FUTURE_BTC_USDT",
            "USDT",
        ));
        let plan = OrderIdentityPlan::from_compile_contract(
            "gate_crossex:okx",
            "BTC",
            FeeProduct::Perp,
            &policy,
        );

        assert!(plan.is_execution_ready(), "{:?}", plan.blockers);
        assert_eq!(plan.native_symbol.as_deref(), Some("OKX_FUTURE_BTC_USDT"));
        assert!(plan
            .evidence_for(OrderIdentityEvidenceKind::Fee)
            .is_some_and(|evidence| evidence.fixture_id.as_deref()
                == Some("gate_crossex/private_fill_update.json")));
    }
}
