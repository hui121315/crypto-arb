use super::*;

const EXECUTION_ORDER_GUARD_KEY: &str = "execution_order";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HedgeExecutionOrder {
    pub(crate) first: HedgeLegRole,
    pub(crate) second: HedgeLegRole,
}

impl HedgeExecutionOrder {
    fn from_first(first: HedgeLegRole) -> Self {
        Self {
            first,
            second: opposite_role(first),
        }
    }
}

pub(crate) fn execution_order(ticket: &HedgeTicket) -> HedgeExecutionOrder {
    execution_order_for_quotes(&ticket.long_leg, &ticket.short_leg)
}

pub(super) fn execution_order_guard(long: &HedgeLegQuote, short: &HedgeLegQuote) -> ExecutionGuard {
    let long_depth = executable_depth(long);
    let short_depth = executable_depth(short);
    let order = execution_order_for_depth(long_depth, short_depth);
    let passed = long_depth.is_some() && short_depth.is_some();
    let detail = match (long_depth, short_depth) {
        (Some(long_depth), Some(short_depth)) => format!(
            "先执行{}，保留{}作为快速对冲腿；0.05% 深度 多腿 ${long_depth:.2} / 空腿 ${short_depth:.2}",
            role_label(order.first),
            role_label(order.second),
        ),
        _ => "双腿 0.05% 深度尚未完整，无法确定低裸露风险执行顺序".to_owned(),
    };
    ExecutionGuard {
        key: EXECUTION_ORDER_GUARD_KEY.to_owned(),
        label: "双腿执行顺序".to_owned(),
        passed,
        detail,
        preflight_outcome: None,
    }
}

pub(super) fn refresh_execution_order_guard(ticket: &mut HedgeTicket) {
    let guard = execution_order_guard(&ticket.long_leg, &ticket.short_leg);
    if let Some(existing) = ticket
        .guards
        .iter_mut()
        .find(|existing| existing.key == EXECUTION_ORDER_GUARD_KEY)
    {
        *existing = guard;
    } else {
        ticket.guards.push(guard);
    }
}

pub(super) fn execution_order_for_quotes(
    long: &HedgeLegQuote,
    short: &HedgeLegQuote,
) -> HedgeExecutionOrder {
    execution_order_for_depth(executable_depth(long), executable_depth(short))
}

fn execution_order_for_depth(
    long_depth: Option<f64>,
    short_depth: Option<f64>,
) -> HedgeExecutionOrder {
    let first = match (long_depth, short_depth) {
        (Some(long), Some(short)) if short < long => HedgeLegRole::Short,
        _ => HedgeLegRole::Long,
    };
    HedgeExecutionOrder::from_first(first)
}

pub(crate) const fn opposite_role(role: HedgeLegRole) -> HedgeLegRole {
    match role {
        HedgeLegRole::Long => HedgeLegRole::Short,
        HedgeLegRole::Short => HedgeLegRole::Long,
    }
}

pub(crate) const fn role_label(role: HedgeLegRole) -> &'static str {
    match role {
        HedgeLegRole::Long => "多腿",
        HedgeLegRole::Short => "空腿",
    }
}
