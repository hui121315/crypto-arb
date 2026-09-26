use crate::state::load_state::LoadState;
use gloo_timers::callback::{Interval, Timeout};
use leptos::prelude::*;
use shared_types::{
    onchain_cex_base_token, onchain_cex_pair_matches, onchain_cex_quote_token,
    onchain_quotes_match, ApiProblem, OnchainCexComparison, OnchainCexInstrumentEvidence,
    OnchainCexInstrumentStatus, OnchainComparisonConfig, OnchainComparisonDirection,
    OnchainComparisonQuality, OnchainComparisonSnapshot, OnchainCrossChainBuildRequest,
    OnchainCrossChainBuildResponse, OnchainCrossChainInventoryStatus, OnchainCrossChainLeg,
    OnchainCrossChainLegKind, OnchainCrossChainQuality, OnchainDexComparisonDirection,
    OnchainDexComparisonQuality, OnchainDexRouteComparison, OnchainDirectionReadiness,
    OnchainExecutionBuildRequest, OnchainExecutionBuildResponse, OnchainExecutionLegKind,
    OnchainExecutionLegResult, OnchainExecutionLegStatus, OnchainExecutionRecoveryAction,
    OnchainExecutionRunStatus, OnchainExecutionSubmitResponse, OnchainInventoryLocation,
    OnchainInventoryStatus, OnchainPathAvailability, OnchainPathKind,
    OnchainQuoteConversionSequence, OnchainReplenishmentBuildRequest,
    OnchainReplenishmentPlanResponse, OnchainReplenishmentRun, OnchainSpreadAlertMode,
    OnchainTokenApprovalBuildResponse, OnchainTokenApprovalRunStatus,
    OnchainTokenApprovalSubmitResponse, OnchainTransferDirection, OnchainTransferStatus,
    OnchainUnsignedTransaction, OrderSide,
};

use super::super::data::{OnchainCrossChainData, OnchainData, PreviewContext};

use super::super::format::{
    cex_source_label, chain_label, cost_percent_label, direction_label, percent_label, price_label,
    provider_label, raw_amount_label, usd,
};
use super::decision_guidance::empty_decision_guidance;
use super::replenishment_control::replenishment_panel;
use super::spread_chart::{spread_chart, SpreadHistory};

mod accounting_receipt;
mod execution_ticket;
mod replenishment_cost_selection;
mod approval_cost_selection;
mod settlement_receipt;

pub(in crate::panels::modules::onchain) fn decision_board(
    data: OnchainData,
    open_execution_setup: Callback<()>,
    active_direction: PreviewContext,
    show_execution_result: Callback<()>,
) -> impl IntoView {
    let spread_history = SpreadHistory::new(data.state);
    let execution_clock_ms = execution_clock(
        data.execution.cross_chain,
        data.execution.execution_build,
        data.execution.approval_build,
        data.replenishment.plan,
        data.replenishment.run,
    );
    let execution_evidence_open = RwSignal::new(false);
    view! {
        <section class="onchain-decision-board">
            {move || data.state.with(|state| state.problem().map(|problem| view! {
                <div class="onchain-snapshot-warning" role="status">
                    <strong>{if state.value().is_some() { "最新状态读取失败 · 保留上次快照" } else { "行情读取失败" }}</strong>
                    <span>{problem.message.clone()}</span>
                </div>
            }))}
            <Show when=move || data.state.with(|state| state.value().is_some())
                fallback=move || if data.configuration.awaiting_confirmation() {
                    view! { <div class="workbench-empty-state" role="status"><strong>"当前配置待确认"</strong><span>"上次操作尚未核对完整，当前报价与执行暂不可用。"</span></div> }.into_any()
                } else { data.state.with(|state| match state.problem() {
                    Some(problem) => error_state(problem.message.clone()),
                    None => loading_state("正在读取链上与 交易所 报价…"),
                }) }
            >
                {snapshot_view(data, spread_history, open_execution_setup, active_direction,
                    show_execution_result, execution_clock_ms, execution_evidence_open)}
            </Show>
            {super::cross_chain_control::cross_chain_control(data.execution.cross_chain, execution_clock_ms)}
        </section>
    }
}

fn execution_clock(
    cross_chain: OnchainCrossChainData,
    execution_build: RwSignal<Option<Result<OnchainExecutionBuildResponse, ApiProblem>>>,
    approval_build: RwSignal<Option<Result<OnchainTokenApprovalBuildResponse, String>>>,
    replenishment_plan: RwSignal<Option<Result<OnchainReplenishmentPlanResponse, String>>>,
    replenishment_run: RwSignal<Option<Result<OnchainReplenishmentRun, String>>>,
) -> RwSignal<i64> {
    let clock = RwSignal::new(crate::state::polling::now_ms() as i64);
    let interval = StoredValue::new_local(None::<Interval>);
    let expiry = StoredValue::new_local(None::<Timeout>);
    Effect::new(move |_| {
        cancel_execution_clock(interval, expiry);
        let deadline = execution_build
            .with(|result| {
                result
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map(|build| build.valid_until_ms)
            })
            .into_iter()
            .chain(cross_chain.build.with(|result| {
                result
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map(|build| build.valid_until_ms)
            }))
            .chain(cross_chain.recovery.with(|state| {
                state
                    .rows
                    .iter()
                    .map(|run| run.authorization.valid_until_ms)
                    .max()
            }))
            .chain(cross_chain.recovery.with(|state| state.plans.iter()
                .filter(|plan| matches!(plan.status, shared_types::OnchainCrossChainRecoveryPlanStatus::AwaitingAuthorization
                    | shared_types::OnchainCrossChainRecoveryPlanStatus::Reserved))
                .filter_map(|plan| plan.preview.valid_until_ms).max()))
            .chain(cross_chain.recovery_preview.with(|result| result.as_ref()
                .and_then(|result| result.as_ref().ok()).and_then(|preview| preview.valid_until_ms)))
            .chain(approval_build.with(|result| {
                result
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map(|build| build.valid_until_ms)
            }))
            .chain(replenishment_plan.with(|result| {
                result
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map(|plan| plan.valid_until_ms)
            }))
            .chain(replenishment_run.with(|result| {
                result
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map(|run| run.authorization.valid_until_ms)
            }))
            .max();
        let now_ms = crate::state::polling::now_ms() as i64;
        clock.set(now_ms);
        let Some(deadline) = deadline.filter(|deadline| *deadline > now_ms) else {
            return;
        };
        interval.set_value(Some(Interval::new(250, move || {
            clock.set(crate::state::polling::now_ms() as i64);
        })));
        let delay_ms = deadline
            .saturating_sub(now_ms)
            .clamp(1, i64::from(u32::MAX)) as u32;
        expiry.set_value(Some(Timeout::new(delay_ms, move || {
            clock.set(crate::state::polling::now_ms() as i64);
            interval.update_value(|slot| {
                if let Some(interval) = slot.take() {
                    interval.cancel();
                }
            });
        })));
    });
    on_cleanup(move || {
        cancel_execution_clock(interval, expiry);
    });
    clock
}

fn cancel_execution_clock(
    interval: StoredValue<Option<Interval>, LocalStorage>,
    expiry: StoredValue<Option<Timeout>, LocalStorage>,
) {
    interval.update_value(|slot| {
        if let Some(interval) = slot.take() {
            interval.cancel();
        }
    });
    expiry.update_value(|slot| {
        if let Some(timeout) = slot.take() {
            timeout.cancel();
        }
    });
}

fn snapshot_view(
    data: OnchainData,
    spread_history: SpreadHistory,
    open_execution_setup: Callback<()>,
    active_direction: PreviewContext,
    show_execution_result: Callback<()>,
    execution_clock_ms: RwSignal<i64>,
    execution_evidence_open: RwSignal<bool>,
) -> impl IntoView {
    let snapshot = Memo::new(move |_| {
        let state = data.state.get();
        let mut snapshot = state.value().cloned().unwrap_or_default();
        if let Some(problem) = state.problem() {
            snapshot.quality = OnchainComparisonQuality::Stale;
            snapshot.cross_chain.quality = OnchainCrossChainQuality::Stale;
            snapshot.cross_chain.preview_ready = false;
            snapshot.dex_comparison.quality = OnchainDexComparisonQuality::Stale;
            snapshot.degradation_reasons.insert(0, problem.message.clone());
        }
        snapshot
    });
    view! {
        <div class="onchain-comparison-panel"
            class:is-inactive=move || !snapshot.with(|snapshot| snapshot.config.enabled)
            data-onchain-workspace=move || if snapshot.with(|snapshot| snapshot.config.enabled) { "active" } else { "inactive" }
        >
            <div class="onchain-trading-canvas">
                {spread_chart(spread_history, active_direction)}
                <div class="onchain-decision-workspace"
                    class:is-awaiting=move || snapshot.with(|snapshot| !snapshot.config.enabled || snapshot.comparisons.is_empty())
                >
                    <div class="onchain-decision-lanes" data-table-budget="bounded-small">
                    {move || snapshot.with(|snapshot| {
                        if !snapshot.config.enabled {
                            return inactive_lanes(snapshot);
                        }
                        let mut comparisons = snapshot.comparisons.clone();
                        comparisons.sort_by_key(|row| direction_rank(row.direction));
                        comparison_lanes(&comparisons, snapshot, active_direction)
                    })}
                    {move || snapshot.with(dex_cross_panel)}
                    {cross_chain_panel(snapshot, data, execution_clock_ms)}
                    </div>
                    <aside class="onchain-primary-execution" aria-label="当前方向执行判断">
                        {execution_ticket::ticket(data, open_execution_setup, active_direction,
                            show_execution_result, execution_evidence_open)}
                    </aside>
                </div>
            </div>
            <div class="onchain-execution-overlay" aria-live="polite">
                {move || execution_build_panel(data.execution.execution_build.get(), data, execution_clock_ms)}
                {move || token_approval_panel(data, execution_clock_ms)}
                {move || replenishment_panel(data, execution_clock_ms)}
                {execution_history_panel(data)}
            </div>
        </div>
    }
}

const fn direction_rank(direction: OnchainComparisonDirection) -> u8 {
    match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => 0,
        OnchainComparisonDirection::BuyCexSellOnchain => 1,
    }
}

fn inactive_lanes(snapshot: &OnchainComparisonSnapshot) -> AnyView {
    let (next_step, next_detail, tone) = inactive_next_step(snapshot);
    let chain = chain_label(&snapshot.config.chain);
    let provider = provider_label(&snapshot.config.provider);
    let venue = snapshot.config.cex_venue.to_uppercase();
    let symbol = snapshot.config.cex_symbol.clone();
    view! {
        <div class=format!("onchain-empty-route-book {tone}") role="status">
            <header class="onchain-route-book-header">
                <div><strong>"执行路径"</strong><small>{format!("{chain} / {venue} · 监控暂停")}</small></div>
                <span>"STANDBY"</span>
            </header>
            <div class="onchain-route-placeholder">
                <div class="onchain-route-placeholder-leg is-buy">
                    <span class="onchain-route-book-kind">"链上"</span>
                    <div><strong>{chain}</strong><small>{provider}</small></div>
                    <span>"买入"</span><b class="num">"--"</b>
                </div>
                <div class="onchain-route-placeholder-spread">
                    <small>"费后净差"</small><strong class="num">"--"</strong>
                    <span>{next_step}</span>
                </div>
                <div class="onchain-route-placeholder-leg is-sell">
                    <span class="onchain-route-book-kind">"交易所"</span>
                    <div><strong>{venue}</strong><small>{symbol}</small></div>
                    <span>"卖出"</span><b class="num">"--"</b>
                </div>
            </div>
            <small class="onchain-execution-blocker">{next_detail}</small>
        </div>
    }.into_any()
}

fn inactive_next_step(snapshot: &OnchainComparisonSnapshot) -> (String, String, &'static str) {
    if !snapshot.provider_configured {
        return (
            "完成 报价服务配置".to_owned(),
            snapshot
                .provider_problem
                .clone()
                .unwrap_or_else(|| "确认凭证、路由或节点状态后再启用监控。".to_owned()),
            "is-warning",
        );
    }
    if let Some(problem) = snapshot.provider_problem.clone() {
        return ("等待 报价服务 恢复".to_owned(), problem, "is-warning");
    }
    (
        "启用套利监控".to_owned(),
        "确认左侧配置后打开监控，系统会建立链上报价、交易所 WS 与余额数据依据。".to_owned(),
        "is-neutral",
    )
}

fn comparison_lanes(
    comparisons: &[OnchainCexComparison],
    snapshot: &OnchainComparisonSnapshot,
    active_direction: PreviewContext,
) -> AnyView {
    if comparisons.is_empty() {
        let guidance = empty_decision_guidance(snapshot);
        let technical = guidance
            .technical
            .filter(|problem| problem != &guidance.detail);
        let cex_venue = snapshot.config.cex_venue.to_uppercase();
        let cex_symbol = snapshot.config.cex_symbol.clone();
        let chain = chain_label(&snapshot.config.chain);
        let provider = provider_label(&snapshot.config.provider);
        return view! {
            <div class=format!("onchain-empty-route-book {}", guidance.tone) role="status">
                <header class="onchain-route-book-header">
                    <div>
                        <strong>"执行路径"</strong>
                        <small>{format!("{chain} / {cex_venue} · 等待双源首帧")}</small>
                    </div>
                    <span>{guidance.status}</span>
                </header>
                <div class="onchain-route-placeholder">
                    <div class="onchain-route-placeholder-leg is-buy">
                        <span class="onchain-route-book-kind">"链上"</span>
                        <div>
                            <strong>{chain}</strong>
                            <small>{provider}</small>
                        </div>
                        <span>"买入"</span>
                        <b class="num">"--"</b>
                    </div>
                    <div class="onchain-route-placeholder-spread">
                        <small>"费后净差"</small>
                        <strong class="num">"--"</strong>
                        <span>"双源同步后自动计算"</span>
                    </div>
                    <div class="onchain-route-placeholder-leg is-sell">
                        <span class="onchain-route-book-kind">"交易所"</span>
                        <div>
                            <strong>{cex_venue}</strong>
                            <small>{cex_symbol}</small>
                        </div>
                        <span>"卖出"</span>
                        <b class="num">"--"</b>
                    </div>
                </div>
                <div class="onchain-route-placeholder-state">
                    <small>{guidance.title}</small>
                    <strong>{guidance.detail}</strong>
                    <span>{guidance.next_step}</span>
                    {technical.map(|problem| view! {
                        <details class="onchain-empty-guidance-detail">
                            <summary>"技术详情"</summary>
                            <p>{problem}</p>
                        </details>
                    })}
                </div>
            </div>
        }
        .into_any();
    }
    let selected = comparisons
        .iter()
        .find(|row| row.direction == active_direction.get())
        .or_else(|| comparisons.first());
    let Some(selected) = selected else {
        return ().into_any();
    };
    view! {
            <div id="onchain-route-detail" class="onchain-route-detail" role="tabpanel">
                {comparison_lane(selected, snapshot)}
            </div>
    }
    .into_any()
}

