mod bitget;
mod common;
mod gate_crossex;
mod kraken;
mod kucoin;

use super::*;
use common::{identity_contract_fields, identity_evidence_constraints};

pub(super) fn attach_order_identity_evidence(
    state: &AppState,
    plans: [&mut shared_types::OrderCompilePlan; 2],
) {
    for plan in plans {
        let venue = venue_family(&plan.exchange);
        if !matches!(
            venue,
            "binance" | "bitget" | "bybit" | "gate_crossex" | "kraken" | "kucoin" | "okx"
        ) || plan.product != shared_types::FeeProduct::Perp
        {
            continue;
        }
        let registry = state.instrument_registry();
        let instrument = registry.resolve_hedge_instrument_for_product(
            &plan.exchange,
            &plan.symbol,
            plan.product,
        );
        let Some(instrument) = instrument else {
            continue;
        };
        let Some(quote_asset) = instrument
            .quote_asset
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(settle_asset) = instrument
            .settle_asset
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !quote_asset.eq_ignore_ascii_case(settle_asset) {
            continue;
        }
        let constraints = match venue {
            "binance" => {
                binance_identity_constraints(&plan.symbol, &instrument.native_symbol, quote_asset)
            }
            "bitget" => bitget::bitget_identity_constraints(
                &plan.symbol,
                &instrument.native_symbol,
                quote_asset,
            ),
            "bybit" => {
                bybit_identity_constraints(&plan.symbol, &instrument.native_symbol, quote_asset)
            }
            "gate_crossex" => gate_crossex::gate_crossex_identity_constraints(
                &plan.symbol,
                &instrument.native_symbol,
                quote_asset,
            ),
            "kraken" => kraken::kraken_identity_constraints(
                &plan.symbol,
                &instrument.native_symbol,
                quote_asset,
            ),
            "kucoin" => kucoin::kucoin_identity_constraints(
                &plan.symbol,
                &instrument.native_symbol,
                quote_asset,
            ),
            "okx" => okx_identity_constraints(&plan.symbol, &instrument.native_symbol, quote_asset),
            _ => continue,
        };
        plan.client_order_id_policy.constraints.extend(constraints);
    }
}

pub(super) fn okx_identity_constraints(
    canonical_symbol: &str,
    native_symbol: &str,
    settle_asset: &str,
) -> Vec<String> {
    let mut constraints = identity_contract_fields(canonical_symbol, native_symbol, settle_asset);
    constraints.extend(identity_evidence_constraints(
        "metadata",
        "okx-v5-public-instruments",
        "/api/v5/public/instruments official fixture registry",
        "okx/public_instruments_swap.json",
        "okx_instrument_rule_parses_official_swap_fixture",
    ));
    constraints.extend(identity_evidence_constraints(
        "user_stream",
        "okx-v5-private-orders-stream",
        "OKX V5 private orders channel",
        "okx/ws_user_orders_partial_fill.json",
        "okx_partial_fill_fixture_preserves_identity_fee_and_deduplicates",
    ));
    constraints.extend(identity_evidence_constraints(
        "order_finality",
        "okx-v5-private-orders-finality",
        "private orders channel with signed REST order fallback",
        "okx/ws_user_orders_canceled.json",
        "okx_cancel_fixture_is_final_only_after_private_order_event",
    ));
    constraints.extend(identity_evidence_constraints(
        "fee",
        "okx-v5-private-orders-fill-fee",
        "OKX V5 private orders fillFee/fillFeeCcy",
        "okx/ws_user_orders_partial_fill.json",
        "okx_partial_fill_fixture_preserves_identity_fee_and_deduplicates",
    ));
    constraints
}

pub(super) fn bybit_identity_constraints(
    canonical_symbol: &str,
    native_symbol: &str,
    settle_asset: &str,
) -> Vec<String> {
    let mut constraints = identity_contract_fields(canonical_symbol, native_symbol, settle_asset);
    constraints.extend(identity_evidence_constraints(
        "metadata",
        "bybit-v5-linear-instrument-identity",
        "/v5/market/instruments-info official fixture registry",
        "bybit/instruments_info_linear_identity_matrix.json",
        "bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts",
    ));
    constraints.extend(identity_evidence_constraints(
        "user_stream",
        "bybit-v5-private-order-stream",
        "Bybit V5 private order topic",
        "bybit/ws_user_order_filled.json",
        "parses_official_order_fixture_to_terminal_delta",
    ));
    constraints.extend(identity_evidence_constraints(
        "order_finality",
        "bybit-v5-private-order-finality",
        "private order stream with signed REST order fallback",
        "bybit/ws_user_order_filled.json",
        "bybit_private_fill_and_order_finality_project_once",
    ));
    constraints.extend(identity_evidence_constraints(
        "fee",
        "bybit-v5-private-execution-fee",
        "Bybit V5 private execution topic",
        "bybit/ws_user_execution_fill.json",
        "bybit_private_fill_and_order_finality_project_once",
    ));
    constraints
}

