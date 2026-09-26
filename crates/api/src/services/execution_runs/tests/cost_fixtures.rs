use super::super::*;
use super::*;

pub(super) fn cost() -> ExecutionCostReconciliation {
    ExecutionCostReconciliation {
        estimated_open_cost_usd: 0.5,
        estimated_close_cost_usd: 0.5,
        estimated_slippage_usd: 0.0,
        estimated_total_cost_usd: 1.0,
        filled_fee_usd: None,
        actual_slippage_usd: None,
        actual_open_cost_usd: None,
        actual_funding_usd: None,
        funding_event_ids: Vec::new(),
        actual_unwind_fee_usd: None,
        actual_unwind_slippage_usd: None,
        actual_unwind_cost_usd: None,
        unwind_event_ids: Vec::new(),
        missing_fields: Vec::new(),
        actual_cost_usd: None,
        cost_delta_usd: None,
    }
}

pub(super) fn ledger_slippage_event_row(
    run: &ExecutionRun,
    role: HedgeLegRole,
    exchange_order_id: &str,
    amount_usd: f64,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("slippage:{exchange_order_id}:{role:?}"),
        event_type: ExecutionLedgerEventType::Slippage,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: Some(run.run_id.clone()),
            ticket_id: Some(run.ticket_id.clone()),
            leg_role: Some(role),
            reduce_only: None,
            exchange: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: OrderSide::Sell,
            identity: VenueOrderIdentity {
                account_scope: None,
                internal_order_id: format!("internal-{exchange_order_id}"),
                public_client_order_id: format!("client-{exchange_order_id}"),
                venue_client_order_id: None,
                exchange_order_id: Some(exchange_order_id.to_owned()),
                product: shared_types::FeeProduct::Perp,
                client_order_id_policy: None,
                transport_metadata: Default::default(),
            },
        },
        payload: ExecutionLedgerPayload::Slippage(shared_types::SlippageLedgerRecord {
            amount_usd,
            reference_price: 100.0,
            fill_price: 100.0 + amount_usd,
            quantity: 1.0,
            quality: ExecutionLedgerQuality::Actual,
        }),
        occurred_at_ms: 12,
        captured_at_ms: 13,
    }
}