fn cross_chain_panel(
    snapshot: Memo<OnchainComparisonSnapshot>,
    data: OnchainData,
    execution_clock_ms: RwSignal<i64>,
) -> impl IntoView {
    let cross = data.execution.cross_chain;
    let preview_request = Memo::new(move |_| snapshot.with(cross_chain_preview_request));
    let cost_locked = Signal::derive(move || data.saving.get() || cross.building.get() || cross.authorizing.get()
        || cross.submitting.get() || cross.rechecking.get() || cross.recovery_previewing.get() || cross.recovery_mutating.get()
        || !cross.recovery.with(|state| state.can_build(execution_clock_ms.get())));
    let invalidate_costs = Callback::new(move |()| { cross.build.set(None); cross.confirmation.set(String::new()); });
    let approval_market = Callback::new(move |run: shared_types::OnchainTokenApprovalSubmitResponse| {
        snapshot.with_untracked(|snapshot| run.fee_receipts.first().is_some_and(|receipt| {
            snapshot.cross_chain.legs.iter().any(|leg| {
                snapshot.cross_chain.inventory.iter().find(|item| item.chain == leg.from_chain).is_some_and(|item|
                    item.chain.eq_ignore_ascii_case(&receipt.basis.chain)
                    && item.wallet_address.eq_ignore_ascii_case(&receipt.basis.wallet)
                    && receipt.basis.assets.first().is_some_and(|asset|
                        asset.address.eq_ignore_ascii_case(&leg.from_token) && asset.decimals == leg.input_decimals))
            })
        }))
    });
    view! {
        <Show when=move || snapshot.with(|snapshot| snapshot.config.enabled && snapshot.config.cross_chain.enabled)>
        <section class=move || snapshot.with(|snapshot| format!("onchain-cross-chain-route {}", cross_chain_route_quality(snapshot).1)) aria-label="跨链完整流程监控">
            {move || snapshot.with(cross_chain_summary)}
            <div class="onchain-cross-chain-costs">
                {approval_cost_selection::selection_for(data, cross.selected_approvals, cost_locked, invalidate_costs, approval_market)}
                {replenishment_cost_selection::selection_for(data, cross.selected_replenishments, cost_locked, invalidate_costs, true)}
            </div>
            <div class="onchain-cross-chain-preview">
                <button type="button" class="row-action onchain-cross-chain-preview-action"
                    disabled=move || preview_request.get().is_none() || cost_locked.get()
                    on:click=move |_| {
                        if cost_locked.get_untracked() { return; }
                        if let Some(mut request) = data.current_state().value().and_then(cross_chain_preview_request) {
                            request.approval_run_ids = cross.selected_approvals.get_untracked();
                            request.replenishment_run_ids = cross.selected_replenishments.get_untracked();
                            cross.build_preview.run(request);
                        }
                    }>
                    {move || if cross.building.get() { "正在生成…" }
                        else if !cross.recovery.with(|state| state.loaded) { "核对运行记录…" }
                        else if !cross.recovery.with(|state| state.can_build(execution_clock_ms.get())) { "先处理已有运行" }
                        else if preview_request.get().is_some() { "生成完整流程预览" }
                        else { "等待完整四腿报价" }}
                </button>
                {move || cross_chain_build_result(cross.build.get(), execution_clock_ms.get())}
            </div>
        </section>
        </Show>
    }
}

fn cross_chain_route_quality(snapshot: &OnchainComparisonSnapshot) -> (&'static str, &'static str) {
    cross_chain_quality(snapshot.cross_chain.quality)
}

fn cross_chain_preview_request(snapshot: &OnchainComparisonSnapshot) -> Option<OnchainCrossChainBuildRequest> {
    let route = &snapshot.cross_chain;
    if !snapshot.config.enabled || !snapshot.config.cross_chain.enabled
        || !route.preview_ready || !matches!(route.quality, OnchainCrossChainQuality::Fresh
            | OnchainCrossChainQuality::NoNetProfit | OnchainCrossChainQuality::EvidencePending) {
        return None;
    }
    route.quote_observed_at_ms.map(|expected_quote_observed_at_ms| OnchainCrossChainBuildRequest {
        expected_quote_observed_at_ms, approval_run_ids: Vec::new(), replenishment_run_ids: Vec::new(),
    })
}

fn cross_chain_summary(snapshot: &OnchainComparisonSnapshot) -> impl IntoView {
    let route = &snapshot.cross_chain;
    let (status, _) = cross_chain_route_quality(snapshot);
    let path = format!(
        "{} → {} → {}",
        chain_label(&snapshot.config.chain),
        route
            .peer_chain
            .as_deref()
            .map(chain_label)
            .unwrap_or_else(|| "目标链待选择".to_owned()),
        chain_label(&snapshot.config.chain),
    );
    let net = route
        .net_return_bps
        .map_or_else(|| "待核算".to_owned(), percent_label);
    let cost = route
        .total_cost_bps
        .map_or_else(|| "数据依据待补".to_owned(), cost_percent_label);
    let duration = route
        .estimated_duration_seconds
        .map_or_else(|| "时效待取证".to_owned(), duration_label);
    let problem = route.problem.clone();
    let legs = route.legs.iter().map(cross_chain_leg).collect_view();
    let inventory = route
        .inventory
        .iter()
        .map(|item| {
            let (state, tone) = match item.status {
                OnchainCrossChainInventoryStatus::Ready => ("已就绪", "is-positive"),
                OnchainCrossChainInventoryStatus::Insufficient => ("不足", "is-danger"),
                OnchainCrossChainInventoryStatus::Unknown => ("待核对", "is-warning"),
            };
            let label = format!("{} · {}", chain_label(&item.chain), item.asset);
            view! {
                <span class=tone title=item.problem.clone().unwrap_or_default()>
                    <small>{label}</small><strong>{state}</strong>
                </span>
            }
        })
        .collect_view();
    view! {
            <header>
                <span><strong>"跨链完整流程"</strong><small>{path}</small></span>
                <em>{status}</em>
            </header>
            <dl class="onchain-cross-chain-summary">
                <div><dt>"费后回报"</dt><dd class="num">{net}</dd></div>
                <div><dt>"总成本"</dt><dd class="num">{cost}</dd></div>
                <div><dt>"桥接时效"</dt><dd>{duration}</dd></div>
                <div><dt>"执行方式"</dt><dd>"非原子 · 逐腿重报价"</dd></div>
            </dl>
            <div class="onchain-cross-chain-legs">{legs}</div>
            {(!route.inventory.is_empty()).then(|| view! {
                <div class="onchain-cross-chain-inventory"><small>"入场库存"</small>{inventory}</div>
            })}
            {problem.map(|problem| view! { <p title=problem.clone()>{problem.clone()}</p> })}
    }
}

fn cross_chain_build_result(
    result: Option<Result<OnchainCrossChainBuildResponse, ApiProblem>>,
    now_ms: i64,
) -> AnyView {
    match result {
        None => view! {
            <small class="onchain-cross-chain-preview-boundary">
                "预览只锁定四腿数据依据，不签名、不广播、不提交订单"
            </small>
        }
        .into_any(),
        Some(Err(problem)) => view! {
            <div class="onchain-cross-chain-preview-result is-danger" role="alert">
                <strong>"预览生成失败"</strong>
                <span title=problem.message.clone()>{problem.message.clone()}</span>
            </div>
        }
        .into_any(),
        Some(Ok(build)) => {
            let validity = execution_build_validity(build.valid_until_ms, now_ms);
            let build_id = build.build_id.clone();
            let blockers = build.blockers.join("；");
            let blocker_count = build.blockers.len();
            let warnings = build.warnings.join("；");
            let warning_count = build.warnings.len();
            let bridge_contract_count = build.bridge_executions.len();
            let swap_contract_count = build.swap_executions.len();
            let bridge_contracts = build
                .bridge_executions
                .iter()
                .map(|execution| {
                    format!(
                        "第 {} 腿 {}：交易数据已核对，第 {} 腿确认后重报价",
                        execution.position,
                        provider_label(&execution.provider),
                        execution.rebuild_after_position,
                    )
                })
                .collect::<Vec<_>>()
                .join("；");
            let net = build
                .net_return_bps
                .map_or_else(|| "待核算".to_owned(), percent_label);
            view! {
                <div class=format!("onchain-cross-chain-preview-result {}", validity.tone)>
                    <span>
                        <small>"四腿数据依据已锁定"</small>
                        <strong class="num">{net}</strong>
                    </span>
                    <span title=build_id>
                        <small>"有效期"</small>
                        <strong>{validity.label}</strong>
                    </span>
                    <span title=bridge_contracts>
                        <small>"四腿交易合同"</small>
                        <strong>{format!(
                            "{}/4 · 链上 {swap_contract_count}/2 · 跨链 {bridge_contract_count}/2 · 已锁定恢复账本",
                            swap_contract_count + bridge_contract_count
                        )}</strong>
                    </span>
                    <span title=if blockers.is_empty() { warnings } else { blockers }>
                        <small>"执行边界"</small>
                        <strong>{if now_ms >= build.valid_until_ms {
                            "已过期，需重新预览".to_owned()
                        } else if build.submit_ready && !build.monitor_only && build.blockers.is_empty() {
                            format!("可授权 · {warning_count} 项风险提示")
                        } else {
                            format!("{blocker_count} 项阻断 · {warning_count} 项提示")
                        }}</strong>
                    </span>
                </div>
            }
            .into_any()
        }
    }
}

fn cross_chain_leg(leg: &OnchainCrossChainLeg) -> impl IntoView {
    let kind = match leg.kind {
        OnchainCrossChainLegKind::SourceSwap => "源链买入",
        OnchainCrossChainLegKind::OutboundBridge => "Base 跨链",
        OnchainCrossChainLegKind::TargetSwap => "目标链卖出",
        OnchainCrossChainLegKind::ReturnBridge => "Quote 回链",
    };
    let chain = if leg.from_chain.eq_ignore_ascii_case(&leg.to_chain) {
        chain_label(&leg.from_chain).to_owned()
    } else {
        format!(
            "{} → {}",
            chain_label(&leg.from_chain),
            chain_label(&leg.to_chain)
        )
    };
    let expected = raw_amount_label(
        leg.minimum_output_amount_raw
            .as_deref()
            .unwrap_or(&leg.expected_output_amount_raw),
        leg.output_decimals,
        &leg.to_asset,
    );
    let output_label = if leg.minimum_output_amount_raw.is_some() {
        format!("最少 {expected}")
    } else {
        expected
    };
    let cost = match (leg.fee_usd, leg.gas_usd) {
        (Some(fee), Some(gas)) => format!("费 {} · Gas {}", usd(fee), usd(gas)),
        (Some(fee), None) => format!("费 {} · Gas 待取证", usd(fee)),
        (None, Some(gas)) => format!("Gas {}", usd(gas)),
        (None, None) => "成本已计入缓冲".to_owned(),
    };
    view! {
        <article class="onchain-cross-chain-leg">
            <span class="num">{format!("{:02}", leg.position)}</span>
            <div><small>{kind}</small><strong>{format!("{} → {}", leg.from_asset, leg.to_asset)}</strong></div>
            <div><small>{chain}</small><strong>{output_label}</strong></div>
            <small title=leg.route_id.clone().unwrap_or_default()>{format!("{} · {cost}", provider_label(&leg.provider))}</small>
        </article>
    }
}

fn duration_label(seconds: u64) -> String {
    if seconds < 60 {
        return format!("约 {seconds}s");
    }
    let minutes = seconds.div_ceil(60);
    if minutes < 60 {
        format!("约 {minutes}m")
    } else {
        format!("约 {}h {}m", minutes / 60, minutes % 60)
    }
}

const fn cross_chain_quality(quality: OnchainCrossChainQuality) -> (&'static str, &'static str) {
    match quality {
        OnchainCrossChainQuality::Disabled => ("未启用", "is-neutral"),
        OnchainCrossChainQuality::Pending => ("读取中", "is-neutral"),
        OnchainCrossChainQuality::Fresh => ("完整流程有净差", "is-positive"),
        OnchainCrossChainQuality::NoNetProfit => ("无净收益", "is-neutral"),
        OnchainCrossChainQuality::Stale => ("已过期", "is-warning"),
        OnchainCrossChainQuality::PeerMissing => ("目标缺失", "is-danger"),
        OnchainCrossChainQuality::EvidencePending => ("仅监控", "is-warning"),
        OnchainCrossChainQuality::UpstreamUnavailable => ("来源异常", "is-danger"),
    }
}