pub(super) fn binance_identity_constraints(
    canonical_symbol: &str,
    native_symbol: &str,
    settle_asset: &str,
) -> Vec<String> {
    let mut constraints = identity_contract_fields(canonical_symbol, native_symbol, settle_asset);
    constraints.extend(identity_evidence_constraints(
        "metadata",
        "binance-usdm-exchange-info",
        "/fapi/v1/exchangeInfo",
        "binance/usdm_exchange_info_usdt_usdc.json",
        "registry_projection_uses_compiled_usdt_and_usdc_specs",
    ));
    constraints.extend(identity_evidence_constraints(
        "user_stream",
        "binance-usdm-order-trade-update",
        "ORDER_TRADE_UPDATE official fixture registry",
        "binance/usdm_order_trade_update_partial_fill.json",
        "parses_order_trade_update_to_order_delta",
    ));
    constraints.extend(identity_evidence_constraints(
        "order_finality",
        "binance-usdm-durable-finality",
        "private user stream with signed REST order fallback",
        "binance/usdm_order_trade_update_filled.json",
        "parses_order_trade_update_filled_fixture_to_terminal_delta",
    ));
    constraints.extend(identity_evidence_constraints(
        "fee",
        "binance-usdm-order-trade-fill-fee",
        "ORDER_TRADE_UPDATE commission asset and amount",
        "binance/usdm_order_trade_update_filled.json",
        "parses_order_trade_update_filled_fixture_to_terminal_delta",
    ));
    constraints
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{FeeProduct, OrderIdentityEvidenceKind, OrderIdentityPlan};

    fn binance_plan() -> OrderIdentityPlan {
        let mut policy = exchange::client_order_id_policy("binance", "xl-order-1");
        policy
            .constraints
            .extend(binance_identity_constraints("BTC", "BTCUSDT", "USDT"));
        OrderIdentityPlan::from_compile_contract("binance", "BTC", FeeProduct::Perp, &policy)
    }

    fn okx_plan() -> OrderIdentityPlan {
        let mut policy = exchange::client_order_id_policy("okx", "xl0123456789abcdefl");
        policy
            .constraints
            .extend(okx_identity_constraints("BTC", "BTC-USDT-SWAP", "USDT"));
        OrderIdentityPlan::from_compile_contract("okx", "BTC", FeeProduct::Perp, &policy)
    }

    #[test]
    fn binance_identity_constraints_bind_terminal_fill_fee_fixture() {
        let plan = binance_plan();

        assert!(plan.is_execution_ready(), "{:?}", plan.blockers);
        for kind in [
            OrderIdentityEvidenceKind::OrderFinality,
            OrderIdentityEvidenceKind::Fee,
        ] {
            let evidence = plan.evidence_for(kind);
            assert!(evidence.is_some(), "missing {kind:?} evidence");
            let Some(evidence) = evidence else {
                continue;
            };
            assert_eq!(
                evidence.fixture_id.as_deref(),
                Some("binance/usdm_order_trade_update_filled.json")
            );
            assert_eq!(
                evidence.parser_test.as_deref(),
                Some("parses_order_trade_update_filled_fixture_to_terminal_delta")
            );
            assert!(evidence.is_verified());
        }
    }

    #[test]
    fn binance_identity_plan_fails_closed_without_private_finality_evidence() {
        let mut policy = exchange::client_order_id_policy("binance", "xl-order-1");
        policy.constraints.extend(
            binance_identity_constraints("BTC", "BTCUSDT", "USDT")
                .into_iter()
                .filter(|value| !value.contains("identity.evidence.order_finality")),
        );
        let plan =
            OrderIdentityPlan::from_compile_contract("binance", "BTC", FeeProduct::Perp, &policy);

        assert!(plan
            .blockers
            .contains(&"ORDER_IDENTITY_ORDER_FINALITY_EVIDENCE_UNAVAILABLE".to_owned()));
    }

    #[test]
    fn okx_identity_constraints_produce_execution_ready_plan() {
        let plan = okx_plan();

        assert!(plan.is_execution_ready(), "{:?}", plan.blockers);
        assert_eq!(plan.native_symbol.as_deref(), Some("BTC-USDT-SWAP"));
        assert_eq!(plan.settle_asset.as_deref(), Some("USDT"));
        assert_eq!(plan.quote_asset.as_deref(), Some("USDT"));
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
    }

    #[test]
    fn okx_identity_plan_fails_closed_without_private_finality_evidence() {
        let mut policy = exchange::client_order_id_policy("okx", "xl0123456789abcdefl");
        policy.constraints.extend(
            okx_identity_constraints("BTC", "BTC-USDT-SWAP", "USDT")
                .into_iter()
                .filter(|value| !value.contains("identity.evidence.order_finality")),
        );
        let plan =
            OrderIdentityPlan::from_compile_contract("okx", "BTC", FeeProduct::Perp, &policy);

        assert!(plan
            .blockers
            .contains(&"ORDER_IDENTITY_ORDER_FINALITY_EVIDENCE_UNAVAILABLE".to_owned()));
    }
}
