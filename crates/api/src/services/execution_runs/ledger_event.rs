use super::*;

pub(super) fn ledger_fill_event(event: &ExecutionLedgerEvent) -> Option<&FillLedgerSnapshot> {
    if event.event_type != ExecutionLedgerEventType::FillEvent {
        return None;
    }
    match &event.payload {
        ExecutionLedgerPayload::FillSnapshot(fill) => valid_fill(fill),
        _ => None,
    }
}

pub(super) fn ledger_order_state_event(event: &ExecutionLedgerEvent) -> Option<LiveOrderState> {
    if !matches!(
        event.event_type,
        ExecutionLedgerEventType::OrderState | ExecutionLedgerEventType::Cancel
    ) {
        return None;
    }
    match event.payload {
        ExecutionLedgerPayload::OrderState { state, .. } => Some(state),
        _ => None,
    }
}

pub(super) fn ledger_funding_payment_event(
    event: &ExecutionLedgerEvent,
) -> Option<&FundingPaymentLedgerRecord> {
    if event.event_type != ExecutionLedgerEventType::FundingPayment {
        return None;
    }
    match &event.payload {
        ExecutionLedgerPayload::FundingPayment(payment) => funding_payment_usd_amount(payment)
            .is_some()
            .then_some(payment),
        _ => None,
    }
}

pub(super) fn ledger_slippage_event(event: &ExecutionLedgerEvent) -> Option<&SlippageLedgerRecord> {
    if event.event_type != ExecutionLedgerEventType::Slippage {
        return None;
    }
    match &event.payload {
        ExecutionLedgerPayload::Slippage(slippage) => (slippage.quality
            == shared_types::ExecutionLedgerQuality::Actual
            && slippage.amount_usd.is_finite())
        .then_some(slippage),
        _ => None,
    }
}

pub(super) fn apply_ledger_event_update(
    run: &mut ExecutionRun,
    event: &ExecutionLedgerEvent,
) -> bool {
    if let Some(fill) = ledger_fill_event(event) {
        return apply_ledger_fill_update(run, event, fill);
    }
    if let Some(slippage) = ledger_slippage_event(event) {
        return apply_ledger_slippage_update(run, event, slippage);
    }
    if let Some(state) = ledger_order_state_event(event) {
        return apply_ledger_order_state_update(run, event, state);
    }
    if let Some(payment) = ledger_funding_payment_event(event) {
        return apply_ledger_funding_payment_update(run, event, payment);
    }
    false
}

pub(super) fn valid_fill(fill: &FillLedgerSnapshot) -> Option<&FillLedgerSnapshot> {
    (fill.quantity.is_finite()
        && fill.quantity > 0.0
        && fill.quote_value.is_finite()
        && fill.quote_value >= 0.0)
        .then_some(fill)
}

pub(super) fn apply_ledger_order_state_update(
    run: &mut ExecutionRun,
    event: &ExecutionLedgerEvent,
    state: LiveOrderState,
) -> bool {
    if event.order.reduce_only == Some(true) {
        return apply_unwind_state_update(run, event, state);
    }
    let run_match = LedgerRunMatch::from_run(run);
    let long_update = apply_ledger_state_to_leg(&run_match, &mut run.long_leg, event, state);
    let short_update = apply_ledger_state_to_leg(&run_match, &mut run.short_leg, event, state);
    if !long_update.matched && !short_update.matched {
        return false;
    }
    refresh_cost_reconciliation(run);
    update_exposure(run);
    if let Some(problem) = long_update.problem.or(short_update.problem) {
        apply_valuation_problem(run, problem);
    } else {
        update_state(run);
        run.status_reason = status_reason(run.state).to_owned();
    }
    true
}

fn apply_unwind_state_update(
    run: &mut ExecutionRun,
    event: &ExecutionLedgerEvent,
    state: LiveOrderState,
) -> bool {
    let run_match = LedgerRunMatch::from_run(run);
    if !unwind_event_matches_run(&run_match, run, event) {
        return false;
    }
    if state == LiveOrderState::Filled {
        run.state = ExecutionRunState::Closed;
        run.net_exposure_usd = 0.0;
        run.recovery_action = None;
        run.unwind_problem = None;
        run.status_reason = "补偿单成交，裸露已关闭".to_owned();
    } else if leg_failed(state) {
        run.state = ExecutionRunState::UnwindRequired;
        run.recovery_action = Some(RecoveryAction::ManualReview);
        run.status_reason = "补偿单终态失败，需要人工复核".to_owned();
    } else {
        run.state = ExecutionRunState::Unwinding;
        run.unwind_problem = None;
        run.status_reason = "补偿单状态已回填".to_owned();
    }
    refresh_cost_reconciliation(run);
    true
}

