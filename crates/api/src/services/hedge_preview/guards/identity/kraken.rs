use super::{identity_contract_fields, identity_evidence_constraints};

pub(super) fn kraken_identity_constraints(
    canonical_symbol: &str,
    native_symbol: &str,
    settle_asset: &str,
) -> Vec<String> {
    let mut constraints = identity_contract_fields(canonical_symbol, native_symbol, settle_asset);
    constraints.extend(identity_evidence_constraints(
        "metadata",
        "kraken-futures-instrument-identity",
        "Kraken Derivatives instruments registry",
        "kraken/futures_instruments_pf_xbtusd.json",
        "instruments_authorize_linear_perp_and_keep_inverse_observation_only",
    ));
    constraints.extend(identity_evidence_constraints(
        "user_stream",
        "kraken-futures-open-orders-stream",
        "Kraken Futures open_orders private feed",
        "kraken/futures_open_orders_snapshot.json",
        "parses_official_orders_fills_positions_and_balances",
    ));
    constraints.extend(identity_evidence_constraints(
        "order_finality",
        "kraken-futures-fill-finality",
        "Kraken Futures fills feed with bounded order status fallback",
        "kraken/futures_fills_snapshot.json",
        "parses_official_orders_fills_positions_and_balances",
    ));
    constraints.extend(identity_evidence_constraints(
        "fee",
        "kraken-futures-fill-fee",
        "Kraken Futures fills fee_paid and fee_currency",
        "kraken/futures_fills_snapshot.json",
        "parses_official_orders_fills_positions_and_balances",
    ));
    constraints
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{FeeProduct, OrderIdentityEvidenceKind, OrderIdentityPlan};

    #[test]
    fn kraken_identity_contract_is_execution_ready() {
        let mut policy = exchange::client_order_id_policy("kraken", "xl-order-1");
        policy
            .constraints
            .extend(kraken_identity_constraints("BTC", "PF_XBTUSD", "USD"));
        let plan =
            OrderIdentityPlan::from_compile_contract("kraken", "BTC", FeeProduct::Perp, &policy);

        assert!(plan.is_execution_ready(), "{:?}", plan.blockers);
        assert_eq!(plan.native_symbol.as_deref(), Some("PF_XBTUSD"));
        assert!(plan
            .evidence_for(OrderIdentityEvidenceKind::OrderFinality)
            .is_some_and(|evidence| evidence.fixture_id.as_deref()
                == Some("kraken/futures_fills_snapshot.json")));
    }
}
