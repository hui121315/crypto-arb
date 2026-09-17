use super::*;
use crate::services::hedge_preflight::OrderCapabilityCheck;
use exchange::ExchangeCapabilities;

pub(crate) fn available_order_types(capabilities: &ExchangeCapabilities) -> Vec<OrderType> {
    let mut types = Vec::with_capacity(3);
    if capabilities.supports_limit_orders {
        types.push(OrderType::Limit);
    }
    if capabilities.supports_market_orders {
        types.push(OrderType::Market);
    }
    if capabilities.supports_limit_orders && capabilities.supports_post_only {
        types.push(OrderType::PostOnly);
    }
    types
}

pub(crate) fn append_order_capability_guard(
    state: &AppState,
    ticket: &mut HedgeTicket,
    mode: ExecutionMode,
    long_order_plan: &shared_types::OrderCompilePlan,
    short_order_plan: &shared_types::OrderCompilePlan,
) {
    if mode != ExecutionMode::Live {
        return;
    }
    let trading = state.trading_service();
    let checks = [
        OrderCapabilityCheck::new(
            long_order_plan,
            trading
                .exchange_capabilities(&long_order_plan.exchange)
                .map_err(|error| error.to_string()),
        ),
        OrderCapabilityCheck::new(
            short_order_plan,
            trading
                .exchange_capabilities(&short_order_plan.exchange)
                .map_err(|error| error.to_string()),
        ),
    ];
    crate::services::hedge_ticket::append_guard(
        ticket,
        crate::services::hedge_preflight::order_capability_guard(&checks),
    );
}

pub(crate) fn account_mode_plan(
    mode: ExecutionMode,
    plan: &shared_types::OrderCompilePlan,
) -> bool {
    crate::services::hedge_preflight::account_mode_plan(mode, plan)
}
