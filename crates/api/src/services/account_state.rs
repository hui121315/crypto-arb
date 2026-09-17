use crate::services::{
    account_balances, account_open_orders, account_positions, account_quality,
    venue_operation_health,
};
use crate::state::AppState;
use shared_types::{
    normalized_venue_name, problem::codes, AccountBindingEvidence, AccountFieldQuality,
    AccountFieldQualityStatus, AccountFieldSubject, AccountStateSnapshot, ApiProblem, ListStatus,
    VenueAccountSummary, VenueBalanceEnvelope, VenueOpenOrdersEnvelope, VenueOperationHealth,
    VenueOperationKind, VenueOperationStatus, VenuePositionEnvelope,
};
use std::collections::BTreeSet;

const ACCOUNT_STATE_SOURCE: &str = "account_state_runtime";
const ACCOUNT_STATE_WARMING_SOURCE: &str = "account_state_warming";
const ACCOUNT_STATE_WARMING_RETRY_AFTER_MS: u64 = 2_000;
const EQUITY_FIELD: &str = "equity";

mod derive;

use derive::{
    account_equity_unknown_problem, account_equity_unknown_quality,
    account_operation_field_quality, account_operation_health_from_rows, account_state_status,
    account_summary_field_quality, account_summary_problems, apply_account_scopes,
    child_account_bindings, child_field_quality, child_operation_health, child_problems,
    is_unknown_equity_quality,
};

pub(crate) async fn snapshot(state: &AppState) -> AccountStateSnapshot {
    state
        .trading_service()
        .schedule_hyperliquid_account_evidence_refresh();
    let observed_at_ms = common::time::now_ms();
    let operation_rows = venue_operation_health::snapshot(state).rows;
    // Balances and positions are the portfolio-critical paths. Open orders use a
    // stale-while-revalidate snapshot so a single slow venue cannot stall position publication.
    let open_orders = account_open_orders::cached_envelope_with_operation_health(
        state,
        account_open_orders::operation_health_from_rows(&operation_rows),
    );
    let (balances, positions) = tokio::join!(
        account_balances::envelope_with_operation_health(
            state,
            account_balances::operation_health_from_rows(&operation_rows),
        ),
        account_positions::envelope_with_operation_health(
            state,
            account_positions::operation_health_from_rows(&operation_rows),
        ),
    );
    snapshot_from_parts(
        balances,
        positions,
        open_orders,
        &account_operation_health_from_rows(&operation_rows),
        observed_at_ms,
    )
}

/// 热路径只读：从后台 portfolio 快照缓存读取账户状态，绝不在请求线程触发 private REST。
/// 后台 portfolio updater 已按 venue 并发刷新并写入该缓存；缓存冷启动时返回 typed
/// warming 快照（带明确 problem），不把"缓存未就绪"伪装成无余额/无凭证。
pub(crate) fn cached_snapshot(state: &AppState) -> AccountStateSnapshot {
    match state.portfolio_snapshot().value_now() {
        Some(snapshot) => snapshot.account_state,
        None => warming_snapshot(common::time::now_ms()),
    }
}

/// 后台快照尚未产出时的 typed warming 快照：Degraded + 明确 warming problem。
pub(crate) fn warming_snapshot(observed_at_ms: i64) -> AccountStateSnapshot {
    let problems = vec![warming_problem()];
    let balances = VenueBalanceEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        ACCOUNT_STATE_WARMING_SOURCE,
        observed_at_ms,
        problems.clone(),
        Vec::new(),
    );
    let positions = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        ACCOUNT_STATE_WARMING_SOURCE,
        observed_at_ms,
        problems.clone(),
        Vec::new(),
    );
    let open_orders = VenueOpenOrdersEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        ACCOUNT_STATE_WARMING_SOURCE,
        observed_at_ms,
        problems.clone(),
        Vec::new(),
    );
    AccountStateSnapshot {
        balances,
        positions,
        open_orders,
        status: ListStatus::Degraded,
        source: ACCOUNT_STATE_WARMING_SOURCE.to_owned(),
        observed_at_ms,
        problems,
        operation_health: Vec::new(),
        field_quality: Vec::new(),
        account_bindings: Vec::new(),
    }
}

fn warming_problem() -> ApiProblem {
    ApiProblem::new(
        codes::ACCOUNT_STATE_SNAPSHOT_WARMING,
        "account state snapshot is warming; background refresh has not produced a snapshot yet",
    )
    .with_retry_after_ms(Some(ACCOUNT_STATE_WARMING_RETRY_AFTER_MS))
    .with_source(ACCOUNT_STATE_WARMING_SOURCE)
}

pub(crate) fn snapshot_from_envelopes(
    balances: VenueBalanceEnvelope,
    positions: VenuePositionEnvelope,
    observed_at_ms: i64,
) -> AccountStateSnapshot {
    snapshot_from_parts(
        balances,
        positions,
        VenueOpenOrdersEnvelope::default(),
        &[],
        observed_at_ms,
    )
}

fn snapshot_from_parts(
    balances: VenueBalanceEnvelope,
    positions: VenuePositionEnvelope,
    open_orders: VenueOpenOrdersEnvelope,
    account_operation_health: &[VenueOperationHealth],
    observed_at_ms: i64,
) -> AccountStateSnapshot {
    let mut problems = child_problems(&balances, &positions, &open_orders);
    problems.extend(account_summary_problems(&balances.account_summaries));
    let operation_health = child_operation_health(
        &balances,
        &positions,
        &open_orders,
        account_operation_health,
    );
    let account_bindings = child_account_bindings(&balances, &positions, &open_orders);
    let mut field_quality = child_field_quality(&balances, &positions, &open_orders);
    field_quality.extend(account_operation_field_quality(
        account_operation_health,
        observed_at_ms,
    ));
    field_quality.extend(account_summary_field_quality(&balances.account_summaries));
    field_quality.extend(account_equity_unknown_quality(
        &balances,
        &positions,
        &open_orders,
        &operation_health,
        &account_bindings,
        observed_at_ms,
    ));
    apply_account_scopes(&mut field_quality, &account_bindings);
    let status = account_state_status(
        balances.status,
        positions.status,
        open_orders.status,
        &problems,
        &operation_health,
        &field_quality,
    );
    if field_quality.iter().any(is_unknown_equity_quality) {
        problems.push(account_equity_unknown_problem(None, observed_at_ms));
    }
    AccountStateSnapshot {
        balances,
        positions,
        open_orders,
        status,
        source: ACCOUNT_STATE_SOURCE.to_owned(),
        observed_at_ms,
        problems,
        operation_health,
        field_quality,
        account_bindings,
    }
}

#[cfg(test)]
mod tests;