fn dex_cross_panel(snapshot: &OnchainComparisonSnapshot) -> AnyView {
    if !snapshot.config.enabled || !snapshot.config.dex_comparison.enabled {
        return ().into_any();
    }
    let comparison = &snapshot.dex_comparison;
    let (status, tone) = dex_cross_quality(comparison.quality);
    let problem = comparison.problem.clone();
    let routes = comparison
        .routes
        .iter()
        .map(|route| dex_cross_route(route, snapshot))
        .collect_view();
    view! {
        <section class=format!("onchain-dex-cross {tone}") aria-label="同链 链上 对比">
            <header>
                <span><strong>"DEX ↔ DEX"</strong><small>"同链 · 同合约 · 同数量"</small></span>
                <em>{status}</em>
            </header>
            <div class="onchain-dex-cross-routes">
                {routes}
                {(comparison.routes.is_empty()).then(|| view! {
                    <div class="onchain-dex-cross-empty">
                        <strong>"等待第二报价源"</strong>
                        <span>{problem.clone().unwrap_or_else(|| "正在读取同数量往返报价".to_owned())}</span>
                    </div>
                })}
            </div>
            {problem.map(|problem| view! { <p title=problem.clone()>{problem.clone()}</p> })}
        </section>
    }
    .into_any()
}

fn dex_cross_route(
    route: &OnchainDexRouteComparison,
    snapshot: &OnchainComparisonSnapshot,
) -> impl IntoView {
    let direction = match route.direction {
        OnchainDexComparisonDirection::BuyPrimarySellPeer => "主源买 → 第二源卖",
        OnchainDexComparisonDirection::BuyPeerSellPrimary => "第二源买 → 主源卖",
    };
    let buy = provider_label(&route.buy_provider);
    let sell = provider_label(&route.sell_provider);
    let amount = raw_amount_label(
        &route.input_quote_amount_raw,
        snapshot.config.quote_decimals,
        &snapshot.config.quote_token,
    );
    let edge = route
        .net_return_bps
        .map_or_else(|| "待核算".to_owned(), percent_label);
    let tone = if route.net_return_bps.is_some_and(|value| value > 0.0) {
        "is-positive"
    } else {
        "is-neutral"
    };
    let detail = route.problem.clone().unwrap_or_else(|| {
        format!(
            "毛差 {} · 双笔成本 {}",
            percent_label(route.gross_return_bps),
            route
                .total_cost_bps
                .map_or_else(|| "待汇率".to_owned(), cost_percent_label),
        )
    });
    view! {
        <div class="onchain-dex-cross-route" title=detail>
            <span><small>{direction}</small><strong>{format!("{buy} → {sell}")}</strong></span>
            <span><small>"测试金额"</small><strong class="num">{amount}</strong></span>
            <span class=tone><small>"费后回报"</small><strong class="num">{edge}</strong></span>
        </div>
    }
}

const fn dex_cross_quality(quality: OnchainDexComparisonQuality) -> (&'static str, &'static str) {
    match quality {
        OnchainDexComparisonQuality::Disabled => ("未启用", "is-neutral"),
        OnchainDexComparisonQuality::Pending => ("读取中", "is-neutral"),
        OnchainDexComparisonQuality::Fresh => ("已核算", "is-positive"),
        OnchainDexComparisonQuality::NoNetProfit => ("无净收益", "is-neutral"),
        OnchainDexComparisonQuality::Stale => ("已过期", "is-warning"),
        OnchainDexComparisonQuality::DuplicateRoute => ("同路由", "is-warning"),
        OnchainDexComparisonQuality::EvidencePending => ("仅监控", "is-warning"),
        OnchainDexComparisonQuality::UpstreamUnavailable => ("来源异常", "is-danger"),
    }
}

const fn compact_direction_label(direction: OnchainComparisonDirection) -> &'static str {
    match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => "链买 / 交易所 卖",
        OnchainComparisonDirection::BuyCexSellOnchain => "交易所 买 / 链卖",
    }
}

fn comparison_lane(
    row: &OnchainCexComparison,
    snapshot: &OnchainComparisonSnapshot,
) -> impl IntoView {
    let raw_observation = raw_observation_mode(snapshot);
    let primary_bps = if raw_observation {
        row.gross_spread_bps
    } else {
        row.net_spread_bps
    };
    let retained = unconfirmed_quote(snapshot);
    let net_tone = if retained {
        "is-neutral"
    } else if raw_observation {
        "is-observation"
    } else if row.net_spread_bps > 0.0 {
        "is-positive"
    } else {
        "is-negative"
    };
    let lane_class = if raw_observation {
        "onchain-direction-lane is-observation"
    } else {
        "onchain-direction-lane"
    };
    let primary_label = if retained {
        "上次测算"
    } else if raw_observation {
        "原始价差"
    } else {
        "费后净差"
    };
    let edge_detail = if raw_observation {
        "原始价格对比 · 未进行费后收益判断".to_owned()
    } else {
        format!(
            "毛差 {} · 总成本 {}",
            percent_label(row.gross_spread_bps),
            cost_percent_label(row.total_cost_bps),
        )
    };
    let onchain_price = price_label(row.onchain_price);
    let cex_price = price_label(row.cex_price);
    let route = direction_context(row.direction, snapshot);
    let ((onchain_side, onchain_tone), (cex_side, cex_tone)) = route_leg_sides(row.direction);
    let onchain_leg = RouteLegView {
        kind: "链上",
        venue: chain_label(&snapshot.config.chain),
        source: provider_label(&snapshot.config.provider),
        side: onchain_side,
        price: onchain_price,
        tone: onchain_tone,
    };
    let cex_leg = RouteLegView {
        kind: "交易所",
        venue: snapshot.config.cex_venue.to_uppercase(),
        source: cex_source_label(&snapshot.cex_source),
        side: cex_side,
        price: cex_price,
        tone: cex_tone,
    };
    view! {
        <article class=lane_class>
            <header class="onchain-route-book-header">
                <div>
                    <strong>"执行路径"</strong>
                    <small>{route}</small>
                </div>
                <span>{if matches!(snapshot.quality, OnchainComparisonQuality::Stale | OnchainComparisonQuality::Pending | OnchainComparisonQuality::UpstreamUnavailable) { "报价待确认" } else { "实时预览" }}</span>
            </header>
            <div class="onchain-route-book" aria-label=direction_label(row.direction)>
                {route_book_leg(onchain_leg, retained)}
                <div class=format!("onchain-route-book-edge {net_tone}")>
                    <span>{primary_label}</span>
                    <strong class="num">{percent_label(primary_bps)}</strong>
                    <small>{edge_detail}</small>
                </div>
                {route_book_leg(cex_leg, retained)}
            </div>
        </article>
    }
}

struct RouteLegView {
    kind: &'static str,
    venue: String,
    source: String,
    side: &'static str,
    price: String,
    tone: &'static str,
}

fn route_book_leg(leg: RouteLegView, retained: bool) -> impl IntoView {
    view! {
        <div class="onchain-route-book-leg">
            <span class="onchain-route-book-kind">{leg.kind}</span>
            <div class="onchain-route-book-venue">
                <strong>{leg.venue}</strong>
                <small>{leg.source}</small>
            </div>
            <span class=format!("onchain-route-book-side {}", leg.tone)>{leg.side}</span>
            <div class="onchain-route-book-price">
                <small>{if retained { "上次价格" } else { "预估成交价" }}</small>
                <strong class="num">{leg.price}</strong>
            </div>
        </div>
    }
}

const fn route_leg_sides(
    direction: OnchainComparisonDirection,
) -> ((&'static str, &'static str), (&'static str, &'static str)) {
    match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => (("买入", "is-buy"), ("卖出", "is-sell")),
        OnchainComparisonDirection::BuyCexSellOnchain => (("卖出", "is-sell"), ("买入", "is-buy")),
    }
}


fn comparison_capital_label(snapshot: &OnchainComparisonSnapshot) -> String {
    snapshot
        .quote_evidence
        .iter()
        .find(|row| {
            token_identity_matches(
                &snapshot.config.chain,
                &row.input_mint,
                &snapshot.config.quote_mint,
            ) && token_identity_matches(
                &snapshot.config.chain,
                &row.output_mint,
                &snapshot.config.base_mint,
            )
        })
        .map_or_else(
            || {
                raw_amount_label(
                    &snapshot.config.quote_amount_raw,
                    snapshot.config.quote_decimals,
                    &snapshot.config.quote_token,
                )
            },
            |row| {
                raw_amount_label(
                    &row.input_amount_raw,
                    snapshot.config.quote_decimals,
                    &snapshot.config.quote_token,
                )
            },
        )
}

fn direction_context(
    direction: OnchainComparisonDirection,
    snapshot: &OnchainComparisonSnapshot,
) -> String {
    let venue = snapshot.config.cex_venue.to_uppercase();
    let base = snapshot.config.base_token.trim();
    let quote = snapshot.config.quote_token.trim();
    let cex_base = onchain_cex_base_token(&snapshot.config.cex_symbol).unwrap_or("未知");
    if !cex_base.eq_ignore_ascii_case(base) {
        return match direction {
            OnchainComparisonDirection::BuyOnchainSellCex => {
                format!("链上买 {} · {venue} 卖 {cex_base} · 独立市场观察", base)
            }
            OnchainComparisonDirection::BuyCexSellOnchain => {
                format!("{venue} 买 {cex_base} · 链上卖 {} · 独立市场观察", base)
            }
        };
    }
    if raw_observation_mode(snapshot) {
        return match direction {
            OnchainComparisonDirection::BuyOnchainSellCex => {
                format!("链上买 {base} · {venue} 卖出同量 · 仅原始观察")
            }
            OnchainComparisonDirection::BuyCexSellOnchain => {
                format!("{venue} 买 {base} · 链上卖出同量 · 仅原始观察")
            }
        };
    }
    match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            format!("{} 买 {} · {venue} 卖出同量", quote, base)
        }
        OnchainComparisonDirection::BuyCexSellOnchain => {
            format!("{venue} 买 {base} · 链上卖出同量")
        }
    }
}

fn raw_observation_detail(config: &OnchainComparisonConfig) -> String {
    if !config.base_identity_resolved {
        return format!(
            "链上 Base {} 已读取合约与精度，但币种符号尚未核对；当前只比较原始价格，不判断净收益",
            config.base_token
        );
    }
    if !config.quote_identity_resolved {
        return format!(
            "链上 Quote {} 已读取合约与精度，但币种符号尚未核对；当前只比较原始价格，不判断净收益",
            config.quote_token
        );
    }
    if config.spread_alert.mode == OnchainSpreadAlertMode::RawObservation
        && onchain_cex_pair_matches(config)
    {
        return "当前明确选择原始观察模式；资产与 Quote 已匹配，但页面只比较名义价格，不扣费用、不判断可执行利润".to_owned();
    }
    let cex_base = onchain_cex_base_token(&config.cex_symbol).unwrap_or("未知");
    if !cex_base.eq_ignore_ascii_case(config.base_token.trim()) {
        return format!(
            "链上 Base 为 {}，交易所 Base 为 {cex_base}；当前只比较两个独立市场的原始价格，不判断净收益",
            config.base_token
        );
    }
    let cex_quote = onchain_cex_quote_token(&config.cex_symbol).unwrap_or("未知");
    if !onchain_quotes_match(config) {
        return format!(
            "链上 Quote 为 {}，交易所 Quote 为 {cex_quote}；当前只比较未换算的原始价格，不判断净收益",
            config.quote_token
        );
    }
    "当前自定义市场只用于原始价格观察".to_owned()
}

fn raw_observation_next_step(config: &OnchainComparisonConfig) -> String {
    if !config.base_identity_resolved || !config.quote_identity_resolved {
        return "等待链上币种身份基础资料自动恢复；监控期间不会进入执行".to_owned();
    }
    if config.spread_alert.mode == OnchainSpreadAlertMode::RawObservation
        && onchain_cex_pair_matches(config)
    {
        return "如需核对费后利润并构建交易计划，切换为“费后机会”模式".to_owned();
    }
    let cex_base = onchain_cex_base_token(&config.cex_symbol).unwrap_or("未知");
    if !cex_base.eq_ignore_ascii_case(config.base_token.trim()) {
        return format!(
            "如需执行，选择以 {} 为 Base 的 交易所 交易对",
            config.base_token
        );
    }
    format!(
        "如需执行，选择 {}/{} 同 Quote 市场，或补充明确汇率数据依据",
        config.base_token, config.quote_token,
    )
}

fn token_identity_matches(chain: &str, left: &str, right: &str) -> bool {
    if chain.eq_ignore_ascii_case("solana") {
        left == right
    } else {
        left.eq_ignore_ascii_case(right)
    }
}


fn unconfirmed_quote(snapshot: &OnchainComparisonSnapshot) -> bool {
    matches!(snapshot.quality, OnchainComparisonQuality::Pending
        | OnchainComparisonQuality::Stale | OnchainComparisonQuality::UpstreamUnavailable)
}

fn execution_ticket_metrics(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
) -> impl IntoView {
    let raw = raw_observation_mode(snapshot);
    let (primary_label, primary_value, primary_tone, secondary_label, secondary_value) = if unconfirmed_quote(snapshot) {
        ("预估净收益", "--".to_owned(), "", "本次可做", "--".to_owned())
    } else if raw {
        (
            "比较资金",
            comparison_capital_label(snapshot),
            "is-observation",
            "当前方向",
            compact_direction_label(comparison.direction).to_owned(),
        )
    } else {
        let expected_profit = expected_profit_usd(
            comparison.observable_notional_usd,
            comparison.net_spread_bps,
        );
        (
            "预估净收益",
            usd(expected_profit),
            if expected_profit > 0.0 {
                "is-positive"
            } else if expected_profit < 0.0 {
                "is-negative"
            } else {
                ""
            },
            "本次可做",
            usd(comparison.observable_notional_usd),
        )
    };
    view! {
        <dl class="onchain-ticket-summary" aria-label="执行规模与预估收益">
            <div class="is-primary">
                <dt>{primary_label}</dt>
                <dd class=format!("num {primary_tone}")>{primary_value}</dd>
            </div>
            <div><dt>{secondary_label}</dt><dd class="num">{secondary_value}</dd></div>
        </dl>
    }
}

