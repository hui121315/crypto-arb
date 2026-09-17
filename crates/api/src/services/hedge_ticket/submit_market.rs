use super::*;
use std::time::Duration;

const SECOND_LEG_REFRESH_TIMEOUT: Duration = Duration::from_millis(500);

pub(crate) struct SubmitMarketSnapshot {
    pub(crate) long_leg: HedgeLegQuote,
    pub(crate) short_leg: HedgeLegQuote,
    pub(crate) cost: Option<ExecutionCostProfile>,
    pub(crate) sizing: HedgeSizing,
    pub(crate) profit_proof: arbitrage::profit_proof::StrategyProfitProof,
    pub(crate) target_base_quantity: f64,
    checked_at_ms: i64,
}

impl SubmitMarketSnapshot {
    pub(crate) fn rejection_detail(&self) -> Option<String> {
        let mut blockers = self
            .long_leg
            .blockers
            .iter()
            .chain(&self.short_leg.blockers)
            .cloned()
            .collect::<Vec<_>>();
        if self.cost.is_none() {
            blockers.push("最新双腿盘口缺少完整费用或滑点证据".to_owned());
        }
        if self.sizing.max_executable_notional.status != HedgeDepthStatus::Available {
            blockers.push(
                self.sizing
                    .max_executable_notional
                    .reason
                    .clone()
                    .unwrap_or_else(|| "最新双腿可执行深度不足".to_owned()),
            );
        }
        if !self.profit_proof.passed {
            blockers.push(self.profit_proof.detail.clone());
        }
        let blockers = dedup(blockers);
        (!blockers.is_empty()).then(|| blockers.join("; "))
    }

    pub(crate) fn apply_to(self, ticket: &mut HedgeTicket) {
        ticket.long_leg = self.long_leg;
        ticket.short_leg = self.short_leg;
        ticket.cost = self.cost;
        ticket.sizing = self.sizing;
        ticket.market_checked_at_ms = self.checked_at_ms;
        refresh_submit_market_guards(ticket, &self.profit_proof);
        refresh_execution_order_guard(ticket);
    }

    pub(crate) const fn checked_at_ms(&self) -> i64 {
        self.checked_at_ms
    }

    pub(crate) fn set_target_base_quantity(&mut self, quantity: f64) {
        self.target_base_quantity = quantity;
        self.sizing.target_base_quantity = Some(quantity);
        self.sizing.max_executable_notional = executable_notional_for_quantity(
            self.sizing.target_notional_usd,
            Some(quantity),
            &self.long_leg,
            &self.short_leg,
        );
    }
}

pub(crate) async fn refresh_submit_market(
    state: &AppState,
    ticket: &HedgeTicket,
    target_base_quantity: f64,
) -> SubmitMarketSnapshot {
    let now_ms = common::time::now_ms();
    let (long_leg, short_leg) = join(
        refresh_ticket_leg_quote_for_base_quantity(
            state,
            ticket,
            &ticket.long_leg,
            target_base_quantity,
            now_ms,
            EXECUTION_DEPTH_BPS,
        ),
        refresh_ticket_leg_quote_for_base_quantity(
            state,
            ticket,
            &ticket.short_leg,
            target_base_quantity,
            now_ms,
            EXECUTION_DEPTH_BPS,
        ),
    )
    .await;
    let cost = refresh_ticket_cost(ticket, &long_leg, &short_leg, now_ms);
    let sizing = HedgeSizing {
        target_base_quantity: Some(target_base_quantity),
        max_executable_notional: executable_notional_for_quantity(
            ticket.sizing.target_notional_usd,
            Some(target_base_quantity),
            &long_leg,
            &short_leg,
        ),
        ..ticket.sizing.clone()
    };
    let profit_proof = strategy_profit_proof(
        ticket.strategy,
        ticket.spot_leg_mode,
        &long_leg,
        &short_leg,
        cost.as_ref(),
        now_ms,
    );
    SubmitMarketSnapshot {
        long_leg,
        short_leg,
        cost,
        sizing,
        profit_proof,
        target_base_quantity,
        checked_at_ms: now_ms,
    }
}

