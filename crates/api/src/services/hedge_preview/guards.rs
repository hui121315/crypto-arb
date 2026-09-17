use super::*;
use shared_types::VenueSymbolCapability;

mod identity;
mod runtime;
mod spot_inventory;
#[cfg(test)]
mod test_helpers;

use identity::attach_order_identity_evidence;
#[cfg(test)]
use identity::{binance_identity_constraints, bybit_identity_constraints};
use runtime::{
    append_instrument_sizing_guard, append_live_operation_health_guard, attach_instrument_contracts,
};
#[cfg(test)]
pub(crate) use test_helpers::{
    account_mode_plan, append_order_capability_guard, available_order_types,
};

fn attach_order_capability_options(
    state: &AppState,
    plans: [&mut shared_types::OrderCompilePlan; 2],
) {
    let trading = state.trading_service();
    for plan in plans {
        let evidence = trading
            .exchange_capabilities(&plan.exchange)
            .and_then(|capabilities| {
                trading
                    .exchange_capability_matrix_for_product(&plan.exchange, plan.product)
                    .map(|matrix| (capabilities, matrix))
            });
        let (capabilities, matrix) = match evidence {
            Ok(evidence) => evidence,
            Err(error) => {
                plan.blockers.push(format!(
                    "{} {} 未配置可用交易路由：{}",
                    plan.exchange, plan.symbol, error
                ));
                continue;
            }
        };
        plan.available_order_types = matrix
            .orders
            .iter()
            .map(|capability| capability.requested_order_type)
            .collect();
        if let Some(order) = matrix.order(plan.requested_order_type) {
            plan.available_time_in_force = order.time_in_force.clone();
        }
        plan.available_margin_modes = matrix.account.order_margin_modes.clone();
        plan.venue_capability = VenueSymbolCapability {
            venue: shared_types::normalized_venue_name(&plan.exchange),
            symbol: plan.symbol.clone(),
            product: plan.product,
            available_order_types: plan.available_order_types.clone(),
            available_time_in_force: plan.available_time_in_force.clone(),
            available_margin_modes: plan.available_margin_modes.clone(),
            market_order_styles: matrix
                .order(plan.requested_order_type)
                .map(|order| order.market_order_styles.clone())
                .unwrap_or_default(),
            supports_reduce_only: capabilities.supports_reduce_only,
            matrix,
            account_mode: None,
            account_mode_error: None,
            source: "live_trading_adapter.exchange_capability_matrix_for_product".to_owned(),
        };
    }
}

pub(super) async fn append_preview_guards(input: PreviewGuardInput<'_>) -> Result<(), AppError> {
    attach_order_capability_options(
        input.state,
        [&mut *input.long_order_plan, &mut *input.short_order_plan],
    );
    attach_instrument_contracts(
        input.state,
        input.mode,
        [
            (
                &mut *input.long_order_plan,
                &mut *input.long_leg,
                input.long_notional,
                input.long_price,
            ),
            (
                &mut *input.short_order_plan,
                &mut *input.short_leg,
                input.short_notional,
                input.short_price,
            ),
        ],
    );
    attach_order_identity_evidence(
        input.state,
        [&mut *input.long_order_plan, &mut *input.short_order_plan],
    );
    append_order_compile_blockers(
        input.ticket,
        &[&*input.long_order_plan, &*input.short_order_plan],
    );
    let (long_preflight, short_preflight) = tokio::join!(
        crate::services::hedge_preflight::collect_live_order_preflight(
            input.state,
            input.mode,
            &*input.long_leg,
            input.long_order_plan,
        ),
        crate::services::hedge_preflight::collect_live_order_preflight(
            input.state,
            input.mode,
            &*input.short_leg,
            input.short_order_plan,
        ),
    );
    if let (Some(long_preflight), Some(short_preflight)) = (long_preflight, short_preflight) {
        for guard in crate::services::hedge_preflight::hedge_live_order_preflight_guards(
            long_preflight,
            short_preflight,
        ) {
            crate::services::hedge_ticket::append_guard(input.ticket, guard);
        }
    }
    append_live_operation_health_guard(
        input.state,
        input.ticket,
        input.mode,
        input.long_order_plan,
        input.short_order_plan,
    );
    append_instrument_sizing_guard(
        input.state,
        input.ticket,
        input.mode,
        [
            (input.long_order_plan, input.long_notional, input.long_price),
            (
                input.short_order_plan,
                input.short_notional,
                input.short_price,
            ),
        ],
    );
    if let Some(guard) = spot_inventory::guard(
        input.state,
        input.mode,
        input.ticket.strategy,
        [input.long_order_plan, input.short_order_plan],
        [&*input.long_leg, &*input.short_leg],
    )
    .await
    {
        crate::services::hedge_ticket::append_guard(input.ticket, guard);
    }
    let margin_guard = crate::services::hedge_margin::preview_margin_guard(
        input.state,
        &[&*input.long_leg, &*input.short_leg],
    )
    .await?;
    crate::services::hedge_ticket::append_guard(input.ticket, margin_guard);
    Ok(())
}

