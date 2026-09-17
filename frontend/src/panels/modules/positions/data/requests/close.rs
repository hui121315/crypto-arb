//! `CloseRun` 与持仓平仓的 `ApiClient` 任务。

use crate::api::rest::{ApiClient, ApiError, MutationRequestContext};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    ApiProblem, CloseAllPositionsRequest, ClosePositionRequest, CloseRun,
    CloseRunCompensationRequest, CloseRunManualTerminalRequest, OrderRecord, PortfolioSnapshot,
    PositionOrigin, PositionRow, TradingStatusResponse,
};

use super::{close_compensation_problem, CloseRequestResult};

pub(in crate::panels::modules::positions) fn close_position_request(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
    row: &PositionRow,
    expected_leg_count: usize,
    reason: &'static str,
) -> CloseRequestResult<ClosePositionRequest> {
    Ok(ClosePositionRequest {
        side: Some(row.side),
        snapshot_version: Some(close_snapshot_version(snapshot_state)?),
        expected_leg_count: Some(expected_leg_count),
        reason: Some(reason.to_owned()),
    })
}

pub(in crate::panels::modules::positions) fn close_all_request(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
    confirmation_phrase: String,
    reason: &'static str,
) -> CloseRequestResult<CloseAllPositionsRequest> {
    let snapshot = close_snapshot(snapshot_state)?;
    Ok(CloseAllPositionsRequest {
        confirmation_phrase,
        snapshot_version: Some(required_snapshot_version(&snapshot)?),
        expected_leg_count: Some(snapshot.positions.len()),
        reason: Some(reason.to_owned()),
    })
}

pub(in crate::panels::modules::positions) fn close_execution_scope(
    trading_status: RwSignal<LoadState<TradingStatusResponse>>,
    requires_live: bool,
) -> CloseRequestResult<String> {
    let status_state = trading_status.get_untracked();
    let Some(status) = status_state.value() else {
        if requires_live {
            return Err(close_mode_problem(
                "执行环境状态尚未就绪，不能关闭交易所真实仓位",
            ));
        }
        return Ok("adapter=unknown:environment=paper".to_owned());
    };
    if requires_live
        && (status.environment != shared_types::ExecutionEnvironment::Live
            || !status.risk.live_trading_enabled)
    {
        return Err(close_mode_problem(
            "当前为模拟模式，不能关闭交易所真实仓位；请先在设置的执行环境中两步启用实盘",
        ));
    }
    Ok(format!(
        "adapter={}:environment={}",
        status.adapter,
        execution_environment_scope(status.environment)
    ))
}

pub(in crate::panels::modules::positions) fn portfolio_close_requires_live(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) -> bool {
    snapshot_state
        .get_untracked()
        .value()
        .is_some_and(|snapshot| {
            snapshot
                .positions
                .iter()
                .any(|row| row.origin == PositionOrigin::AccountPrivate)
        })
}

pub(super) fn close_run_snapshot_version(run: &CloseRun) -> CloseRequestResult<String> {
    let version = run.snapshot_version.trim();
    if version.is_empty() {
        return Err(close_compensation_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "平仓事故缺少版本号，不能提交补偿单",
        ));
    }
    Ok(version.to_owned())
}

pub(in crate::panels::modules::positions) async fn close_position_task(
    client: ApiClient,
    row: PositionRow,
    request: ClosePositionRequest,
    context: MutationRequestContext,
) -> Result<CloseRun, ApiError> {
    client
        .close_portfolio_position_with_context(&row.venue, &row.symbol, request, &context)
        .await
}

pub(in crate::panels::modules::positions) async fn close_position_pair_task(
    client: ApiClient,
    row: PositionRow,
    request: ClosePositionRequest,
    context: MutationRequestContext,
) -> Result<CloseRun, ApiError> {
    client
        .close_portfolio_position_pair_with_context(&row.venue, &row.symbol, request, &context)
        .await
}

pub(in crate::panels::modules::positions) async fn close_all_positions_task(
    client: ApiClient,
    request: CloseAllPositionsRequest,
    context: MutationRequestContext,
) -> Result<CloseRun, ApiError> {
    client
        .close_all_portfolio_positions_with_context(request, &context)
        .await
}

pub(in crate::panels::modules::positions) async fn submit_close_run_compensation_task(
    client: ApiClient,
    close_run_id: &str,
    request: CloseRunCompensationRequest,
    context: MutationRequestContext,
) -> Result<CloseRun, ApiError> {
    client
        .submit_close_run_compensation_with_context(close_run_id, request, &context)
        .await
}

pub(in crate::panels::modules::positions) async fn submit_close_run_manual_terminal_task(
    client: ApiClient,
    close_run_id: &str,
    request: CloseRunManualTerminalRequest,
    context: MutationRequestContext,
) -> Result<CloseRun, ApiError> {
    client
        .submit_close_run_manual_terminal_with_context(close_run_id, request, &context)
        .await
}

pub(in crate::panels::modules::positions) async fn cancel_close_run_compensation_task(
    client: ApiClient,
    order_id: &str,
    context: MutationRequestContext,
) -> Result<OrderRecord, ApiError> {
    client.cancel_order_with_context(order_id, &context).await
}

fn close_snapshot_version(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) -> CloseRequestResult<String> {
    required_snapshot_version(&close_snapshot(snapshot_state)?)
}

fn close_snapshot(
    snapshot_state: RwSignal<LoadState<PortfolioSnapshot>>,
) -> CloseRequestResult<PortfolioSnapshot> {
    match snapshot_state.get_untracked() {
        LoadState::Ready(snapshot)
        | LoadState::Stale {
            value: snapshot, ..
        } => Ok(snapshot),
        LoadState::Loading => Err(snapshot_required_problem("持仓快照仍在加载，不能提交平仓")),
        LoadState::Error(problem) => Err(Box::new(problem.with_source("positions.close_request"))),
    }
}

fn required_snapshot_version(snapshot: &PortfolioSnapshot) -> CloseRequestResult<String> {
    let version = snapshot.snapshot_version.trim();
    if version.is_empty() {
        return Err(snapshot_required_problem(
            "持仓快照缺少版本号，不能提交平仓",
        ));
    }
    Ok(version.to_owned())
}

fn snapshot_required_problem(message: &'static str) -> Box<ApiProblem> {
    Box::new(
        ApiProblem::new(
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_REQUIRED,
            message,
        )
        .with_source("positions.close_request"),
    )
}

fn close_mode_problem(message: &'static str) -> Box<ApiProblem> {
    Box::new(
        ApiProblem::new(shared_types::problem::codes::RISK_BLOCKED, message)
            .with_source("positions.close_mode"),
    )
}

const fn execution_environment_scope(
    environment: shared_types::ExecutionEnvironment,
) -> &'static str {
    match environment {
        shared_types::ExecutionEnvironment::Paper => "paper",
        shared_types::ExecutionEnvironment::Live => "live",
    }
}
