use crate::state::AppState;
use shared_types::{
    HedgeLegQuote, HedgePreviewResponse, OrderIntent, OrderRecord, OrderSide, OrderType,
    RiskDecision,
};

mod market_binding;
mod protection;
use market_binding::{refresh_order_sizing_plans, SizingRefreshMode};
use protection::*;

struct HedgeRecheck {
    long: RiskDecision,
    short: RiskDecision,
}

impl HedgeRecheck {
    fn allowed(&self) -> bool {
        self.long.allowed && self.short.allowed
    }

    fn rejection_detail(&self, prefix: &str) -> Option<String> {
        if self.allowed() {
            None
        } else {
            Some(format!(
                "{prefix}: 多腿: {}; 空腿: {}",
                decision_detail(&self.long),
                decision_detail(&self.short)
            ))
        }
    }
}

pub(crate) async fn pre_submit_rejection(
    state: &AppState,
    preview: &mut HedgePreviewResponse,
) -> Option<String> {
    let (long, short) = state
        .trading_service()
        .check_hedge(&preview.long_leg, &preview.short_leg);
    if let Some(detail) = (HedgeRecheck { long, short }).rejection_detail("执行前风控复检未通过")
    {
        return Some(detail);
    }
    let mut market = crate::services::hedge_ticket::refresh_submit_market(
        state,
        &preview.ticket,
        preview.long_leg.quantity,
    )
    .await;
    if let Some(detail) = market.rejection_detail() {
        return Some(format!("执行前盘口复检未通过: {detail}"));
    }
    if let Some(detail) = order_protection_rejection(preview, &market) {
        return Some(format!("执行前价格保护未通过: {detail}"));
    }
    if let Some(detail) =
        refresh_order_sizing_plans(preview, &mut market, SizingRefreshMode::PairedWithinCaps)
    {
        return Some(format!("执行前双腿数量复检未通过: {detail}"));
    }
    if let Some(detail) = refreshed_market_risk_rejection(
        state,
        preview,
        &market,
        state.trading_service().open_order_count(),
        "执行前最新行情风控复检未通过",
    ) {
        return Some(detail);
    }
    let checked_at_ms = market.checked_at_ms();
    market.apply_to(&mut preview.ticket);
    crate::services::hedge_preview::refresh_submit_market_projection(preview, checked_at_ms);
    None
}

pub(crate) async fn before_second_leg_rejection(
    state: &AppState,
    preview: &mut HedgePreviewResponse,
    first_leg: &OrderRecord,
    first_role: shared_types::HedgeLegRole,
) -> Option<String> {
    let open_orders = open_orders_excluding_self(state.trading_service().open_order_count());
    let (long, short) = state.trading_service().check_hedge_with_open_orders(
        &preview.long_leg,
        &preview.short_leg,
        open_orders,
    );
    if let Some(detail) = (HedgeRecheck { long, short }).rejection_detail("二次风控复检未通过")
    {
        return Some(detail);
    }
    let mut market = crate::services::hedge_ticket::refresh_second_leg_market(
        state,
        &preview.ticket,
        first_leg,
        first_role,
    )
    .await;
    if let Some(detail) = market.rejection_detail() {
        return Some(format!("二次盘口复检未通过: {detail}"));
    }
    if let Some(detail) = order_protection_rejection(preview, &market) {
        return Some(format!("二次价格保护未通过: {detail}"));
    }
    if let Some(detail) =
        refresh_order_sizing_plans(preview, &mut market, SizingRefreshMode::ExactQuantity)
    {
        return Some(format!("二次数量复检未通过: {detail}"));
    }
    if let Some(detail) = refreshed_market_risk_rejection(
        state,
        preview,
        &market,
        open_orders,
        "第二腿最新行情风控复检未通过",
    ) {
        return Some(detail);
    }
    let checked_at_ms = market.checked_at_ms();
    market.apply_to(&mut preview.ticket);
    crate::services::hedge_preview::refresh_submit_market_projection(preview, checked_at_ms);
    None
}

fn refreshed_market_risk_rejection(
    state: &AppState,
    preview: &mut HedgePreviewResponse,
    market: &crate::services::hedge_ticket::SubmitMarketSnapshot,
    open_orders: usize,
    prefix: &str,
) -> Option<String> {
    let mut long_intent = preview.long_leg.clone();
    long_intent.price = market.long_leg.open_vwap_price;
    let mut short_intent = preview.short_leg.clone();
    short_intent.price = market.short_leg.open_vwap_price;
    let (long, short) = state.trading_service().check_hedge_with_open_orders(
        &long_intent,
        &short_intent,
        open_orders,
    );
    let rejection = HedgeRecheck {
        long: long.clone(),
        short: short.clone(),
    }
    .rejection_detail(prefix);
    preview.long_risk = long;
    preview.short_risk = short;
    rejection
}

fn open_orders_excluding_self(current_open_orders: usize) -> usize {
    current_open_orders.saturating_sub(1)
}

fn decision_detail(decision: &RiskDecision) -> String {
    if decision.allowed {
        "通过".to_owned()
    } else if !decision.evidence.is_empty() {
        decision
            .evidence
            .iter()
            .map(shared_types::RiskBlockEvidence::compact_summary)
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!("{:?}", decision.reasons)
    }
}

#[cfg(test)]
#[path = "hedge_recheck/test_support.rs"]
mod test_support;
#[cfg(test)]
use test_support::*;

#[cfg(test)]
#[path = "hedge_recheck/tests.rs"]
mod tests;