fn append_order_compile_blockers(
    ticket: &mut HedgeTicket,
    plans: &[&shared_types::OrderCompilePlan],
) {
    for plan in plans {
        ticket.blockers.extend(plan.blockers.iter().cloned());
        ticket.blockers.extend(plan.identity_plan().blockers);
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use shared_types::{
        ClientOrderIdDerivation, ClientOrderIdPolicy, FeeProduct, OrderIdentityEvidenceKind,
    };

    fn bybit_plan(constraints: Vec<String>) -> shared_types::OrderIdentityPlan {
        let policy = ClientOrderIdPolicy {
            venue: "bybit".to_owned(),
            venue_family: "bybit".to_owned(),
            venue_field: "orderLinkId".to_owned(),
            public_client_order_id: "public-1".to_owned(),
            venue_client_order_id: Some("public-1".to_owned()),
            derivation: ClientOrderIdDerivation::Identity,
            policy_version: "client-order-id-policy-v1".to_owned(),
            official_format: "1..=36 alphanumeric, dash, underscore".to_owned(),
            max_length: Some(36),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            constraints,
            blockers: Vec::new(),
            official_doc_urls: Vec::new(),
        };
        shared_types::OrderIdentityPlan::from_compile_contract(
            "bybit",
            "BTC",
            FeeProduct::Perp,
            &policy,
        )
    }

    #[test]
    fn binance_identity_constraints_produce_execution_ready_usdc_plan() {
        let policy = ClientOrderIdPolicy {
            venue: "binance".to_owned(),
            venue_family: "binance".to_owned(),
            venue_field: "newClientOrderId/origClientOrderId".to_owned(),
            public_client_order_id: "public-1".to_owned(),
            venue_client_order_id: Some("public-1".to_owned()),
            derivation: ClientOrderIdDerivation::Identity,
            policy_version: "client-order-id-policy-v1".to_owned(),
            official_format: "1..=36 ASCII".to_owned(),
            max_length: Some(36),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            constraints: binance_identity_constraints("BTC", "BTCUSDC", "USDC"),
            blockers: Vec::new(),
            official_doc_urls: Vec::new(),
        };

        let plan = shared_types::OrderIdentityPlan::from_compile_contract(
            "binance",
            "BTC",
            FeeProduct::Perp,
            &policy,
        );

        assert!(plan.is_execution_ready(), "{:?}", plan.blockers);
        assert_eq!(plan.native_symbol.as_deref(), Some("BTCUSDC"));
        assert_eq!(plan.quote_asset.as_deref(), Some("USDC"));
        assert!(plan
            .evidence_for(OrderIdentityEvidenceKind::OrderFinality)
            .is_some_and(shared_types::OrderIdentityEvidence::is_verified));
    }

    #[test]
    fn bybit_identity_constraints_produce_execution_ready_usdc_plan() {
        let plan = bybit_plan(bybit_identity_constraints("BTC", "BTCPERP", "USDC"));

        assert!(plan.is_execution_ready(), "{:?}", plan.blockers);
        assert_eq!(plan.native_symbol.as_deref(), Some("BTCPERP"));
        assert_eq!(plan.quote_asset.as_deref(), Some("USDC"));
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
                evidence.fixture_id.as_deref() == Some("bybit/ws_user_order_filled.json")
                    && evidence.parser_test.as_deref()
                        == Some("bybit_private_fill_and_order_finality_project_once")
            }));
        assert!(plan
            .evidence_for(OrderIdentityEvidenceKind::Fee)
            .is_some_and(|evidence| {
                evidence.fixture_id.as_deref() == Some("bybit/ws_user_execution_fill.json")
                    && evidence.parser_test.as_deref()
                        == Some("bybit_private_fill_and_order_finality_project_once")
            }));
    }

    #[test]
    fn bybit_identity_plan_fails_closed_without_private_finality_evidence() {
        let constraints = bybit_identity_constraints("BTC", "BTCPERP", "USDC")
            .into_iter()
            .filter(|value| !value.contains("identity.evidence.order_finality"))
            .collect();
        let plan = bybit_plan(constraints);

        assert!(plan
            .blockers
            .iter()
            .any(|blocker| blocker == "ORDER_IDENTITY_ORDER_FINALITY_EVIDENCE_UNAVAILABLE"));
    }
}
