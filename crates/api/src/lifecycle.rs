//! 应用生命周期：注册外部依赖、启动后台任务、处理优雅关闭。

mod automated_arbitrage;
mod drain;
mod env;
mod exchanges;
mod funding;
mod funding_payments;
mod gate_crossex;
mod instruments;
mod ledger_projection;
#[cfg(feature = "legacy-chat")]
mod llm;
mod market_data;
pub(crate) mod nav_persist;
mod onchain_comparison;
mod portfolio;
mod portfolio_pnl;
mod private_ws;
mod profit_exit;
mod reconciliation;
mod review_projection;
mod shutdown;
mod snapshot;
mod system;
mod tasks;
mod venue_quality;
mod webhook;
mod webhook_events;
mod ws_housekeeping;

pub(crate) use drain::drain_runtime;
pub(crate) use shutdown::shutdown_signal;
pub(crate) use tasks::BackgroundTasks;

use crate::state::AppState;

pub(crate) fn refresh_exchange(state: &AppState, venue: &str) -> Result<(), String> {
    exchanges::refresh(state, venue)?;
    instruments::invalidate_transfer_probe_after_exchange_change(state, venue);
    Ok(())
}

pub(crate) fn request_onchain_transfer_networks(
    state: &AppState,
    snapshot: &shared_types::OnchainComparisonSnapshot,
) {
    instruments::request_transfer_networks_for_onchain(state, snapshot);
}

pub(crate) async fn refresh_onchain_transfer_networks(
    state: &AppState,
    snapshot: &shared_types::OnchainComparisonSnapshot,
) -> Result<bool, String> {
    instruments::refresh_transfer_networks_for_onchain(state, snapshot).await
}

/// 启动时初始化所有外部依赖：
///
/// 1. 10 家交易所适配器（凭证可选；公共行情始终可用）
/// 2. legacy chat feature 启用时注册 LLM provider（按 env API key 是否存在条件注册）
/// 3. 套利 snapshot updater（周期扫描 + WS 广播 `arbitrage` 频道）
/// 4. 资金费率 updater（周期聚合 + WS 广播 `funding-rates` 频道）
/// 5. 系统健康 updater（周期聚合 + WS 广播 `system` 频道）
/// 6. 持仓 / 风控 updater（周期聚合 + WS 广播 `portfolio` 频道）
///
/// 返回后台 task 句柄，调用方在关闭时调用 `shutdown` 协作式 drain（超时再 abort 兜底）。
pub(crate) fn init_services(state: &AppState) -> BackgroundTasks {
    let mut tasks = BackgroundTasks::new(state.task_registry().clone());

    exchanges::register(state);
    state.backpack_stocks().resume_rfq(state.ws_hub().clone());
    state.backpack_stocks().resume_funding(state.ws_hub().clone());
    #[cfg(feature = "legacy-chat")]
    llm::register(state);
    market_data::spawn_prewarm(state, &mut tasks);
    automated_arbitrage::spawn_worker(state, &mut tasks);
    onchain_comparison::spawn_worker(state, &mut tasks);
    snapshot::spawn_updater(state, &mut tasks);
    funding::spawn_updater(state, &mut tasks);
    funding_payments::spawn_updater(state, &mut tasks);
    gate_crossex::spawn_worker(state, &mut tasks);
    ledger_projection::spawn_worker(state, &mut tasks);
    instruments::spawn_updater(state, &mut tasks);
    private_ws::spawn_supervisor(state, &mut tasks);
    portfolio_pnl::spawn_updater(state, &mut tasks);
    review_projection::spawn_updater(state, &mut tasks);
    portfolio::spawn_updater(state, &mut tasks);
    profit_exit::spawn_worker(state, &mut tasks);
    reconciliation::spawn_updater(state, &mut tasks);
    system::spawn_updater(state, &mut tasks);
    venue_quality::spawn_updater(state, &mut tasks);
    webhook::spawn_worker(state, &mut tasks);
    webhook_events::spawn_worker(state, &mut tasks);
    ws_housekeeping::spawn_pruner(state, &mut tasks);

    tasks
}
