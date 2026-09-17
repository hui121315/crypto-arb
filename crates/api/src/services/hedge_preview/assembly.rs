use super::*;

pub(super) struct PreviewLegBuildInput<'a> {
    pub(super) opportunity: &'a ArbitrageOpportunityDto,
    pub(super) key: &'a str,
    pub(super) params: &'a HedgeExecutionParams,
    pub(super) mode: ExecutionMode,
    pub(super) strategy: Option<StrategyKind>,
    pub(super) quantity: f64,
    pub(super) prices: [f64; 2],
}

pub(super) struct PreviewLegBuilds {
    pub(super) long_leg: shared_types::OrderIntent,
    pub(super) short_leg: shared_types::OrderIntent,
    pub(super) long_order_plan: shared_types::OrderCompilePlan,
    pub(super) short_order_plan: shared_types::OrderCompilePlan,
}

pub(super) fn build_preview_legs(input: &PreviewLegBuildInput<'_>) -> PreviewLegBuilds {
    let long_build = LegBuild {
        opp: input.opportunity,
        key: input.key,
        is_long: true,
        quantity: input.quantity,
        price: input.prices[0],
        params: input.params,
        mode: input.mode,
        strategy: input.strategy,
    };
    let long_leg = build_leg(long_build);
    let long_order_plan = compile_order_plan(&long_build, &long_leg);
    let short_build = LegBuild {
        opp: input.opportunity,
        key: input.key,
        is_long: false,
        quantity: input.quantity,
        price: input.prices[1],
        params: input.params,
        mode: input.mode,
        strategy: input.strategy,
    };
    let short_leg = build_leg(short_build);
    let short_order_plan = compile_order_plan(&short_build, &short_leg);
    PreviewLegBuilds {
        long_leg,
        short_leg,
        long_order_plan,
        short_order_plan,
    }
}

pub(super) fn opportunity_strategy(opportunity: &ArbitrageOpportunityDto) -> Option<StrategyKind> {
    opportunity
        .strategy_kind
        .or_else(|| p0_strategy_from_arb_type(opportunity.arb_type))
}

pub(super) fn preview_target_base_quantity(
    ticket: &HedgeTicket,
    legs: (f64, f64, f64, f64),
) -> f64 {
    ticket.sizing.target_base_quantity.unwrap_or_else(|| {
        let (long_notional, short_notional, long_price, short_price) = legs;
        (long_notional / long_price).min(short_notional / short_price)
    })
}

pub(super) struct InitialPreviewEconomics {
    pub(super) funding_usd: f64,
    pub(super) gross_edge_usd: f64,
    pub(super) costs: pricing::PreviewCosts,
}

pub(super) fn initial_preview_economics(
    ticket: &HedgeTicket,
    long_order_plan: &shared_types::OrderCompilePlan,
    long_quantity: f64,
    long_price: f64,
) -> InitialPreviewEconomics {
    let execution_notional = long_order_plan
        .sizing_plan
        .map_or(long_quantity * long_price, |plan| plan.actual_notional_usd);
    InitialPreviewEconomics {
        funding_usd: execution_notional * preview_funding_yield(ticket),
        gross_edge_usd: pricing::gross_edge_usd(ticket, execution_notional),
        costs: estimated_costs_usd(ticket, execution_notional),
    }
}

pub(super) fn ticket_plans(
    ticket: &HedgeTicket,
    long_order_plan: shared_types::OrderCompilePlan,
    short_order_plan: shared_types::OrderCompilePlan,
) -> Result<HedgeTicketOrderPlans, AppError> {
    HedgeTicketOrderPlans::from_compile_plans(
        ticket.ticket_id.clone(),
        long_order_plan,
        short_order_plan,
    )
    .map_err(|error| {
        AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_TICKET_BLOCKED,
            format!("ticket order plan evidence invalid: {error}"),
        )
    })
}