pub(crate) async fn refresh_second_leg_market(
    state: &AppState,
    ticket: &HedgeTicket,
    first_leg: &OrderRecord,
    first_role: HedgeLegRole,
) -> SubmitMarketSnapshot {
    let now_ms = common::time::now_ms();
    let second_role = opposite_role(first_role);
    let first_ticket_leg = quote_for_role(ticket, first_role);
    let second_ticket_leg = quote_for_role(ticket, second_role).clone();
    let target_base_quantity = filled_base_quantity(first_leg)
        .or(ticket.sizing.target_base_quantity)
        .unwrap_or_else(|| {
            ticket.sizing.target_notional_usd / first_ticket_leg.open_vwap_price.unwrap_or(1.0)
        });
    let mut long_leg = ticket.long_leg.clone();
    let mut short_leg = ticket.short_leg.clone();
    if let Some(fill_price) = first_leg
        .filled_price
        .filter(|price| price.is_finite() && *price > 0.0)
    {
        let first_quote = quote_for_role_mut(&mut long_leg, &mut short_leg, first_role);
        first_quote.open_vwap_price = Some(fill_price);
        first_quote.open_slippage_bps = slippage_bps(
            first_quote.reference_price,
            Some(fill_price),
            first_quote.side,
        );
        first_quote.market_timestamp_ms = Some(if first_leg.updated_at_ms > 0 {
            first_leg.updated_at_ms
        } else {
            now_ms
        });
    }
    let refreshed_second = refresh_second_leg_quote(
        state,
        ticket,
        &second_ticket_leg,
        target_base_quantity,
        now_ms,
    )
    .await;
    *quote_for_role_mut(&mut long_leg, &mut short_leg, second_role) = refreshed_second;
    let cost = refresh_ticket_cost(ticket, &long_leg, &short_leg, now_ms);
    let sizing = HedgeSizing {
        target_base_quantity: Some(target_base_quantity),
        max_executable_notional: executable_notional_for_quantity(
            ticket.sizing.target_notional_usd,
            Some(target_base_quantity),
            &long_leg,
            &short_leg,
        ),
        ..ticket.sizing.clone()
    };
    let profit_proof = strategy_profit_proof(
        ticket.strategy,
        ticket.spot_leg_mode,
        &long_leg,
        &short_leg,
        cost.as_ref(),
        now_ms,
    );
    SubmitMarketSnapshot {
        long_leg,
        short_leg,
        cost,
        sizing,
        profit_proof,
        target_base_quantity,
        checked_at_ms: now_ms,
    }
}

async fn refresh_second_leg_quote(
    state: &AppState,
    ticket: &HedgeTicket,
    ticket_leg: &HedgeLegQuote,
    target_base_quantity: f64,
    now_ms: i64,
) -> HedgeLegQuote {
    let refresh = refresh_ticket_leg_quote_for_base_quantity(
        state,
        ticket,
        ticket_leg,
        target_base_quantity,
        now_ms,
        EXECUTION_DEPTH_BPS,
    );
    match tokio::time::timeout(SECOND_LEG_REFRESH_TIMEOUT, refresh).await {
        Ok(quote) => quote,
        Err(_) => second_leg_refresh_timeout_quote(ticket_leg),
    }
}

fn second_leg_refresh_timeout_quote(ticket_leg: &HedgeLegQuote) -> HedgeLegQuote {
    let mut quote = ticket_leg.clone();
    let reason = format!(
        "{} {} 第一腿成交后未在 {}ms 内取得最新第二腿 WS 盘口",
        quote.exchange,
        quote.symbol,
        SECOND_LEG_REFRESH_TIMEOUT.as_millis()
    );
    quote.open_vwap_price = None;
    quote.open_slippage_bps = None;
    quote.depth_usd_5bps = None;
    quote.depth_usd_10bps = None;
    quote.depth_usd_20bps = None;
    quote.max_notional_usd = None;
    quote.depth_reason = Some(reason.clone());
    quote.blockers.push(reason);
    quote.blockers = dedup(quote.blockers);
    quote
}

fn filled_base_quantity(record: &OrderRecord) -> Option<f64> {
    record
        .filled_quantity
        .map(f64::abs)
        .filter(|quantity| quantity.is_finite() && *quantity > 0.0)
}

fn quote_for_role(ticket: &HedgeTicket, role: HedgeLegRole) -> &HedgeLegQuote {
    match role {
        HedgeLegRole::Long => &ticket.long_leg,
        HedgeLegRole::Short => &ticket.short_leg,
    }
}

fn quote_for_role_mut<'a>(
    long: &'a mut HedgeLegQuote,
    short: &'a mut HedgeLegQuote,
    role: HedgeLegRole,
) -> &'a mut HedgeLegQuote {
    match role {
        HedgeLegRole::Long => long,
        HedgeLegRole::Short => short,
    }
}