fn expected_profit_usd(observable_notional_usd: f64, net_spread_bps: f64) -> f64 {
    observable_notional_usd * net_spread_bps / 10_000.0
}


#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildActionState {
    SetupRequired,
    Replenishable,
    Buildable,
    Blocked,
}

const fn execution_state_badge(action_state: BuildActionState) -> (&'static str, &'static str) {
    match action_state {
        BuildActionState::SetupRequired => ("待接入", "is-warning"),
        BuildActionState::Replenishable => ("可补仓", "is-warning"),
        BuildActionState::Buildable => ("可构建", "is-positive"),
        BuildActionState::Blocked => ("已阻断", "is-danger"),
    }
}

fn build_action_label(
    quality: OnchainComparisonQuality,
    net_spread_bps: f64,
    min_net_spread_bps: f64,
    action_state: BuildActionState,
    needs_depth_probe: bool,
    building: bool,
) -> &'static str {
    if building {
        return "构建中…";
    }
    if action_state == BuildActionState::SetupRequired {
        return "打开执行接入";
    }
    if action_state == BuildActionState::Replenishable {
        return if building {
            "核对补仓中…"
        } else {
            "生成补仓计划"
        };
    }
    if action_state == BuildActionState::Blocked {
        if matches!(
            quality,
            OnchainComparisonQuality::Pending
                | OnchainComparisonQuality::Stale
                | OnchainComparisonQuality::UpstreamUnavailable
        ) {
            return "等待实时报价";
        }
        if quality == OnchainComparisonQuality::MappingInvalid {
            return "修正资产映射";
        }
        if net_spread_bps <= 0.0 || net_spread_bps < min_net_spread_bps.max(0.0) {
            return "等待费后盈利";
        }
        return "补齐执行数据依据";
    }
    if needs_depth_probe {
        "核对深度并构建"
    } else {
        "构建交易计划"
    }
}

fn direction_market_gate(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
) -> Option<(&'static str, &'static str, String)> {
    if snapshot.quality == OnchainComparisonQuality::Pending {
        return Some((
            "等待首次报价",
            "is-neutral",
            "链上报价与 交易所 WS 最优价正在形成，暂不使用未完成快照判断收益".to_owned(),
        ));
    }
    if snapshot.quality == shared_types::OnchainComparisonQuality::Stale {
        return Some((
            if snapshot.onchain_freshness_ms.is_none() || snapshot.cex_freshness_ms.is_none() {
                "时效待确认"
            } else { "报价已过期" },
            "is-warning",
            snapshot.degradation_reasons.first().cloned().unwrap_or_else(||
                "链上报价或 交易所 WS 最优价已超过新鲜度上限，等待下一份实时数据".to_owned()),
        ));
    }
    if snapshot.quality == OnchainComparisonQuality::UpstreamUnavailable {
        let problem = snapshot
            .provider_problem
            .as_ref()
            .or(snapshot.cex_problem.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                "链上报价 报价服务 或 交易所 行情源暂不可用，系统会自动重试".to_owned()
            });
        return Some(("行情源不可用", "is-danger", problem));
    }
    if snapshot.quality == OnchainComparisonQuality::MappingInvalid {
        return Some((
            "资产映射阻断",
            "is-danger",
            snapshot
                .degradation_reasons
                .first()
                .cloned()
                .unwrap_or_else(|| "链上资产身份与所选 交易所 市场映射未通过".to_owned()),
        ));
    }
    if raw_observation_mode(snapshot) {
        let problem = readiness_for(snapshot, comparison.direction)
            .and_then(|row| row.cex_instrument.problem.clone())
            .unwrap_or_else(|| raw_observation_detail(&snapshot.config));
        let label = if onchain_cex_pair_matches(&snapshot.config) {
            "原始观察模式"
        } else {
            "自定义原始观察"
        };
        return Some((label, "is-warning", problem));
    }
    if let Some(instrument) = readiness_for(snapshot, comparison.direction)
        .map(|row| &row.cex_instrument)
        .filter(|instrument| instrument.problem.is_some())
    {
        let (label, tone) = match instrument.status {
            OnchainCexInstrumentStatus::Syncing => ("执行规格同步中", "is-neutral"),
            OnchainCexInstrumentStatus::Stale => ("执行规格已过期", "is-warning"),
            OnchainCexInstrumentStatus::Unavailable => ("执行规格刷新失败", "is-danger"),
            OnchainCexInstrumentStatus::Unlisted => ("交易所 未挂牌", "is-danger"),
            OnchainCexInstrumentStatus::Unsupported => ("执行规格未接入", "is-danger"),
            OnchainCexInstrumentStatus::Ready | OnchainCexInstrumentStatus::Incomplete => (
                "执行规格阻断",
                if instrument.ready {
                    "is-warning"
                } else {
                    "is-danger"
                },
            ),
        };
        return Some((label, tone, instrument.problem.clone().unwrap_or_default()));
    }
    if comparison.net_spread_bps <= 0.0 {
        return Some((
            "当前不可盈利",
            "is-danger",
            format!(
                "费后净差 {}，无法覆盖手续费、滑点与 Gas",
                percent_label(comparison.net_spread_bps)
            ),
        ));
    }
    let minimum_bps = snapshot.config.spread_alert.min_net_spread_bps.max(0.0);
    if comparison.net_spread_bps < minimum_bps {
        return Some((
            "收益未达门槛",
            "is-warning",
            format!(
                "费后净差 {}，低于配置门槛 {}",
                percent_label(comparison.net_spread_bps),
                cost_percent_label(minimum_bps)
            ),
        ));
    }
    None
}

fn raw_observation_mode(snapshot: &OnchainComparisonSnapshot) -> bool {
    snapshot.config.spread_alert.mode == OnchainSpreadAlertMode::RawObservation
        || matches!(
            snapshot.quality,
            OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
        )
}

fn depth_probe_note(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
) -> Option<String> {
    (comparison.observable_notional_usd < snapshot.config.min_liquidity_usd).then(|| {
        format!(
            "最优档预览 {}；点击构建读取 100 档并核对目标 {}",
            usd(comparison.observable_notional_usd),
            usd(snapshot.config.min_liquidity_usd)
        )
    })
}

fn inventory_chip(row: &shared_types::OnchainInventoryEvidence) -> impl IntoView {
    let location = match row.location {
        OnchainInventoryLocation::Onchain => "链上",
        OnchainInventoryLocation::Cex => "交易所",
    };
    let (value, tone) = inventory_chip_state(row);
    let title = row
        .problem
        .as_ref()
        .map(|problem| format!("{} · {problem}", inventory_label(row)))
        .unwrap_or_else(|| inventory_label(row));
    view! {
        <span class=format!("onchain-inventory-chip {tone}") title=title>
            <small>{format!("{location} {}", row.asset)}</small>
            <strong class="num">{value}</strong>
        </span>
    }
}

fn transfer_chip(row: &shared_types::OnchainTransferEvidence) -> impl IntoView {
    let action = match row.direction {
        OnchainTransferDirection::WithdrawToChain => "提至链上",
        OnchainTransferDirection::DepositToCex => "充入 交易所",
    };
    let (value, tone) = match row.status {
        OnchainTransferStatus::Ready => ("可用", "is-positive"),
        OnchainTransferStatus::Refreshing => ("读取中", "is-warning"),
        OnchainTransferStatus::Unknown => ("待核对", "is-warning"),
        OnchainTransferStatus::Blocked => ("不可用", "is-danger"),
        OnchainTransferStatus::Unsupported => ("未接入", "is-danger"),
    };
    let network = row.network.as_deref().unwrap_or("网络待核对");
    let fee = row.fee.map_or_else(
        || "手续费待核对".to_owned(),
        |fee| format!("手续费 {fee:.8} {}", row.asset),
    );
    let problem = row
        .problem
        .as_deref()
        .map(|problem| format!(" · {problem}"))
        .unwrap_or_default();
    let confirmations = match (row.credit_confirmations, row.unlock_confirmations) {
        (Some(credit), Some(unlock)) => format!(" · 到账 {credit} 确认 / 解锁 {unlock} 确认"),
        (Some(credit), None) => format!(" · 到账 {credit} 确认"),
        (None, Some(unlock)) => format!(" · 解锁 {unlock} 确认"),
        (None, None) => String::new(),
    };
    let network_status = row
        .network_status
        .as_deref()
        .map(|status| format!(" · 网络 {status}"))
        .unwrap_or_default();
    let title = format!(
        "{} {} {} · {}{}{}{}",
        row.venue.to_ascii_uppercase(),
        row.asset,
        network,
        fee,
        confirmations,
        network_status,
        problem,
    );
    view! {
        <span class=format!("onchain-inventory-chip {tone}") title=title>
            <small>{format!("补仓 {} · {action}", row.asset)}</small>
            <strong>{value}</strong>
        </span>
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutionReadinessFact {
    label: &'static str,
    value: String,
    tone: &'static str,
    title: String,
}

fn execution_readiness_strip(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
    row: &OnchainDirectionReadiness,
) -> impl IntoView {
    view! {
        <dl class="onchain-readiness-strip" aria-label="执行准备度五步执行条件">
            {execution_readiness_facts(snapshot, comparison, row)
                .into_iter()
                .map(execution_readiness_fact)
                .collect_view()}
        </dl>
    }
}

fn execution_readiness_fact(fact: ExecutionReadinessFact) -> impl IntoView {
    let ExecutionReadinessFact {
        label,
        value,
        tone,
        title,
    } = fact;
    view! {
        <div class=format!("onchain-readiness-fact {tone}") title=title>
            <dt>{label}</dt>
            <dd>{value}</dd>
        </div>
    }
}

fn execution_readiness_facts(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
    row: &OnchainDirectionReadiness,
) -> Vec<ExecutionReadinessFact> {
    let (profit_value, profit_tone, profit_title) = profit_readiness(snapshot, comparison);
    let (inventory_value, inventory_tone, inventory_title) = inventory_readiness(row);
    let (instrument, instrument_detail) = cex_instrument_label(&row.cex_instrument);
    let instrument_value = match row.cex_instrument.status {
        OnchainCexInstrumentStatus::Ready if row.cex_instrument.problem.is_none() => "已核对",
        OnchainCexInstrumentStatus::Ready => "执行受限",
        OnchainCexInstrumentStatus::Syncing => "同步中",
        OnchainCexInstrumentStatus::Unlisted => "未挂牌",
        OnchainCexInstrumentStatus::Incomplete => "不完整",
        OnchainCexInstrumentStatus::Stale => "已过期",
        OnchainCexInstrumentStatus::Unavailable => "刷新失败",
        OnchainCexInstrumentStatus::Unsupported => "未接入",
    };
    let depth_title = depth_probe_note(snapshot, comparison).unwrap_or_else(|| {
        "盈利与静态数据依据通过后，点击构建才读取 firm quote 和 交易所 100 档深度".to_owned()
    });
    let (path_value, path_tone) = path_readiness_label(row);

    vec![
        ExecutionReadinessFact {
            label: "收益",
            value: profit_value,
            tone: profit_tone,
            title: profit_title,
        },
        ExecutionReadinessFact {
            label: "余额",
            value: inventory_value,
            tone: inventory_tone,
            title: inventory_title,
        },
        ExecutionReadinessFact {
            label: "规格",
            value: instrument_value.to_owned(),
            tone: cex_instrument_tone(&row.cex_instrument),
            title: instrument_detail.unwrap_or(instrument),
        },
        ExecutionReadinessFact {
            label: "深度",
            value: "构建时".to_owned(),
            tone: "is-neutral",
            title: depth_title,
        },
        ExecutionReadinessFact {
            label: "路径",
            value: path_value,
            tone: path_tone,
            title: row.path.summary.clone(),
        },
    ]
}

fn path_readiness_label(row: &OnchainDirectionReadiness) -> (String, &'static str) {
    let legs = match row.path.kind {
        OnchainPathKind::DirectTwoLeg => "2腿",
        OnchainPathKind::QuoteConvertedThreeLeg => "3腿",
    };
    match row.path.availability {
        OnchainPathAvailability::ReadyToBuild => (format!("{legs}可构建"), "is-positive"),
        OnchainPathAvailability::SetupRequired => (format!("{legs}待接入"), "is-warning"),
        OnchainPathAvailability::Replenishable => (format!("{legs}可补仓"), "is-warning"),
        OnchainPathAvailability::TransferUnprofitable => (format!("{legs}搬运后亏损"), "is-danger"),
        OnchainPathAvailability::InventoryRequired => (format!("{legs}缺库存"), "is-danger"),
        OnchainPathAvailability::EvidencePending => (format!("{legs}待数据依据"), "is-warning"),
        OnchainPathAvailability::MonitoringOnly => ("仅监控".to_owned(), "is-warning"),
        OnchainPathAvailability::Blocked => ("已阻断".to_owned(), "is-danger"),
    }
}

fn path_inventory_guidance(row: &OnchainDirectionReadiness) -> Option<String> {
    match row.path.availability {
        OnchainPathAvailability::Replenishable => {
            let economics = row.path.post_transfer_net_profit_usd.map_or_else(
                || "搬运后收益待核对".to_owned(),
                |profit| format!("搬运后预计净利 ${profit:.4}"),
            );
            Some(format!(
                "当前库存不足；官方充提路径可用，{economics}，到账后再构建"
            ))
        }
        OnchainPathAvailability::TransferUnprofitable => {
            let cost = row.path.transfer_cost_usd.unwrap_or_default();
            let profit = row.path.post_transfer_net_profit_usd.unwrap_or_default();
            Some(format!(
                "当前价差无法覆盖搬运成本 ${cost:.4}；搬运后预计净利 ${profit:.4}，已阻止补仓"
            ))
        }
        OnchainPathAvailability::EvidencePending if !row.path.replenishment.is_empty() => {
            Some("当前库存不足；正在核对对应币种的官方充提网络、费用与合约身份".to_owned())
        }
        _ => None,
    }
}

fn profit_readiness(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
) -> (String, &'static str, String) {
    let net = percent_label(comparison.net_spread_bps);
    let minimum = cost_percent_label(snapshot.config.spread_alert.min_net_spread_bps.max(0.0));
    if matches!(
        snapshot.quality,
        OnchainComparisonQuality::Pending
            | OnchainComparisonQuality::Stale
            | OnchainComparisonQuality::UpstreamUnavailable
    ) {
        return (
            "待报价".to_owned(),
            "is-warning",
            format!("当前费后净差 {net}；双源报价恢复新鲜后重新判断"),
        );
    }
    if comparison.net_spread_bps <= 0.0 {
        return (
            "未盈利".to_owned(),
            "is-danger",
            format!("费后净差 {net}，不能覆盖交易费、滑点与 Gas"),
        );
    }
    if comparison.net_spread_bps < snapshot.config.spread_alert.min_net_spread_bps.max(0.0) {
        return (
            "未达门槛".to_owned(),
            "is-warning",
            format!("费后净差 {net}，低于配置门槛 {minimum}"),
        );
    }
    (
        "已通过".to_owned(),
        "is-positive",
        format!("费后净差 {net}，已达到配置门槛 {minimum}"),
    )
}

fn inventory_readiness(row: &OnchainDirectionReadiness) -> (String, &'static str, String) {
    let inventory_ready = row
        .inventory
        .iter()
        .filter(|item| item.status == OnchainInventoryStatus::Ready)
        .count();
    let hard_blocker = row
        .inventory
        .iter()
        .any(|item| item.status == OnchainInventoryStatus::Insufficient);
    let tone = if inventory_ready == row.inventory.len() && !row.inventory.is_empty() {
        "is-positive"
    } else if hard_blocker {
        "is-danger"
    } else {
        "is-warning"
    };
    let title = if row.inventory.is_empty() {
        "尚未生成链上、交易所 与 Gas 余额数据依据".to_owned()
    } else {
        row.inventory
            .iter()
            .map(inventory_label)
            .collect::<Vec<_>>()
            .join("；")
    };
    (
        if row.inventory.is_empty() { "待核对".to_owned() }
        else { format!("{inventory_ready}/{}", row.inventory.len()) },
        tone,
        title,
    )
}

fn execution_overall_state(
    snapshot: &OnchainComparisonSnapshot,
    comparison: &OnchainCexComparison,
    row: &OnchainDirectionReadiness,
    buildable: bool,
) -> (&'static str, &'static str) {
    let (_, profit_tone, _) = profit_readiness(snapshot, comparison);
    if profit_tone != "is-positive" {
        return ("收益阻断", profit_tone);
    }
    if cex_instrument_tone(&row.cex_instrument) != "is-positive" {
        return ("规格待补", cex_instrument_tone(&row.cex_instrument));
    }
    if row.path.availability == OnchainPathAvailability::Replenishable {
        return ("可补仓后执行", "is-warning");
    }
    if row.path.availability == OnchainPathAvailability::EvidencePending
        && !row.path.replenishment.is_empty()
    {
        return ("充提待核对", "is-warning");
    }
    let (_, inventory_tone, _) = inventory_readiness(row);
    if inventory_tone != "is-positive" {
        return (
            if inventory_tone == "is-danger" {
                "余额不足"
            } else {
                "余额待核对"
            },
            inventory_tone,
        );
    }
    if !buildable {
        return ("构建待补", "is-warning");
    }
    if snapshot.execution_readiness.chain_submission_ready
        && snapshot.execution_readiness.cex_live_mode_ready
    {
        ("可构建", "is-positive")
    } else {
        ("可构建 · 提交待接入", "is-warning")
    }
}

fn execution_cost_breakdown(comparison: &OnchainCexComparison) -> impl IntoView {
    let variable_cost_bps = comparison.total_cost_bps - comparison.gas_bps;
    let fee_rates = if comparison.quote_conversion_fee_bps > 0.0 {
        format!(
            "交易所 费率 {}；换币费率 {}",
            cost_percent_label(comparison.cex_fee_bps),
            cost_percent_label(comparison.quote_conversion_fee_bps),
        )
    } else {
        format!("交易所 费率 {}", cost_percent_label(comparison.cex_fee_bps))
    };
    let variable_cost_title = format!(
        "{}；滑点预留 {}；按各笔交易金额核算，不直接相加费率",
        fee_rates,
        cost_percent_label(comparison.slippage_bps),
    );
    let gas_title = format!(
        "预计 Gas {}，折合 {}",
        usd(comparison.gas_usd),
        cost_percent_label(comparison.gas_bps),
    );
    view! {
        <dl class="onchain-cost-breakdown" aria-label="费后成本明细">
            <div title=variable_cost_title>
                <dt>"交易与滑点"</dt>
                <dd class="num">{cost_percent_label(variable_cost_bps)}</dd>
            </div>
            <div title=gas_title>
                <dt>"Gas"</dt>
                <dd class="num">{usd(comparison.gas_usd)}</dd>
            </div>
            <div title="交易费、换币费、滑点预留与 Gas 折合总成本">
                <dt>"总成本"</dt>
                <dd class="num">{cost_percent_label(comparison.total_cost_bps)}</dd>
            </div>
        </dl>
    }
}

fn inventory_chip_state(row: &shared_types::OnchainInventoryEvidence) -> (String, &'static str) {
    match row.status {
        OnchainInventoryStatus::Ready => (
            row.available.map_or_else(
                || "可用".to_owned(),
                |value| format!("可用 {}", compact_balance(value)),
            ),
            "is-positive",
        ),
        OnchainInventoryStatus::Insufficient => (
            format!(
                "{} / 需 {}",
                row.available
                    .map_or_else(|| "0".to_owned(), compact_balance),
                compact_balance(row.required),
            ),
            "is-danger",
        ),
        OnchainInventoryStatus::Unknown => ("待核对".to_owned(), "is-warning"),
    }
}

fn compact_balance(value: f64) -> String {
    if value.abs() >= 1_000.0 {
        format!("{value:.2}")
    } else if value.abs() >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.6}")
    }
}