pub(super) fn apply_ledger_fill_update(
    run: &mut ExecutionRun,
    event: &ExecutionLedgerEvent,
    fill: &FillLedgerSnapshot,
) -> bool {
    if event.order.reduce_only == Some(true) {
        return apply_unwind_fill_cost_update(run, event, fill);
    }
    let run_match = LedgerRunMatch::from_run(run);
    let long_updated = apply_ledger_fill_to_leg(&run_match, &mut run.long_leg, event, fill);
    let short_updated = apply_ledger_fill_to_leg(&run_match, &mut run.short_leg, event, fill);
    if !long_updated && !short_updated {
        return false;
    }
    refresh_cost_reconciliation(run);
    update_exposure(run);
    update_state(run);
    run.status_reason = status_reason(run.state).to_owned();
    true
}

pub(super) fn apply_ledger_slippage_update(
    run: &mut ExecutionRun,
    event: &ExecutionLedgerEvent,
    slippage: &SlippageLedgerRecord,
) -> bool {
    if event.order.reduce_only != Some(true) {
        return false;
    }
    let run_match = LedgerRunMatch::from_run(run);
    if !unwind_event_matches_run(&run_match, run, event) {
        return false;
    }
    let Some(cost) = run.cost_reconciliation.as_mut() else {
        return false;
    };
    if !push_unwind_event_id(cost, &event.event_id) {
        return false;
    }
    cost.actual_unwind_slippage_usd =
        Some(cost.actual_unwind_slippage_usd.unwrap_or(0.0) + slippage.amount_usd);
    refresh_cost_reconciliation(run);
    true
}

fn apply_unwind_fill_cost_update(
    run: &mut ExecutionRun,
    event: &ExecutionLedgerEvent,
    fill: &FillLedgerSnapshot,
) -> bool {
    let run_match = LedgerRunMatch::from_run(run);
    if !unwind_event_matches_run(&run_match, run, event) {
        return false;
    }
    let Some(cost) = run.cost_reconciliation.as_mut() else {
        return false;
    };
    if !push_unwind_event_id(cost, &event.event_id) {
        return false;
    }
    if let Some(fee) = fill.fee.as_ref().filter(|fee| fee.amount.is_finite()) {
        cost.actual_unwind_fee_usd = Some(cost.actual_unwind_fee_usd.unwrap_or(0.0) + fee.amount);
    }
    refresh_cost_reconciliation(run);
    true
}

pub(super) fn apply_ledger_funding_payment_update(
    run: &mut ExecutionRun,
    event: &ExecutionLedgerEvent,
    payment: &FundingPaymentLedgerRecord,
) -> bool {
    let run_match = LedgerRunMatch::from_run(run);
    if !funding_event_matches_run(&run_match, run, event) {
        return false;
    }
    let Some(amount_usd) = funding_payment_usd_amount(payment) else {
        return false;
    };
    let Some(cost) = run.cost_reconciliation.as_mut() else {
        return false;
    };
    if cost
        .funding_event_ids
        .iter()
        .any(|id| id == &event.event_id)
    {
        return false;
    }
    cost.funding_event_ids.push(event.event_id.clone());
    cost.actual_funding_usd = Some(cost.actual_funding_usd.unwrap_or(0.0) + amount_usd);
    refresh_cost_reconciliation(run);
    true
}

fn unwind_event_matches_run(
    run: &LedgerRunMatch,
    execution_run: &ExecutionRun,
    event: &ExecutionLedgerEvent,
) -> bool {
    funding_event_matches_leg(run, &execution_run.long_leg, event)
        || funding_event_matches_leg(run, &execution_run.short_leg, event)
}

fn push_unwind_event_id(
    cost: &mut shared_types::ExecutionCostReconciliation,
    event_id: &str,
) -> bool {
    if cost.unwind_event_ids.iter().any(|id| id == event_id) {
        return false;
    }
    cost.unwind_event_ids.push(event_id.to_owned());
    true
}

pub(super) fn funding_payment_usd_amount(payment: &FundingPaymentLedgerRecord) -> Option<f64> {
    (payment.quality == shared_types::ExecutionLedgerQuality::Actual
        && payment.amount.is_finite()
        && is_usd_settlement_currency(&payment.currency))
    .then_some(payment.amount)
}

pub(super) fn is_usd_settlement_currency(currency: &str) -> bool {
    let currency = currency.trim();
    currency.eq_ignore_ascii_case("USD")
        || currency.eq_ignore_ascii_case("USDC")
        || currency.eq_ignore_ascii_case("USDT")
}
