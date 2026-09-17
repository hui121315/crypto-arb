use super::*;
use crate::services::hedge_preflight::InstrumentSizingCheck;
use crate::services::instrument_registry::InstrumentRegistry;

pub(super) fn attach_instrument_contracts(
    state: &AppState,
    mode: ExecutionMode,
    legs: [(
        &mut shared_types::OrderCompilePlan,
        &mut shared_types::OrderIntent,
        f64,
        f64,
    ); 2],
) {
    let registry = state.instrument_registry();
    attach_paired_instrument_contracts(registry, mode, legs);
}

fn attach_paired_instrument_contracts(
    registry: &InstrumentRegistry,
    mode: ExecutionMode,
    legs: [(
        &mut shared_types::OrderCompilePlan,
        &mut shared_types::OrderIntent,
        f64,
        f64,
    ); 2],
) {
    let [(long_plan, long_intent, long_notional, long_price), (short_plan, short_intent, short_notional, short_price)] =
        legs;
    let long_instrument = registry.resolve_hedge_instrument_for_product(
        &long_plan.exchange,
        &long_plan.symbol,
        long_plan.product,
    );
    let short_instrument = registry.resolve_hedge_instrument_for_product(
        &short_plan.exchange,
        &short_plan.symbol,
        short_plan.product,
    );
    let (long_instrument, short_instrument) = match (long_instrument, short_instrument) {
        (Some(long), Some(short)) => (long, short),
        (long, short) => {
            attach_missing_pair_contract(registry, mode, long_plan, long);
            attach_missing_pair_contract(registry, mode, short_plan, short);
            return;
        }
    };
    match shared_types::plan_paired_leg_sizing(
        long_notional,
        &long_instrument,
        long_price,
        short_notional,
        &short_instrument,
        short_price,
    ) {
        Ok(paired) => {
            long_plan.instrument_spec = Some(long_instrument);
            long_plan.sizing_plan = Some(paired.long);
            short_plan.instrument_spec = Some(short_instrument);
            short_plan.sizing_plan = Some(paired.short);
            long_intent.quantity = paired.base_quantity;
            short_intent.quantity = paired.base_quantity;
        }
        Err(block) => {
            attach_blocked_pair_contract(mode, long_plan, long_instrument, block);
            attach_blocked_pair_contract(mode, short_plan, short_instrument, block);
        }
    }
}

fn attach_missing_pair_contract(
    registry: &InstrumentRegistry,
    mode: ExecutionMode,
    plan: &mut shared_types::OrderCompilePlan,
    instrument: Option<shared_types::InstrumentSpec>,
) {
    plan.instrument_spec = instrument;
    plan.sizing_plan = None;
    if mode == ExecutionMode::Live {
        let code = if plan.instrument_spec.is_some() {
            shared_types::SizingBlock::PairQuantityMismatch.code()
        } else if registry.supports_venue(&plan.exchange) {
            shared_types::SizingBlock::SpecMissing.code()
        } else {
            return;
        };
        push_plan_blocker(plan, code);
    }
}

fn attach_blocked_pair_contract(
    mode: ExecutionMode,
    plan: &mut shared_types::OrderCompilePlan,
    instrument: shared_types::InstrumentSpec,
    block: shared_types::SizingBlock,
) {
    if mode == ExecutionMode::Live {
        plan.instrument_spec = Some(instrument);
        plan.sizing_plan = None;
        push_plan_blocker(plan, block.code());
    } else {
        plan.instrument_spec = None;
        plan.sizing_plan = None;
    }
}

fn push_plan_blocker(plan: &mut shared_types::OrderCompilePlan, blocker: &str) {
    let blocker = format!("{} {} {blocker}", plan.exchange, plan.symbol);
    if !plan.blockers.contains(&blocker) {
        plan.blockers.push(blocker);
    }
}

pub(super) fn append_live_operation_health_guard(
    state: &AppState,
    ticket: &mut HedgeTicket,
    mode: ExecutionMode,
    long_order_plan: &shared_types::OrderCompilePlan,
    short_order_plan: &shared_types::OrderCompilePlan,
) {
    if mode != ExecutionMode::Live {
        return;
    }
    let snapshot = crate::services::venue_operation_health::snapshot(state);
    if let Some(guard) = crate::services::hedge_preflight::live_operation_health_guard(
        mode,
        &[long_order_plan, short_order_plan],
        &snapshot.rows,
    ) {
        crate::services::hedge_ticket::append_guard(ticket, guard);
    }
}

pub(super) fn append_instrument_sizing_guard(
    state: &AppState,
    ticket: &mut HedgeTicket,
    mode: ExecutionMode,
    legs: [(&shared_types::OrderCompilePlan, f64, f64); 2],
) {
    if mode != ExecutionMode::Live {
        return;
    }
    let registry = state.instrument_registry();
    let [(long_plan, long_notional, long_price), (short_plan, short_notional, short_price)] = legs;
    let paired = long_plan
        .instrument_spec
        .as_ref()
        .zip(short_plan.instrument_spec.as_ref())
        .ok_or(shared_types::SizingBlock::SpecMissing)
        .and_then(|(long_instrument, short_instrument)| {
            shared_types::plan_paired_leg_sizing(
                long_notional,
                long_instrument,
                long_price,
                short_notional,
                short_instrument,
                short_price,
            )
        });
    let checks = [
        InstrumentSizingCheck::new(
            long_plan,
            registry.supports_venue(&long_plan.exchange),
            paired
                .as_ref()
                .map(|plan| plan.long)
                .map_err(|block| *block),
        ),
        InstrumentSizingCheck::new(
            short_plan,
            registry.supports_venue(&short_plan.exchange),
            paired
                .as_ref()
                .map(|plan| plan.short)
                .map_err(|block| *block),
        ),
    ];
    if let Some(guard) = crate::services::hedge_preflight::instrument_sizing_guard(mode, &checks) {
        crate::services::hedge_ticket::append_guard(ticket, guard);
    }
}

#[cfg(test)]
#[path = "runtime/tests.rs"]
mod tests;