fn execution_build_panel(
    result: Option<Result<OnchainExecutionBuildResponse, ApiProblem>>,
    data: OnchainData,
    execution_clock_ms: RwSignal<i64>,
) -> AnyView {
    let Some(result) = result else {
        return ().into_any();
    };
    match result {
        Err(problem) => {
            let approval_required = problem.code == "ONCHAIN_TOKEN_APPROVAL_REQUIRED";
            let code = problem.code;
            let message = problem.message;
            let problem_title = message.clone();
            let guidance = build_failure_guidance(&code);
            view! {
                <div class=if approval_required { "onchain-build-result is-warning" } else { "onchain-build-result is-danger" } role="alert">
                    <div class="onchain-build-error-heading">
                        <small class="num">{code}</small>
                        <strong>{if approval_required { "需要代币授权" } else { "交易计划未构建" }}</strong>
                    </div>
                    <span title=problem_title>{message}</span>
                    <small>{guidance}</small>
                </div>
            }
            .into_any()
        }
        Ok(build) => {
            let validity = Memo::new(move |_| execution_build_validity(build.valid_until_ms, execution_clock_ms.get()));
            let steps = execution_plan_steps(&build);
            let leg_count = steps.len();
            let build_id = build.build_id.clone();
            let used_id = StoredValue::new(build_id.clone());
            let used = Memo::new(move |_| data.execution.history.state.build_used(&used_id.get_value()));
            let submit_ready = build.submit_ready;
            let blocker = build.blockers.first().cloned().unwrap_or_else(|| "执行计划已通过提交准备度校验".to_owned());
            let blocker_title = blocker.clone();
            let submit_blocker = blocker.clone();
            view! {
                <div class=move || if validity.with(|v| v.active) { "onchain-build-result is-ready" } else { "onchain-build-result is-ready is-expired" }
                    role="status" aria-label="已构建交易计划">
                    <div class="onchain-build-summary">
                        <div class="onchain-build-state">
                            <small>"交易计划"</small>
                            <strong>{move || if used.get() { "计划已提交" } else if !validity.with(|v| v.active) { "计划已过期" } else if submit_ready { "可立即执行" } else { "待补执行接入" }}</strong>
                        </div>
                        <dl class="onchain-build-metrics">
                            <div><dt>"预计净收益"</dt><dd class="num">{format!("${:.4}", build.estimated_net_profit_usd)}</dd></div>
                            <div><dt>"费后净差"</dt><dd class="num">{format!("{:+.4}%", build.estimated_net_spread_bps / 100.0)}</dd></div>
                            <div class=move || validity.with(|v| v.tone)>
                                <dt>"计划时效"</dt><dd class="num">{move || validity.with(|v| v.label.clone())}</dd>
                            </div>
                            <div class="onchain-build-costs"><dt>"补库费用归属"</dt><dd>{replenishment_cost_selection::scope(&build.replenishment_costs)}</dd></div>
                            <div class="onchain-build-costs"><dt>"授权费用归属"</dt><dd>{approval_cost_selection::scope(&build.approval_costs)}</dd></div>
                        </dl>
                        <button type="button" class="workbench-primary onchain-submit-action"
                            disabled=move || used.get() || !submit_ready || !validity.with(|v| v.active) || data.saving.get()
                                || data.execution.submitting_execution.get() || data.execution.recovery_problem.get().is_some()
                            title=move || if used.get() { "此计划已提交，请查看原执行结果".to_owned() }
                                else if !validity.with(|v| v.active) { "计划已过期，请重新构建".to_owned() }
                                else if submit_ready { format!("按已核对计划执行 {leg_count} 条腿；不再二次确认") }
                                else { submit_blocker.clone() }
                            on:click=move |_| {
                                if build.valid_until_ms > crate::state::polling::now_ms() as i64 && data.saving.try_get_untracked() == Some(false) {
                                    data.execution.submit_execution.run(build_id.clone());
                                }
                            }
                        >{move || if data.execution.submitting_execution.get() { "执行中…" }
                            else if used.get() { "已提交 · 查看处理结果" }
                            else if !validity.with(|v| v.active) { "计划已过期" }
                            else if leg_count == 3 { "立即执行三腿" } else { "立即执行双腿" }}
                        </button>
                    </div>
                    <ol class="onchain-build-legs" aria-label="交易计划执行顺序">
                        {steps.into_iter().map(execution_plan_step).collect_view()}
                    </ol>
                    <div class="onchain-build-boundary">
                        <span>{move || if used.get() { "此计划已提交；执行与结算结果以原处理结果为准" }
                        else if validity.with(|v| v.active) {
                            "构建阶段未下单；点击执行后按上方顺序直接提交，不再二次确认"
                        } else { "计划已过期，不再接受提交；已发出的订单仍以执行结果为准" }}</span>
                        <small title=blocker_title>{blocker}</small>
                    </div>
                </div>
            }.into_any()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutionBuildValidity {
    active: bool,
    label: String,
    tone: &'static str,
}

fn execution_build_validity(valid_until_ms: i64, now_ms: i64) -> ExecutionBuildValidity {
    let remaining_ms = valid_until_ms.saturating_sub(now_ms);
    if remaining_ms <= 0 {
        return ExecutionBuildValidity {
            active: false,
            label: "已过期".to_owned(),
            tone: "is-danger",
        };
    }
    ExecutionBuildValidity {
        active: true,
        label: format!("{:.1}s", remaining_ms as f64 / 1_000.0),
        tone: if remaining_ms <= 1_500 {
            "is-warning"
        } else {
            "is-positive"
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ExecutionPlanStep {
    position: usize,
    kind: &'static str,
    target: String,
    detail: String,
}

fn execution_plan_steps(build: &OnchainExecutionBuildResponse) -> Vec<ExecutionPlanStep> {
    let side = match build.cex_order.side {
        OrderSide::Buy => "买入",
        OrderSide::Sell => "卖出",
    };
    let cex = ExecutionPlanStep {
        position: 0,
        kind: "交易所 主单",
        target: format!(
            "{} · {}",
            build.cex_order.venue.to_uppercase(),
            build.cex_order.native_symbol,
        ),
        detail: format!("{side} {:.8}", build.cex_order.base_quantity),
    };
    let chain = ExecutionPlanStep {
        position: 0,
        kind: "链上交易",
        target: format!("{} → {}", build.input_token, build.output_token),
        detail: match &build.chain_transaction {
            OnchainUnsignedTransaction::SolanaVersioned { router, .. } => {
                format!("Solana v0 · {router}")
            }
            OnchainUnsignedTransaction::EvmCall {
                chain_id,
                allowance_spender,
                ..
            } => allowance_spender.as_deref().map_or_else(
                || format!("EVM {chain_id} · calldata"),
                |spender| format!("EVM {chain_id} · spender {}", compact_address(spender)),
            ),
        },
    };
    let conversion = build.quote_conversion_order.as_ref().map(|plan| {
        let side = match plan.order.side {
            OrderSide::Buy => "买入",
            OrderSide::Sell => "卖出",
        };
        ExecutionPlanStep {
            position: 0,
            kind: "Quote 换汇",
            target: format!(
                "{} · {}",
                plan.order.venue.to_uppercase(),
                plan.order.native_symbol,
            ),
            detail: format!(
                "{side} {:.8} · {:.4} {} → {:.4} {}",
                plan.order.base_quantity,
                plan.planned_from_amount,
                plan.from_asset,
                plan.planned_to_amount,
                plan.to_asset,
            ),
        }
    });
    let mut steps = Vec::with_capacity(if conversion.is_some() { 3 } else { 2 });
    if build
        .quote_conversion_order
        .as_ref()
        .is_some_and(|plan| plan.sequence == OnchainQuoteConversionSequence::BeforePrimaryCex)
    {
        steps.extend(conversion.clone());
    }
    steps.push(cex);
    steps.push(chain);
    if build
        .quote_conversion_order
        .as_ref()
        .is_some_and(|plan| plan.sequence == OnchainQuoteConversionSequence::AfterPrimaryCex)
    {
        steps.extend(conversion);
    }
    for (index, step) in steps.iter_mut().enumerate() {
        step.position = index + 1;
    }
    steps
}

fn execution_plan_step(step: ExecutionPlanStep) -> impl IntoView {
    let ExecutionPlanStep {
        position,
        kind,
        target,
        detail,
    } = step;
    let target_title = target.clone();
    let detail_title = detail.clone();
    view! {
        <li>
            <span class="num">{format!("{position:02}")}</span>
            <div>
                <strong>{kind}</strong>
                <small title=target_title>{target}</small>
            </div>
            <span title=detail_title>{detail}</span>
            <em>"待执行"</em>
        </li>
    }
}

fn build_failure_guidance(code: &str) -> &'static str {
    match code {
        "ONCHAIN_TOKEN_APPROVAL_REQUIRED" => "本次没有下单；先完成下方独立授权，再重新构建交易计划",
        "ONCHAIN_CEX_DEPTH_MISSING" | "ONCHAIN_CEX_DEPTH_STALE" => {
            "本次没有下单；等待 交易所 100 档 WS 深度恢复后重新构建"
        }
        "ONCHAIN_NET_PROFIT_RECHECK_FAILED" => {
            "firm quote 与完整深度复核后利润已消失；等待下一次费后机会"
        }
        "ONCHAIN_FIRM_BUILD_EXPIRED" => "远程交易检查完成前报价已过期；本次没有下单，请重新构建",
        "ONCHAIN_CEX_INSTRUMENT_STALE" | "ONCHAIN_QUOTE_CONVERSION_INSTRUMENT_STALE" => {
            "本次没有下单；等待官方交易规格刷新后重新构建"
        }
        _ => "构建阶段不会提交订单；按错误原因处理后重新构建",
    }
}

fn token_approval_panel(data: OnchainData, execution_clock_ms: RwSignal<i64>) -> AnyView {
    let build = data.execution.approval_build.get();
    let run = data.execution.approval_submit.get();
    let building = data.execution.building_approval.get();
    if build.is_none() && !building && run.is_none() {
        return ().into_any();
    }
    let recheck = run.as_ref().and_then(|r| r.as_ref().ok()).filter(|r| r.fee_checks_exhausted).map(|r| r.approval_id.clone());
    view! {
        <div class="onchain-approval-stack" aria-label="ERC-20 代币授权">
            {approval_build_panel(build, building, data, execution_clock_ms)}
            {approval_submit_panel(run)}
            {recheck.map(|id| view! { <button type="button" class="row-action" disabled=move || data.execution.submitting_approval.get()
                on:click=move |_| data.execution.submit_approval.run(id.clone())>"重新核对原授权交易"</button> })}
        </div>
    }
    .into_any()
}

fn approval_build_panel(
    result: Option<Result<OnchainTokenApprovalBuildResponse, String>>,
    building: bool,
    data: OnchainData,
    execution_clock_ms: RwSignal<i64>,
) -> AnyView {
    let Some(result) = result else {
        return building
            .then(|| {
                view! {
                    <div class="onchain-approval-result is-warning" role="status">
                        <strong>"正在读取 allowance"</strong>
                        <span>"核对代币、spender 与本次精确授权数量…"</span>
                    </div>
                }
            })
            .into_any();
    };
    match result {
        Err(problem) => {
            let title = problem.clone();
            view! {
                <div class="onchain-approval-result is-danger" role="alert">
                    <strong>"授权计划未生成"</strong>
                    <span title=title>{problem}</span>
                </div>
            }
            .into_any()
        }
        Ok(plan) => approval_plan(plan, data, execution_clock_ms),
    }
}

fn approval_plan(
    plan: OnchainTokenApprovalBuildResponse,
    data: OnchainData,
    execution_clock_ms: RwSignal<i64>,
) -> AnyView {
    let required = raw_amount_label(
        &plan.required_amount_raw,
        plan.token_decimals,
        &plan.token_symbol,
    );
    let current = raw_amount_label(
        &plan.current_allowance_raw,
        plan.token_decimals,
        &plan.token_symbol,
    );
    let spender = compact_address(&plan.spender);
    let steps = match plan.transactions.len() {
        0 => "无需新增授权".to_owned(),
        1 => "1 笔：授权本次精确数量".to_owned(),
        count => format!("{count} 笔：先清零，再授权本次精确数量"),
    };
    let approval_id = plan.approval_id.clone();
    let submit_ready = plan.submit_ready;
    let validity = execution_build_validity(plan.valid_until_ms, execution_clock_ms.get());
    let can_submit = submit_ready && validity.active;
    let blocker = plan
        .blockers
        .first()
        .cloned()
        .unwrap_or_else(|| "授权签名与自定义 RPC 已就绪".to_owned());
    let button_title = if !validity.active {
        "授权计划已过期；没有广播交易，请重新构建授权计划".to_owned()
    } else if submit_ready {
        "只签名并广播 ERC-20 approve；不会提交 交易所 订单".to_owned()
    } else {
        blocker.clone()
    };
    let blocker_title = blocker.clone();
    view! {
        <div class=if plan.approval_required { "onchain-approval-result is-warning" } else { "onchain-approval-result is-positive" } role="status">
            <strong>{if plan.approval_required { "先完成代币授权" } else { "授权额度已满足" }}</strong>
            <span class="num onchain-approval-amount">{format!("当前 {current} · 需要 {required}")}</span>
            <span class="onchain-approval-spender" title=plan.spender.clone()>{format!("spender {spender}")}</span>
            <small class="onchain-approval-steps">{format!("{steps} · {}", validity.label)}</small>
            <a class="onchain-approval-docs" href=plan.official_docs_url target="_blank" rel="noreferrer">"官方授权契约"</a>
            {plan.approval_required.then(|| view! {
                <button
                    type="button"
                    class="workbench-primary onchain-approval-action"
                    disabled=move || !can_submit || data.execution.submitting_approval.get()
                    title=button_title
                    on:click=move |_| data.execution.submit_approval.run(approval_id.clone())
                >
                    {move || if data.execution.submitting_approval.get() {
                        "授权中…"
                    } else if !validity.active {
                        "授权已过期"
                    } else {
                        "独立授权"
                    }}
                </button>
            })}
            <small class="onchain-approval-blocker" title=blocker_title>{blocker}</small>
        </div>
    }
    .into_any()
}

fn approval_submit_panel(
    result: Option<Result<OnchainTokenApprovalSubmitResponse, String>>,
) -> AnyView {
    let Some(result) = result else {
        return ().into_any();
    };
    match result {
        Err(problem) => {
            let title = problem.clone();
            view! {
                <div class="onchain-approval-result is-danger" role="alert">
                    <strong>"授权未启动"</strong>
                    <span title=title>{problem}</span>
                </div>
            }
            .into_any()
        }
        Ok(run) => approval_run_receipt(run).into_any(),
    }
}

fn approval_run_receipt(run: OnchainTokenApprovalSubmitResponse) -> impl IntoView {
            let (label, tone) = match run.status {
                OnchainTokenApprovalRunStatus::Completed => ("授权已确认", "is-positive"),
                OnchainTokenApprovalRunStatus::AwaitingFinality => ("等待授权最终结果", "is-warning"),
                OnchainTokenApprovalRunStatus::FinalityUnresolved => {
                    ("授权最终结果待核对", "is-danger")
                }
                OnchainTokenApprovalRunStatus::Failed => ("授权失败", "is-danger"),
            };
            let transactions = run
                .transaction_ids
                .iter()
                .map(|id| compact_address(id))
                .collect::<Vec<_>>()
                .join(" · ");
            view! {
                <div class=format!("onchain-approval-result onchain-approval-receipt-result {tone}") role="status">
                    <strong>{label}</strong>
                    <span>{run.message}</span>
                    {(!transactions.is_empty()).then(|| view! { <small title=transactions.clone()>{transactions.clone()}</small> })}
                    {run.problem.map(|problem| {
                        let title = problem.clone();
                        view! { <small class="is-danger" title=title>{problem}</small> }
                    })}
                    <div class="onchain-approval-fees">
                        {run.transaction_ids.iter().map(|hash| {
                            let receipt = run.fee_receipts.iter().find(|r| r.basis.transaction_id == *hash);
                            let cost = receipt.and_then(|r| r.network_cost.as_ref());
                            let amount = cost.and_then(|c| c.total_fee_exact.as_ref()).map_or_else(|| "链费待核对".into(), |v|
                                format!("已取得链费 {v} {}", cost.map_or("原生币",|c| c.asset.as_str())));
                            let location = receipt.map(|r| format!("{} · {}", chain_label(&r.basis.chain),r.basis.wallet));
                            view! { <details><summary>{amount}</summary>
                                <span>{location}</span><code>{hash.clone()}</code>
                                {approval_asset_changes(receipt).into_iter().map(|change| view! { <span>{change}</span> }).collect_view()}
                                {receipt.and_then(|r| r.problem.clone()).map(|p| view! { <span class="is-warning">{p}</span> })}
                            </details> }
                        }).collect_view()}
                        <small>"授权链费独立记录；归属以执行收支为准。"</small>
                        {run.fee_checks_exhausted.then(|| view! { <small class="is-warning">"本轮处理结果核对已暂停，交易编号仍保留"</small> })}
                    </div>
                </div>
            }
}

fn approval_asset_changes(receipt: Option<&shared_types::OnchainWalletReceipt>) -> Vec<String> {
    let Some(receipt) = receipt else { return Vec::new(); };
    let mut changes = Vec::new();
    let label = |raw: &str, decimals: u8, symbol: &str| format!("{}{}",
        if raw.starts_with('-') { "-" } else { "+" }, raw_amount_label(raw.trim_start_matches('-'), decimals, symbol));
    for (asset, amount) in receipt.basis.assets.iter().zip(&receipt.asset_changes_raw) {
        if let Some(raw) = amount.as_deref().filter(|v| *v != "0") {
            changes.push(format!("代币变动：{}",label(raw,asset.decimals,&asset.symbol)));
        }
    }
    if let Some(raw) = receipt.additional_native_change_raw.as_deref().filter(|v| *v != "0") {
        if let Some(native) = shared_types::onchain_chain_preset(&receipt.basis.chain) {
            changes.push(format!("额外原生币变动：{}（不含链费）",label(raw,native.base_decimals,native.base_token)));
        }
    }
    changes
}

fn cex_instrument_label(evidence: &OnchainCexInstrumentEvidence) -> (String, Option<String>) {
    let symbol = evidence.native_symbol.as_deref().unwrap_or("精确交易对");
    let label = match evidence.status {
        OnchainCexInstrumentStatus::Ready if evidence.problem.is_some() => {
            format!("交易所 {symbol} 已核对（执行受限）")
        }
        OnchainCexInstrumentStatus::Ready => format!("交易所 {symbol} 已核对"),
        OnchainCexInstrumentStatus::Syncing => "交易所 规格同步中".to_owned(),
        OnchainCexInstrumentStatus::Unlisted => "交易所 官方未挂牌".to_owned(),
        OnchainCexInstrumentStatus::Incomplete => format!("交易所 {symbol} 规格不完整"),
        OnchainCexInstrumentStatus::Stale => format!("交易所 {symbol} 规格已过期"),
        OnchainCexInstrumentStatus::Unavailable => "交易所 规格刷新失败".to_owned(),
        OnchainCexInstrumentStatus::Unsupported => "交易所 规格未接入".to_owned(),
    };
    (label, evidence.problem.clone())
}

fn cex_instrument_tone(evidence: &OnchainCexInstrumentEvidence) -> &'static str {
    match evidence.status {
        OnchainCexInstrumentStatus::Ready if evidence.problem.is_none() => "is-positive",
        OnchainCexInstrumentStatus::Ready | OnchainCexInstrumentStatus::Stale => "is-warning",
        OnchainCexInstrumentStatus::Syncing => "is-neutral",
        OnchainCexInstrumentStatus::Unlisted
        | OnchainCexInstrumentStatus::Incomplete
        | OnchainCexInstrumentStatus::Unavailable
        | OnchainCexInstrumentStatus::Unsupported => "is-danger",
    }
}

fn compact_address(value: &str) -> String {
    if value.len() <= 16 {
        return value.to_owned();
    }
    value
        .get(..8)
        .zip(value.get(value.len() - 6..))
        .map_or_else(
            || value.to_owned(),
            |(head, tail)| format!("{head}..{tail}"),
        )
}

fn execution_recovery_notice(problem: Option<String>) -> impl IntoView {
    problem.map(|problem| {
        let title = problem.clone();
        view! {
        <div class="onchain-execution-recovery" role="alert">
            <strong>"执行记录需要核对"</strong>
            <span title=title>{problem}</span>
            <small>"当前双腿/三腿执行暂停新增提交。保留原记录，按订单号和交易哈希核对；不要重复提交。"</small>
        </div>
        }
    })
}

fn execution_history_panel(data: OnchainData) -> impl IntoView {
    let history = data.execution.history;
    let state = history.state;
    let choices = Memo::new(move |_| state.rows.with(|rows| rows.iter().map(|run| {
        let (label, _) = execution_run_state(run.status);
        (run.run_id.clone(), format!("{} · {label}", compact_address(&run.run_id)))
    }).collect::<Vec<_>>()));
    let selected = Memo::new(move |_| state.selected.with(|result| result.as_ref()
        .and_then(|result| result.as_ref().ok()).map(|run| run.run_id.clone()).unwrap_or_default()));
    view! {
        <section class="onchain-execution-history" aria-label="执行结果">
            <header class="onchain-execution-history-toolbar">
                <strong>"执行结果"</strong>
                <select aria-label="选择执行记录" disabled=move || choices.with(Vec::is_empty)
                    on:change=move |event| state.select(&event_target_value(&event))>
                    <option value="" prop:selected=move || selected.get().is_empty() disabled=true>
                        {move || if !state.loaded.get() { "正在读取" } else if choices.with(Vec::is_empty) { "暂无记录" } else { "选择记录" }}
                    </option>
                    {move || choices.get().into_iter().map(|(id, label)| {
                        let value = id.clone();
                        view! { <option value=value prop:selected=move || selected.get() == id>{label}</option> }
                    }).collect_view()}
                </select>
                <button type="button" class="btn-icon" title="刷新执行结果" aria-label="刷新执行结果"
                    disabled=move || state.reading.get() || state.submitting.get()
                    on:click=move |_| history.refresh.run(())><span aria-hidden="true">"↻"</span></button>
            </header>
            {move || execution_recovery_notice(state.problem.get())}
            {move || execution_submit_panel(state.selected.get())}
        </section>
    }
}

fn execution_submit_panel(
    result: Option<Result<OnchainExecutionSubmitResponse, String>>,
) -> AnyView {
    let Some(result) = result else {
        return ().into_any();
    };
    match result {
        Err(problem) => {
            let problem_title = problem.clone();
            view! {
                <div class="onchain-submit-result is-danger" role="alert">
                    <strong>"执行请求未完成"</strong>
                    <span title=problem_title>{problem}</span>
                </div>
            }
            .into_any()
        }
        Ok(run) => execution_run_panel(run).into_any(),
    }
}

fn execution_run_panel(run: OnchainExecutionSubmitResponse) -> impl IntoView {
    let review_href = crate::panels::routing::settlement_review_href(shared_types::review::settlements::SettlementSource::Onchain, &run.run_id);
    let (label, tone) = execution_run_state(run.status);
    let identifiers = [
        run.cex_order_id.as_deref().map(|id| format!("CEX {id}")),
        run.chain_transaction_id
            .as_deref()
            .map(|id| format!("链上 {id}")),
        run.compensation_order_id
            .as_deref()
            .map(|id| format!("补偿 {id}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let legs = run.legs.clone();
    let recovery_actions = run.recovery_actions.clone();
    let run_id = compact_address(&run.run_id);
    let accounting_pending = (!run.quantity_reconciled
        && (run.status == OnchainExecutionRunStatus::Completed
            || run.legs.iter().any(|leg| {
                leg.kind == OnchainExecutionLegKind::Chain
                    && leg.status == OnchainExecutionLegStatus::Confirmed
            })))
        || run.legs.iter().any(|leg| {
            if leg.kind == OnchainExecutionLegKind::Chain
                && leg.status == OnchainExecutionLegStatus::Confirmed
            {
                leg.chain_settlement.as_ref().is_none_or(|row| {
                    row.status != shared_types::OnchainChainSettlementStatus::Complete
                })
            } else if leg.filled_quantity.is_some_and(|quantity| quantity > 0.0) {
                leg.settlement.as_ref().is_none_or(|row| {
                    row.status != shared_types::OnchainCexSettlementStatus::Complete
                })
            } else {
                false
            }
        });
    let is_flat = run.remaining_exposure_usd <= 0.005
        && !accounting_pending
        && matches!(
            run.status,
            OnchainExecutionRunStatus::Completed | OnchainExecutionRunStatus::Compensated
        );
    let exposure_label = if run.status == OnchainExecutionRunStatus::FinalityUnresolved {
        "成交与暴露待核对".to_owned()
    } else if run.status == OnchainExecutionRunStatus::Exposed
        && run.remaining_exposure_usd <= 0.005
    {
        "剩余币量待处理".to_owned()
    } else if accounting_pending {
        "数量与费用待核对".to_owned()
    } else if is_flat {
        "对冲数量已对齐".to_owned()
    } else if run.remaining_exposure_usd <= 0.005 {
        "执行未完成，数量待核对".to_owned()
    } else {
        format!("未对冲暴露 ${:.2}", run.remaining_exposure_usd)
    };
    view! {
        <div class=format!("onchain-submit-result {tone}") role="status">
            <div class="onchain-submit-heading">
                <small>"执行运行状态"</small>
                <strong>{label}</strong>
                <span class="num" title=run.run_id>{run_id}</span>
                <a class="row-action" href=review_href>"查看收支复盘"</a>
            </div>
            <span class="onchain-submit-message">{run.message}</span>
            <strong class=if is_flat { "onchain-submit-exposure is-flat" }
                else if accounting_pending && run.remaining_exposure_usd <= 0.005
                    && matches!(run.status, OnchainExecutionRunStatus::Completed | OnchainExecutionRunStatus::Compensated) { "onchain-submit-exposure is-pending" }
                else { "onchain-submit-exposure is-exposed" }>{exposure_label}</strong>
            {accounting_receipt::receipt(run.accounting, run.estimated_net_profit_usd, run.replenishment_costs.len(), run.approval_costs.len())}
            {(!identifiers.is_empty()).then(|| view! { <small title=identifiers.clone()>{identifiers.clone()}</small> })}
            {run.problem.map(|problem| {
                let problem_title = problem.clone();
                view! { <small class="is-danger" title=problem_title>{problem}</small> }
            })}
            {(!legs.is_empty()).then(|| view! {
                <div class="onchain-execution-legs" aria-label="执行腿状态">
                    {legs.into_iter().map(execution_leg_row).collect_view()}
                </div>
            })}
            {(!recovery_actions.is_empty()).then(|| view! {
                <div class="onchain-recovery-actions" aria-label="恢复动作">
                    <strong>"下一步"</strong>
                    {recovery_actions.into_iter().map(recovery_action).collect_view()}
                </div>
            })}
        </div>
    }
}

fn execution_run_state(status: OnchainExecutionRunStatus) -> (&'static str, &'static str) {
    match status {
        OnchainExecutionRunStatus::Executing => ("按顺序执行中", "is-warning"),
        OnchainExecutionRunStatus::Completed => ("全部腿已完成", "is-positive"),
        OnchainExecutionRunStatus::Compensated => ("已自动回滚", "is-warning"),
        OnchainExecutionRunStatus::AwaitingChainFinality => ("等待链上最终结果", "is-danger"),
        OnchainExecutionRunStatus::FinalityUnresolved => ("执行最终结果待核对", "is-danger"),
        OnchainExecutionRunStatus::Exposed => ("存在未对冲暴露", "is-danger"),
        OnchainExecutionRunStatus::Failed => ("执行失败", "is-danger"),
    }
}

fn recovery_action(action: OnchainExecutionRecoveryAction) -> impl IntoView {
    view! {
        <span>
            <b>{if action.automated { "系统" } else { "人工" }}</b>
            {action.message}
        </span>
    }
}

fn execution_leg_row(leg: OnchainExecutionLegResult) -> impl IntoView {
    let kind = match leg.kind {
        OnchainExecutionLegKind::QuoteConversion => "Quote 换汇",
        OnchainExecutionLegKind::PrimaryCex => "交易所 主单",
        OnchainExecutionLegKind::Chain => "链上交易",
        OnchainExecutionLegKind::Compensation => "补偿单",
    };
    let (status, tone) = match leg.status {
        OnchainExecutionLegStatus::Filled | OnchainExecutionLegStatus::Confirmed => {
            ("已完成", "is-positive")
        }
        OnchainExecutionLegStatus::Submitted | OnchainExecutionLegStatus::Pending => {
            ("处理中", "is-warning")
        }
        OnchainExecutionLegStatus::PartiallyFilled => ("部分成交", "is-warning"),
        OnchainExecutionLegStatus::Cancelled => ("已撤销", "is-neutral"),
        OnchainExecutionLegStatus::Compensated => ("已回滚", "is-warning"),
        OnchainExecutionLegStatus::Rejected | OnchainExecutionLegStatus::Failed => {
            ("失败", "is-danger")
        }
        OnchainExecutionLegStatus::Exposed => ("存在暴露", "is-danger"),
    };
    let target = leg.symbol.as_deref().map_or_else(
        || leg.venue.to_uppercase(),
        |symbol| format!("{} · {symbol}", leg.venue.to_uppercase()),
    );
    let identifier = leg
        .order_id
        .as_deref()
        .or(leg.transaction_id.as_deref())
        .map(compact_address)
        .unwrap_or_else(|| "-".to_owned());
    let target_title = target.clone();
    let message_title = leg.message.clone();
    view! {
        <div class=format!("onchain-execution-leg {tone}")>
            <span class="num">{format!("{:02}", leg.position)}</span>
            <strong>{kind}</strong>
            <span title=target_title>{target}</span>
            <span>{status}</span>
            <small title=message_title>{leg.message}</small>
            <small class="num">{identifier}</small>
            {settlement_receipt::receipt(leg.settlement, leg.order_id.is_some() && leg.filled_quantity.is_some_and(|quantity| quantity > 0.0))}
            {leg.recovery_residual.map(settlement_receipt::recovery_residual)}
            {leg.chain_input_adjustment.map(settlement_receipt::chain_input_adjustment)}
            {leg.chain_settlement.map(settlement_receipt::chain_receipt)}
        </div>
    }
}

fn readiness_for(
    snapshot: &OnchainComparisonSnapshot,
    direction: OnchainComparisonDirection,
) -> Option<&OnchainDirectionReadiness> {
    snapshot
        .execution_readiness
        .directions
        .iter()
        .find(|row| row.direction == direction)
}

fn inventory_label(row: &shared_types::OnchainInventoryEvidence) -> String {
    let location = match row.location {
        OnchainInventoryLocation::Onchain => "链上",
        OnchainInventoryLocation::Cex => "交易所",
    };
    let available = row
        .available
        .map_or_else(|| "待核对".to_owned(), |value| format!("{value:.6}"));
    let marker = match row.status {
        OnchainInventoryStatus::Ready => "可用",
        OnchainInventoryStatus::Insufficient => "不足",
        OnchainInventoryStatus::Unknown => "未知",
    };
    format!(
        "{location} {} {available}/{:.6} {marker}",
        row.asset, row.required
    )
}

fn loading_state(message: &'static str) -> AnyView {
    view! { <div class="workbench-empty-state"><strong>"加载中"</strong><span>{message}</span></div> }
        .into_any()
}

fn error_state(message: String) -> AnyView {
    view! { <div class="workbench-empty-state is-error"><strong>"读取失败"</strong><span>{message}</span></div> }
        .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_chain_preview_uses_its_own_freshness_and_never_masquerades_as_submission() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.enabled = true;
        snapshot.config.cross_chain.enabled = true;
        snapshot.cross_chain.preview_ready = true;
        snapshot.cross_chain.quote_observed_at_ms = Some(123);
        snapshot.cross_chain.quality = OnchainCrossChainQuality::NoNetProfit;
        snapshot.quality = OnchainComparisonQuality::UpstreamUnavailable;
        assert_eq!(cross_chain_preview_request(&snapshot).unwrap().expected_quote_observed_at_ms, 123);
        snapshot.cross_chain.quality = OnchainCrossChainQuality::Stale;
        assert!(cross_chain_preview_request(&snapshot).is_none());
        snapshot.cross_chain.quality = OnchainCrossChainQuality::Fresh;
        snapshot.config.enabled = false;
        assert!(cross_chain_preview_request(&snapshot).is_none());
    }

    #[test]
    fn approval_receipt_render_keeps_actual_fees_unknown_costs_and_recheck_distinct() {
        Owner::new().with(|| {
            let mut run: OnchainTokenApprovalSubmitResponse = serde_json::from_value(serde_json::json!({
                "runId":"approval-test","approvalId":"approval","status":"completed","transactionIds":["0xhash"],
                "message":"授权已核对","startedAtMs":1000,"updatedAtMs":2000
            })).unwrap();
            let unknown = approval_run_receipt(run.clone()).to_html();
            assert!(unknown.contains("链费待核对"));
            assert!(run.receipt_check_pending());
            run.fee_checks_exhausted = true;
            assert!(!run.receipt_check_pending());
            assert!(approval_run_receipt(run.clone()).to_html().contains("本轮处理结果核对已暂停"));
            if let Ok(path) = std::env::var("CROSSLINE_APPROVAL_RECEIPT_FIXTURE") {
                run = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                let html = approval_run_receipt(run.clone()).to_html();
                assert!(!run.receipt_check_pending());
                assert!(html.contains("已取得链费 0.000021 ETH"));
                assert!(html.contains("尚未归入某笔套利利润"));
                if let Ok(path) = std::env::var("CROSSLINE_APPROVAL_RECEIPT_HTML") { std::fs::write(path,html).unwrap(); }
            }
        });
    }

    #[test]
    fn comparison_direction_order_is_stable() {
        assert!(
            direction_rank(OnchainComparisonDirection::BuyOnchainSellCex)
                < direction_rank(OnchainComparisonDirection::BuyCexSellOnchain)
        );
    }

    #[test]
    fn expected_profit_uses_the_current_notional_and_net_edge() {
        assert!((expected_profit_usd(1_000.0, 25.0) - 2.5).abs() < f64::EPSILON);
        assert!((expected_profit_usd(750.0, -10.0) + 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn cost_breakdown_renders_computed_costs_instead_of_adding_configured_rates() {
        Owner::new().with(|| {
            let comparison = OnchainCexComparison {
                direction: OnchainComparisonDirection::BuyOnchainSellCex,
                onchain_price: 100.0,
                cex_price: 120.0,
                gross_spread_bps: 2_000.0,
                cex_fee_bps: 100.0,
                quote_conversion_fee_bps: 0.0,
                slippage_bps: 50.0,
                gas_usd: 0.2,
                gas_bps: 20.0,
                total_cost_bps: 200.0,
                net_spread_bps: 1_800.0,
                observable_notional_usd: 100.0,
                executable: false,
            };
            let html = execution_cost_breakdown(&comparison).to_html();
            assert!(html.contains("交易与滑点"));
            assert!(html.contains("1.800%"));
            assert!(html.contains("总成本"));
            assert!(html.contains("2.000%"));
            assert!(html.contains("交易所 费率 1.000%"));
            assert!(html.contains("滑点预留 0.500%"));
            assert_eq!(html.matches("<dt>").count(), 3);
        });
    }

    #[test]
    fn route_book_keeps_each_direction_leg_side_explicit() {
        assert_eq!(
            route_leg_sides(OnchainComparisonDirection::BuyOnchainSellCex),
            (("买入", "is-buy"), ("卖出", "is-sell"))
        );
        assert_eq!(
            route_leg_sides(OnchainComparisonDirection::BuyCexSellOnchain),
            (("卖出", "is-sell"), ("买入", "is-buy"))
        );
    }

    #[test]
    fn explicit_raw_mode_keeps_an_exact_pair_observation_only() {
        let mut snapshot = OnchainComparisonSnapshot {
            quality: OnchainComparisonQuality::Fresh,
            ..Default::default()
        };
        assert!(onchain_cex_pair_matches(&snapshot.config));
        assert!(!raw_observation_mode(&snapshot));

        snapshot.config.spread_alert.mode = OnchainSpreadAlertMode::RawObservation;

        assert!(raw_observation_mode(&snapshot));
        assert!(raw_observation_detail(&snapshot.config).contains("明确选择原始观察模式"));
        assert!(raw_observation_next_step(&snapshot.config).contains("切换为“费后机会”模式"));
    }

    #[test]
    fn fresh_cross_quote_with_ws_conversion_is_not_downgraded_to_raw_observation() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.cex_symbol = "SOL/USD".to_owned();
        snapshot.quality = OnchainComparisonQuality::Fresh;

        assert!(!onchain_quotes_match(&snapshot.config));
        assert!(!raw_observation_mode(&snapshot));
    }

    #[test]
    fn compact_address_preserves_short_values_and_abbreviates_evm_addresses() {
        assert_eq!(compact_address("short"), "short");
        assert_eq!(
            compact_address("0x0000000000001fF3684f28c67538d4D072C22734"),
            "0x000000..C22734"
        );
    }

    #[test]
    fn build_action_never_suggests_depth_can_rescue_an_unprofitable_direction() {
        assert_eq!(
            build_action_label(
                OnchainComparisonQuality::NoNetProfit,
                -12.0,
                10.0,
                BuildActionState::Blocked,
                true,
                false,
            ),
            "等待费后盈利"
        );
        assert_eq!(
            build_action_label(
                OnchainComparisonQuality::NoNetProfit,
                8.0,
                10.0,
                BuildActionState::Blocked,
                true,
                false,
            ),
            "等待费后盈利"
        );
    }

    #[test]
    fn profitable_direction_names_the_actual_next_gate() {
        assert_eq!(
            build_action_label(
                OnchainComparisonQuality::LowLiquidity,
                30.0,
                10.0,
                BuildActionState::Buildable,
                true,
                false,
            ),
            "核对深度并构建"
        );
        assert_eq!(
            build_action_label(
                OnchainComparisonQuality::Fresh,
                30.0,
                10.0,
                BuildActionState::Blocked,
                false,
                false,
            ),
            "补齐执行数据依据"
        );
        assert_eq!(
            build_action_label(
                OnchainComparisonQuality::Fresh,
                30.0,
                10.0,
                BuildActionState::SetupRequired,
                false,
                false,
            ),
            "打开执行接入"
        );
    }

    #[test]
    fn unavailable_quote_state_precedes_cached_profit_numbers() {
        let snapshot = OnchainComparisonSnapshot {
            quality: OnchainComparisonQuality::UpstreamUnavailable,
            provider_problem: Some("Jupiter 报价暂不可用".to_owned()),
            ..Default::default()
        };
        let comparison = OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            onchain_price: 1.0,
            cex_price: 0.99,
            gross_spread_bps: -100.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 5.0,
            gas_usd: 0.01,
            gas_bps: 1.0,
            total_cost_bps: 16.0,
            net_spread_bps: -116.0,
            observable_notional_usd: 100.0,
            executable: false,
        };

        let gate = direction_market_gate(&snapshot, &comparison).expect("quote must be blocked");
        assert_eq!(gate.0, "行情源不可用");
        assert_eq!(gate.2, "Jupiter 报价暂不可用");
        assert_eq!(profit_readiness(&snapshot, &comparison).0, "待报价");
    }

    #[test]
    fn built_plan_expires_at_the_server_deadline() {
        assert_eq!(
            execution_build_validity(5_000, 1_000),
            ExecutionBuildValidity {
                active: true,
                label: "4.0s".to_owned(),
                tone: "is-positive",
            }
        );
        assert_eq!(execution_build_validity(5_000, 4_200).tone, "is-warning");
        assert_eq!(
            execution_build_validity(5_000, 5_000),
            ExecutionBuildValidity {
                active: false,
                label: "已过期".to_owned(),
                tone: "is-danger",
            }
        );
    }

    #[test]
    fn execution_status_copy_does_not_guess_the_leg_count() {
        assert_eq!(
            execution_run_state(OnchainExecutionRunStatus::Executing).0,
            "按顺序执行中"
        );
        assert_eq!(
            execution_run_state(OnchainExecutionRunStatus::Completed).0,
            "全部腿已完成"
        );
        assert!(build_failure_guidance("ONCHAIN_CEX_DEPTH_MISSING").contains("没有下单"));
    }

    #[test]
    fn execution_recovery_render_keeps_unknown_fills_and_journal_damage_visible() {
        Owner::new().with(|| {
            let problem = "恢复日志第 2 行未完整写入；不能跳过或继续追加".to_owned();
            let notice = execution_recovery_notice(Some(problem.clone())).to_html();
            assert!(notice.contains("role=\"alert\""));
            assert!(notice.contains(&problem));
            assert!(notice.contains("不要重复提交"));
            assert!(!notice.contains("<button"));
            let run = OnchainExecutionSubmitResponse {
                run_id:"onchain-recovery-fixture".into(),build_id:"build".into(),status:OnchainExecutionRunStatus::FinalityUnresolved,
                cex_order_id:Some("existing-order-id".into()),cex_order_state:None,cex_filled_quantity:None,chain_transaction_id:Some("existing-chain-hash".into()),
                compensation_order_id:None,legs:Vec::new(),recovery_actions:Vec::new(),replenishment_costs:Vec::new(),approval_costs:Vec::new(),estimated_net_profit_usd:1.0,remaining_exposure_usd:0.0,quantity_reconciled:false,accounting:None,
                message:"已保留原订单号，等待核对".into(),problem:Some("交易所 提交结果未知".into()),started_at_ms:1,updated_at_ms:2,
            };
            let html = execution_run_panel(run).to_html();
            assert!(html.contains("成交与暴露待核对"));
            assert!(!html.contains("无未对冲暴露"));
            assert!(html.contains("existing-order-id"));
            assert!(html.contains("existing-chain-hash"));
            let legacy: shared_types::OnchainExecutionRunsResponse = serde_json::from_str("{\"rows\":[],\"observedAtMs\":1}").unwrap();
            assert!(legacy.recovery_problem.is_none());
            if let Ok(path) = std::env::var("EXECUTION_RECOVERY_RENDER_PATH") {
                let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/styles/.generated/input.css")).unwrap();
                std::fs::write(path, format!("<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>执行恢复核对</title><style>{css}</style><body>{notice}{html}</body></html>")).unwrap();
            }
        });
    }

    #[test]
    fn chain_settlement_header_never_hides_unreconciled_or_small_inventory() {
        Owner::new().with(|| {
            let mut run: OnchainExecutionSubmitResponse =
                serde_json::from_value(serde_json::json!({
                    "runId":"quantity-check", "buildId":"build", "status":"exposed",
                    "estimatedNetProfitUsd":1, "remainingExposureUsd":0.00001,
                    "message":"confirmed", "startedAtMs":1, "updatedAtMs":2
                }))
                .unwrap();
            let html = execution_run_panel(run.clone()).to_html();
            assert!(html.contains("剩余币量待处理"));
            assert!(!html.contains("对冲数量已对齐"));
            assert!(!html.contains("onchain-submit-exposure is-flat"));
            run.status = OnchainExecutionRunStatus::Completed;
            let html = execution_run_panel(run.clone()).to_html();
            assert!(html.contains("数量与费用待核对"));
            assert!(!html.contains("对冲数量已对齐"));
            run.quantity_reconciled = true;
            assert!(execution_run_panel(run.clone())
                .to_html()
                .contains("对冲数量已对齐"));
            run.legs.push(
                serde_json::from_value(serde_json::json!({
                    "position":2, "kind":"chain", "status":"confirmed", "venue":"solana",
                    "transactionId":"signature", "message":"confirmed"
                }))
                .unwrap(),
            );
            let html = execution_run_panel(run).to_html();
            assert!(
                html.contains("数量与费用待核对"),
                "missing fee evidence still needs review"
            );
            assert!(!html.contains("对冲数量已对齐"));
        });
    }

    #[test]
    fn execution_inventory_summary_distinguishes_waiting_ready_and_blocked() {
        let inventory = |status| shared_types::OnchainInventoryEvidence {
            location: OnchainInventoryLocation::Onchain,
            scope: "test".to_owned(),
            asset: "USDC".to_owned(),
            required: 10.0,
            available: None,
            status,
            source: "test".to_owned(),
            observed_at_ms: None,
            problem: None,
        };
        let mut readiness = OnchainDirectionReadiness {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            path: Default::default(),
            inventory: vec![
                inventory(OnchainInventoryStatus::Unknown),
                inventory(OnchainInventoryStatus::Unknown),
                inventory(OnchainInventoryStatus::Unknown),
            ],
            cex_instrument: OnchainCexInstrumentEvidence {
                status: OnchainCexInstrumentStatus::Ready,
                ready: true,
                ..Default::default()
            },
            build_ready: false,
            submit_ready: false,
            blockers: Vec::new(),
        };

        assert_eq!(
            inventory_readiness(&readiness),
            (
                "0/3".to_owned(),
                "is-warning",
                "链上 USDC 待核对/10.000000 未知；链上 USDC 待核对/10.000000 未知；链上 USDC 待核对/10.000000 未知".to_owned(),
            )
        );

        readiness
            .inventory
            .iter_mut()
            .for_each(|row| row.status = OnchainInventoryStatus::Ready);
        assert_eq!(inventory_readiness(&readiness).0, "3/3");
        assert_eq!(inventory_readiness(&readiness).1, "is-positive");

        readiness.inventory[0].status = OnchainInventoryStatus::Insufficient;
        assert_eq!(inventory_readiness(&readiness).0, "2/3");
        assert_eq!(inventory_readiness(&readiness).1, "is-danger");
    }

    #[test]
    fn path_badge_distinguishes_direct_inventory_from_cross_quote_inventory() {
        let mut readiness = OnchainDirectionReadiness {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            path: shared_types::OnchainPathReadiness {
                kind: OnchainPathKind::DirectTwoLeg,
                availability: OnchainPathAvailability::ReadyToBuild,
                legs: Vec::new(),
                replenishment: Vec::new(),
                transfer_cost_usd: None,
                post_transfer_net_profit_usd: None,
                summary: "USDC→PUPS @ SOLANA ｜ PUPS→USDC @ KRAKEN".to_owned(),
            },
            inventory: Vec::new(),
            cex_instrument: OnchainCexInstrumentEvidence::default(),
            build_ready: true,
            submit_ready: false,
            blockers: Vec::new(),
        };

        assert_eq!(
            path_readiness_label(&readiness),
            ("2腿可构建".to_owned(), "is-positive")
        );

        readiness.path.kind = OnchainPathKind::QuoteConvertedThreeLeg;
        readiness.path.availability = OnchainPathAvailability::InventoryRequired;
        assert_eq!(
            path_readiness_label(&readiness),
            ("3腿缺库存".to_owned(), "is-danger")
        );
    }

    #[test]
    fn readiness_strip_keeps_profit_depth_and_route_gates_explicit() {
        let mut snapshot = OnchainComparisonSnapshot {
            quality: OnchainComparisonQuality::Fresh,
            ..Default::default()
        };
        snapshot.config.spread_alert.min_net_spread_bps = 10.0;
        snapshot.execution_readiness.chain_submission_ready = true;
        snapshot.execution_readiness.cex_live_mode_ready = false;
        snapshot.execution_readiness.global_blockers = vec!["实盘总开关未开启".to_owned()];
        let comparison = OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            onchain_price: 1.0,
            cex_price: 1.01,
            gross_spread_bps: 100.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 5.0,
            gas_usd: 0.02,
            gas_bps: 2.0,
            total_cost_bps: 17.0,
            net_spread_bps: 83.0,
            observable_notional_usd: 100.0,
            executable: false,
        };
        let readiness = OnchainDirectionReadiness {
            direction: comparison.direction,
            path: shared_types::OnchainPathReadiness {
                kind: OnchainPathKind::DirectTwoLeg,
                availability: OnchainPathAvailability::SetupRequired,
                legs: Vec::new(),
                replenishment: Vec::new(),
                transfer_cost_usd: None,
                post_transfer_net_profit_usd: None,
                summary: "USDC→PUPS @ SOLANA ｜ PUPS→USDC @ KRAKEN".to_owned(),
            },
            inventory: Vec::new(),
            cex_instrument: OnchainCexInstrumentEvidence {
                status: OnchainCexInstrumentStatus::Ready,
                ready: true,
                ..Default::default()
            },
            build_ready: false,
            submit_ready: false,
            blockers: Vec::new(),
        };

        let facts = execution_readiness_facts(&snapshot, &comparison, &readiness);
        assert_eq!(
            facts.iter().map(|fact| fact.label).collect::<Vec<_>>(),
            vec!["收益", "余额", "规格", "深度", "路径"]
        );
        assert_eq!(facts[0].value, "已通过");
        assert_eq!(facts[3].value, "构建时");
        assert!(facts[3].title.contains("100 档深度"));
        assert_eq!(facts[4].value, "2腿待接入");
        assert_eq!(facts[4].tone, "is-warning");
        assert!(facts[4].title.contains("USDC→PUPS"));
    }
}
