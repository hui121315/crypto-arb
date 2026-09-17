use super::super::*;
use super::*;

pub(super) use super::cost_fixtures::*;

pub(super) fn run(id: &str, updated_at_ms: i64) -> ExecutionRun {
    ExecutionRun {
        run_id: id.to_owned(),
        ticket_id: format!("ticket-{id}"),
        opportunity_id: format!("opp-{id}"),
        state: ExecutionRunState::Previewed,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "seed".to_owned(),
        created_at_ms: updated_at_ms,
        updated_at_ms,
    }
}

pub(super) fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Created,
        target_quantity: 0.0,
        filled_quantity: None,
        target_notional_usd: 0.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

pub(super) fn leg_with_order(role: HedgeLegRole, order_id: &str) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: vec![order_id.to_owned()],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Submitted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

pub(super) fn assert_close_option(actual: Option<f64>, expected: f64) -> Result<(), &'static str> {
    let actual = actual.ok_or("missing actual value")?;
    if (actual - expected).abs() < 1e-9 {
        Ok(())
    } else {
        Err("actual value differs from expected")
    }
}

pub(super) fn assert_no_funding_cost(run: &ExecutionRun) -> Result<(), &'static str> {
    let cost = run.cost_reconciliation.as_ref().ok_or("cost missing")?;
    assert_eq!(cost.actual_funding_usd, None);
    assert!(cost.funding_event_ids.is_empty());
    Ok(())
}

pub(super) fn set_funding_quality(
    event: &mut ExecutionLedgerEvent,
    quality: ExecutionLedgerQuality,
) -> Result<(), &'static str> {
    match &mut event.payload {
        ExecutionLedgerPayload::FundingPayment(payment) => {
            payment.quality = quality;
            Ok(())
        }
        _ => Err("expected funding payment payload"),
    }
}

pub(super) fn set_funding_currency(
    event: &mut ExecutionLedgerEvent,
    currency: &str,
) -> Result<(), &'static str> {
    match &mut event.payload {
        ExecutionLedgerPayload::FundingPayment(payment) => {
            payment.currency = currency.to_owned();
            Ok(())
        }
        _ => Err("expected funding payment payload"),
    }
}

pub(super) fn filled_record(id: &str, fee: f64) -> OrderRecord {
    OrderRecord {
        intent: intent(id),
        state: LiveOrderState::Filled,
        risk: None,
        identity: Default::default(),
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: Some(format!("ex-{id}")),
        message: None,
        filled_quantity: Some(1.0),
        filled_price: Some(100.0),
        filled_fee: Some(fee),
        updated_at_ms: 3,
    }
}

pub(super) fn failed_record(id: &str) -> OrderRecord {
    OrderRecord {
        intent: intent(id),
        state: LiveOrderState::Failed,
        risk: None,
        identity: Default::default(),
        last_update_source: OrderUpdateSource::OrderQuery,
        exchange_order_id: Some(format!("ex-{id}")),
        message: Some("rejected".to_owned()),
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 3,
    }
}

pub(super) fn ledger_fill_event_row(
    run: &ExecutionRun,
    role: HedgeLegRole,
    exchange_order_id: &str,
    quantity: f64,
    quote_value: f64,
    fee: Option<f64>,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("fill_event:{exchange_order_id}:{role:?}"),
        event_type: ExecutionLedgerEventType::FillEvent,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: Some(run.run_id.clone()),
            ticket_id: Some(run.ticket_id.clone()),
            leg_role: Some(role),
            reduce_only: None,
            exchange: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: OrderSide::Buy,
            identity: VenueOrderIdentity {
                internal_order_id: format!("internal-{exchange_order_id}"),
                public_client_order_id: format!("client-{exchange_order_id}"),
                venue_client_order_id: None,
                exchange_order_id: Some(exchange_order_id.to_owned()),
                product: shared_types::FeeProduct::Perp,
                client_order_id_policy: None,
                transport_metadata: Default::default(),
            },
        },
        payload: ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
            quantity,
            average_price: 100.0,
            quote_value,
            quality: ExecutionLedgerQuality::Actual,
            confidence: shared_types::ExecutionFillConfidence::VenueFill,
            fee: fee.map(|amount| FeeLedgerSnapshot {
                amount,
                currency: Some("USDC".to_owned()),
                quality: ExecutionLedgerQuality::Actual,
            }),
        }),
        occurred_at_ms: 10,
        captured_at_ms: 11,
    }
}

pub(super) fn ledger_state_event_row(
    run: &ExecutionRun,
    role: HedgeLegRole,
    exchange_order_id: &str,
    event_type: ExecutionLedgerEventType,
    state: LiveOrderState,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("state:{exchange_order_id}:{role:?}:{state:?}"),
        event_type,
        source: OrderUpdateSource::OrderQuery,
        order: ExecutionLedgerOrderRef {
            run_id: Some(run.run_id.clone()),
            ticket_id: Some(run.ticket_id.clone()),
            leg_role: Some(role),
            reduce_only: None,
            exchange: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: OrderSide::Buy,
            identity: VenueOrderIdentity {
                internal_order_id: format!("internal-{exchange_order_id}"),
                public_client_order_id: format!("client-{exchange_order_id}"),
                venue_client_order_id: None,
                exchange_order_id: Some(exchange_order_id.to_owned()),
                product: shared_types::FeeProduct::Perp,
                client_order_id_policy: None,
                transport_metadata: Default::default(),
            },
        },
        payload: ExecutionLedgerPayload::OrderState {
            state,
            message: None,
        },
        occurred_at_ms: 10,
        captured_at_ms: 11,
    }
}

pub(super) fn ledger_funding_event_row(
    run: &ExecutionRun,
    role: HedgeLegRole,
    exchange_order_id: &str,
    amount: f64,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("funding_payment:{exchange_order_id}:{role:?}"),
        event_type: ExecutionLedgerEventType::FundingPayment,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: Some(run.run_id.clone()),
            ticket_id: Some(run.ticket_id.clone()),
            leg_role: Some(role),
            reduce_only: None,
            exchange: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: OrderSide::Buy,
            identity: VenueOrderIdentity {
                internal_order_id: format!("internal-{exchange_order_id}"),
                public_client_order_id: format!("client-{exchange_order_id}"),
                venue_client_order_id: None,
                exchange_order_id: Some(exchange_order_id.to_owned()),
                product: shared_types::FeeProduct::Perp,
                client_order_id_policy: None,
                transport_metadata: Default::default(),
            },
        },
        payload: ExecutionLedgerPayload::FundingPayment(FundingPaymentLedgerRecord {
            amount,
            currency: "USDC".to_owned(),
            funding_time_ms: 12,
            quality: ExecutionLedgerQuality::Actual,
        }),
        occurred_at_ms: 12,
        captured_at_ms: 13,
    }
}

pub(super) fn intent(id: &str) -> OrderIntent {
    OrderIntent {
        id: id.to_owned(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::DryRun,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("client-{id}"),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

pub(super) trait ReduceOnlyRecord {
    fn reduce_only(self) -> Self;
}

impl ReduceOnlyRecord for OrderRecord {
    fn reduce_only(mut self) -> Self {
        self.intent.reduce_only = true;
        self
    }
}
