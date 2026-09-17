use super::*;
use crate::state::AppState;
use shared_types::{ExecutionMode, OrderIntent};

pub(crate) struct LiveOrderPreflight<'a> {
    pub(crate) capability: OrderCapabilityCheck<'a>,
    pub(crate) account_mode: Option<AccountModeCheck<'a>>,
    pub(crate) order_write: Option<OrderWriteCheck<'a>>,
}

pub(crate) async fn collect_live_order_preflight<'a>(
    state: &AppState,
    mode: ExecutionMode,
    intent: &OrderIntent,
    plan: &'a mut OrderCompilePlan,
) -> Option<LiveOrderPreflight<'a>> {
    if mode != ExecutionMode::Live {
        return None;
    }

    Some(collect_required_live_order_preflight(state, intent, plan).await)
}

async fn collect_required_live_order_preflight<'a>(
    state: &AppState,
    intent: &OrderIntent,
    plan: &'a mut OrderCompilePlan,
) -> LiveOrderPreflight<'a> {
    let capabilities = state
        .trading_service()
        .exchange_capabilities(&plan.exchange)
        .map_err(|error| error.to_string());
    let remote_preflight = requires_remote_order_preflight(intent);
    let account_mode = if remote_preflight && account_mode_plan(ExecutionMode::Live, plan) {
        let result = state
            .trading_service()
            .exchange_symbol_account_mode(&plan.exchange, &plan.symbol)
            .await
            .map_err(|error| error.to_string());
        attach_account_mode_evidence(plan, &result);
        Some(result)
    } else {
        None
    };
    let order_write = if remote_preflight {
        let context = plan.submission_context();
        Some(
            state
                .trading_service()
                .preflight_order_with_context(intent, &context)
                .await
                .map_err(|error| error.to_string()),
        )
    } else {
        None
    };

    LiveOrderPreflight {
        capability: OrderCapabilityCheck::new(plan, capabilities),
        account_mode: account_mode.map(|result| AccountModeCheck::new(plan, result)),
        order_write: order_write.map(|result| OrderWriteCheck::new(plan, result)),
    }
}

pub(crate) async fn recheck_hedge_live_order_preflight_guards(
    state: &AppState,
    long_intent: &OrderIntent,
    long_plan: &OrderCompilePlan,
    short_intent: &OrderIntent,
    short_plan: &OrderCompilePlan,
) -> Vec<ExecutionGuard> {
    let mut long_plan = long_plan.clone();
    let mut short_plan = short_plan.clone();
    let (long, short) = tokio::join!(
        collect_required_live_order_preflight(state, long_intent, &mut long_plan),
        collect_required_live_order_preflight(state, short_intent, &mut short_plan),
    );
    hedge_live_order_preflight_guards(long, short)
}

pub(crate) fn single_live_order_preflight_guards(
    preflight: LiveOrderPreflight<'_>,
) -> Vec<ExecutionGuard> {
    let LiveOrderPreflight {
        capability,
        account_mode,
        order_write,
    } = preflight;
    let mut guards = vec![order_capability_guard(&[capability])];
    if let Some(account_mode) = account_mode {
        if let Some(guard) = account_mode_guard(&[account_mode]) {
            guards.push(guard);
        }
    }
    if let Some(order_write) = order_write {
        if let Some(guard) = order_write_guard(&[order_write]) {
            guards.push(guard);
        }
    }
    guards
}

pub(crate) fn hedge_live_order_preflight_guards(
    long: LiveOrderPreflight<'_>,
    short: LiveOrderPreflight<'_>,
) -> Vec<ExecutionGuard> {
    let LiveOrderPreflight {
        capability: long_capability,
        account_mode: long_account_mode,
        order_write: long_order_write,
    } = long;
    let LiveOrderPreflight {
        capability: short_capability,
        account_mode: short_account_mode,
        order_write: short_order_write,
    } = short;
    let mut guards = vec![order_capability_guard(&[long_capability, short_capability])];
    let account_modes = [long_account_mode, short_account_mode]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(guard) = account_mode_guard(&account_modes) {
        guards.push(guard);
    }
    let order_writes = [long_order_write, short_order_write]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(guard) = order_write_guard(&order_writes) {
        guards.push(guard);
    }
    guards
}

pub(super) fn requires_remote_order_preflight(intent: &OrderIntent) -> bool {
    !intent.reduce_only
}

pub(crate) fn account_mode_plan(mode: ExecutionMode, plan: &OrderCompilePlan) -> bool {
    mode == ExecutionMode::Live
        && matches!(
            venue_family(&plan.exchange),
            "binance" | "bitget" | "bybit" | "gate" | "gate_crossex" | "kucoin" | "okx"
        )
}

fn attach_account_mode_evidence(
    plan: &mut OrderCompilePlan,
    account_mode: &Result<Option<VenueAccountModeInfo>, String>,
) {
    match account_mode {
        Ok(Some(info)) => {
            plan.venue_capability.account_mode = Some(info.clone());
            plan.venue_capability.account_mode_error = None;
        }
        Ok(None) => {
            plan.venue_capability.account_mode_error = Some("账户模式证据未返回".to_owned());
        }
        Err(error) => {
            plan.venue_capability.account_mode_error = Some(error.clone());
        }
    }
}
