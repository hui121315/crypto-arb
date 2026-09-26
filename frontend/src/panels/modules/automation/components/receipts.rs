use super::super::{format::date_time_label, receipts::ReceiptData};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    AutomationExecutionReceipt, CloseLegStatus, CloseRun, CloseRunStatus, ExecutionRun,
    ExecutionRunLeg, ExecutionRunState, LiveOrderState, OrderUpdateSource,
};

pub(super) fn receipts_panel(data: ReceiptData) -> impl IntoView {
    let paper = Memo::new(move |_| {
        data.state.with(|state| {
            state
                .value()
                .is_some_and(|receipt| receipt.mode == Some(shared_types::ExecutionMode::DryRun))
        })
    });
    let run = Memo::new(move |_| {
        data.state
            .with(|state| state.value().map(|value| value.run.clone()))
    });
    let closes = Memo::new(move |_| {
        data.state.with(|state| {
            state
                .value()
                .map_or_else(Vec::new, |value| value.close_runs.clone())
        })
    });
    view! {
        <section class="automation-receipts" aria-label="自动化交易记录">
            <header>
                <label class="workbench-field"><span>"运行记录"</span>
                    <select aria-label="选择自动化运行记录" bind:value=data.choice>
                        <option value="">"跟随最近运行"</option>
                        <For each=move || data.options.get() key=|(id, _)| id.clone() children=|(id, label)| view! { <option value=id>{label}</option> } />
                    </select>
                </label>
                <button type="button" class="row-action" title="刷新处理结果"
                    aria-label=move || if data.reading.get() { "读取中…" } else { "刷新处理结果" }
                    disabled=move || data.reading.get() || data.run_id.get().is_none()
                    on:click=move |_| data.refresh.run(())><span aria-hidden="true">"↻"</span></button>
            </header>
            <p class="automation-receipt-source" role="status">{move || {
                if data.run_id.get().is_none() { return "暂无自动化运行编号；没有把无记录当作成交或平仓".to_owned(); }
                data.state.with(|state| match state {
                    LoadState::Loading => "正在读取对应交易记录".into(),
                    LoadState::Ready(receipt) => format!("{} · 本地执行账本 · WS 更新", match receipt.mode {
                        Some(shared_types::ExecutionMode::DryRun) => "模拟记录，非实盘成交",
                        Some(shared_types::ExecutionMode::Testnet) => "测试网记录，非本地模拟",
                        Some(shared_types::ExecutionMode::Live) => "实盘记录，连接健康需另核对",
                        None => "原始执行环境待确认",
                    }),
                    LoadState::Stale { problem, .. } => format!("处理结果待确认，保留上次记录：{}", problem.message),
                    LoadState::Error(problem) => format!("处理结果读取失败：{}", problem.message),
                })
            }}</p>
            <Show when=move || run.get().is_some()>
                <div class="automation-receipt-summary">
                    <div><span>"运行编号"</span><strong>{move || run.with(|run| run.as_ref().map(|run| run.run_id.clone()))}</strong></div>
                    <div><span>"后台状态"</span><strong>{move || run.with(|run| run.as_ref().map(|run| run_label(run.state)))}</strong></div>
                    <div><span>"未对冲金额 USD"</span><strong>{move || run.with(|run| number(run.as_ref().map(|run| run.net_exposure_usd)))}</strong></div>
                    <div><span>"记录更新"</span><strong>{move || run.with(|run| run.as_ref().map(|run| date_time_label(run.updated_at_ms)))}</strong></div>
                </div>
                <div class="automation-receipt-legs">
                    {leg_row("做多腿", Memo::new(move |_| run.with(|run| run.as_ref().map(|run| run.long_leg.clone()))), paper)}
                    {leg_row("做空腿", Memo::new(move |_| run.with(|run| run.as_ref().map(|run| run.short_leg.clone()))), paper)}
                </div>
                <p class=move || run.with(|run| if run.as_ref().is_some_and(|run| run.finality_problem.is_some() || run.unwind_problem.is_some() || run.valuation_problem.is_some()) {
                    "automation-receipt-problem"
                } else { "automation-receipt-source" })>{move || run.with(|run| run.as_ref().map(|run| {
                    let problems = [run.finality_problem.as_ref(), run.unwind_problem.as_ref(), run.valuation_problem.as_ref()]
                        .into_iter().flatten().map(|problem| format!("{} · {}", problem.code, problem.message)).collect::<Vec<_>>();
                    if problems.is_empty() { run.status_reason.clone() } else { problems.join("；") }
                }))}</p>
                <header><strong>"关联退出记录"</strong><span>{move || data.state.with(|state| state.value().map(|receipt|
                    format!("{} / {} 条", receipt.close_runs.len(), receipt.close_run_total)))}</span></header>
                <Show when=move || !closes.get().is_empty() fallback=|| view! { <p>"尚无匹配的平仓结果；不会据此认定已经退出。"</p> }>
                    <For each=move || closes.get() key=|close| close.id.clone() children=move |initial| {
                        let id = initial.id.clone();
                        let close = Memo::new(move |_| closes.with(|rows| rows.iter().find(|row| row.id == id).cloned()).unwrap_or_else(|| initial.clone()));
                        close_row(close, run, paper)
                    } />
                </Show>
                <nav class="automation-receipt-links">
                    <a href=move || run.get().map(|run| crate::panels::routing::execution_run_href(crate::panels::workstation::ModuleId::Positions, &run))>"关联持仓"</a>
                    <a href=move || run.get().map(|run| crate::panels::routing::execution_run_href(crate::panels::workstation::ModuleId::Execution, &run))>"运行订单"</a>
                    <a href=move || run.get().map(|run| crate::panels::routing::execution_run_href(crate::panels::workstation::ModuleId::Review, &run))>"关联复盘"</a>
                </nav>
            </Show>
        </section>
    }
}

fn leg_row(
    label: &'static str,
    leg: Memo<Option<ExecutionRunLeg>>,
    paper: Memo<bool>,
) -> impl IntoView {
    view! { <section class="automation-receipt-leg">
        <header><strong>{label}</strong><span>{move || leg.with(|leg| leg.as_ref().map(|leg| format!("{} · {}", leg.exchange, leg.symbol)))}</span></header>
        <dl>
            <div><dt>"订单状态"</dt><dd>{move || leg.with(|leg| leg.as_ref().map(|leg| order_label(leg.state)))}</dd></div>
            <div><dt>"已成交 / 目标数量"</dt><dd>{move || leg.with(|leg| leg.as_ref().map(|leg| format!("{} / {}", number(leg.filled_quantity), number(Some(leg.target_quantity)))))}</dd></div>
            <div><dt>"成交金额 USD"</dt><dd>{move || leg.with(|leg| number(leg.as_ref().and_then(|leg| leg.filled_notional_usd)))}</dd></div>
            <div><dt>"最终结果来源"</dt><dd>{move || leg.with(|leg| leg.as_ref().map(|leg| if paper.get()
                && leg.finality_source == Some(OrderUpdateSource::AdapterAck) { "本地模拟处理结果" } else { source_label(leg.finality_source) }))}</dd></div>
        </dl>
    </section> }
}

fn close_row(
    close: Memo<CloseRun>,
    run: Memo<Option<ExecutionRun>>,
    paper: Memo<bool>,
) -> impl IntoView {
    // A queued row render can outlive removal when the followed execution changes.
    view! { <details class="automation-close-receipt" data-close-id=move || close.try_with(|close| close.id.clone())>
        <summary><span>{move || close.try_with(|close| close.id.clone())}</span><strong>{move || close.try_with(|close| close_label(close.status))}</strong></summary>
        <p class="automation-receipt-source" title=move || close.try_with(|close| close.reason.clone()).flatten()>{move || close.try_with(|close| exit_reason_label(close.reason.as_deref()).to_owned())}</p>
        <p>{move || close.try_with(|close| close.message.clone())}</p>
        <p>{move || close.try_with(|close| format!("记录更新 {} · 裸露金额 ${}", date_time_label(close.updated_at_ms), number(Some(close.naked_exposure_usd))))}</p>
        <div class="automation-close-legs">{move || close.try_with(|close| run.try_with(|run| run.as_ref().map(|run|
            close.legs.iter().filter(|leg| leg.pair_evidence.as_ref().is_some_and(|pair| AutomationExecutionReceipt::matches_pair(run, pair)))
                .map(|leg| view! { <div><strong>{format!("{} · {} · {}", leg.venue, leg.symbol, if leg.side == shared_types::PositionSide::Long { "多" } else { "空" })}</strong>
                    <span>{format!("{} · 目标 {} · 来源 {}", close_leg_label(leg.status), number(Some(leg.quantity)),
                        if paper.try_get() == Some(true) && leg.finality_source == Some(OrderUpdateSource::AdapterAck)
                            && leg.order.as_ref().is_some_and(|order| order.intent.mode == shared_types::ExecutionMode::DryRun) {
                            "本地模拟处理结果"
                        } else { source_label(leg.finality_source) })}</span></div> }).collect_view()
        )))}</div>
        <p>{move || close.try_with(|close| close.cost_reconciliation.as_ref().map_or_else(|| "退出费用尚未核清".into(), |cost|
            format!("该平仓结果总费用 ${} · {}", number(cost.total_actual_cost_usd), if cost.missing_fields.is_empty() { "费用字段齐备；不等于策略净利润".into() } else { format!("待核对：{}", cost.missing_fields.join("、")) })))}</p>
        <p>{move || close.try_with(|close| if close.scope == shared_types::CloseRunScope::All { "此处理结果包含其他仓位，汇总金额不能单独归给当前策略" } else { "" })}</p>
        <p class="automation-receipt-problem">{move || close.try_with(|close| close.problem.as_ref().or(close.finality_problem.as_ref()).map(|problem| format!("{} · {}", problem.code, problem.message)))}</p>
    </details> }
}

fn exit_reason_label(reason: Option<&str>) -> &str {
    let Some(reason) = reason.filter(|reason| !reason.trim().is_empty()) else {
        return "退出原因未记录";
    };
    // This is the recorded reason, not a claim that the close succeeded or made a profit.
    match reason
        .strip_prefix("auto_pair_exit trigger=")
        .and_then(|value| value.split_ascii_whitespace().next())
    {
        Some("take_profit") => "退出原因：自动止盈",
        Some("stop_loss") => "退出原因：自动止损",
        Some("liquidation_guard") => "退出原因：强平距离保护",
        Some(_) => "退出原因：自动退出，触发类型待确认",
        None => reason,
    }
}

pub(super) fn leg_confirmed(leg: &ExecutionRunLeg, paper: bool) -> bool {
    leg.state == LiveOrderState::Filled
        && leg
            .filled_quantity
            .is_some_and(|qty| qty.is_finite() && qty > 0.0)
        && leg.confirmed_filled_at_ms.is_some()
        && ((paper
            && leg.finality_source == Some(OrderUpdateSource::AdapterAck)
            && leg
                .filled_notional_usd
                .is_some_and(|value| value.is_finite() && value > 0.0))
            || !matches!(
                leg.finality_source,
                None | Some(
                    OrderUpdateSource::Unknown
                        | OrderUpdateSource::AdapterAck
                        | OrderUpdateSource::Manual
                        | OrderUpdateSource::FundingPoller
                )
            ))
}

pub(super) fn exit_confirmed(receipt: &AutomationExecutionReceipt) -> bool {
    receipt.close_runs.iter().any(|close| {
        close.status == CloseRunStatus::Succeeded
            && [
                shared_types::PositionSide::Long,
                shared_types::PositionSide::Short,
            ]
            .iter()
            .all(|side| {
                let opened = if *side == shared_types::PositionSide::Long {
                    &receipt.run.long_leg
                } else {
                    &receipt.run.short_leg
                };
                close.legs.iter().any(|leg| {
                    leg.status == CloseLegStatus::Filled
                        && leg.side == *side
                        && leg.quantity.is_finite()
                        && opened
                            .filled_quantity
                            .is_some_and(|qty| qty.is_finite() && qty > 0.0 && leg.quantity >= qty)
                        && leg.confirmed_filled_at_ms.is_some()
                        && ((receipt.mode == Some(shared_types::ExecutionMode::DryRun)
                            && leg.finality_source == Some(OrderUpdateSource::AdapterAck)
                            && leg.order.as_ref().is_some_and(|order| {
                                order.intent.mode == shared_types::ExecutionMode::DryRun
                                    && order.state == LiveOrderState::Filled
                                    && order
                                        .filled_quantity
                                        .is_some_and(|qty| qty.is_finite() && qty >= leg.quantity)
                                    && order
                                        .filled_price
                                        .is_some_and(|price| price.is_finite() && price > 0.0)
                            }))
                            || !matches!(
                                leg.finality_source,
                                None | Some(
                                    OrderUpdateSource::Unknown
                                        | OrderUpdateSource::AdapterAck
                                        | OrderUpdateSource::Manual
                                        | OrderUpdateSource::FundingPoller
                                )
                            ))
                        && leg.pair_evidence.as_ref().is_some_and(|pair| {
                            pair.side == *side
                                && AutomationExecutionReceipt::matches_pair(&receipt.run, pair)
                        })
                })
            })
    })
}

fn number(value: Option<f64>) -> String {
    value.filter(|value| value.is_finite()).map_or_else(
        || "待确认".into(),
        |value| {
            format!("{value:.8}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
        },
    )
}

fn source_label(source: Option<OrderUpdateSource>) -> &'static str {
    match source {
        Some(OrderUpdateSource::PrivateWs) => "私有 WS",
        Some(OrderUpdateSource::OrderQuery) => "订单查询",
        Some(OrderUpdateSource::Reconcile) => "对账",
        Some(OrderUpdateSource::Internal) => "内部账本",
        Some(OrderUpdateSource::Manual) => "人工记录",
        Some(OrderUpdateSource::AdapterAck) => "受理确认，非成交最终结果",
        Some(OrderUpdateSource::FundingPoller) => "资金费 记录",
        _ => "待确认",
    }
}

fn order_label(state: LiveOrderState) -> &'static str {
    match state {
        LiveOrderState::Created => "已创建",
        LiveOrderState::RiskChecked => "交易检查通过",
        LiveOrderState::Submitted => "已提交",
        LiveOrderState::Accepted => "已受理",
        LiveOrderState::PartiallyFilled => "部分成交",
        LiveOrderState::Filled => "已成交",
        LiveOrderState::CancelRequested => "撤单待确认",
        LiveOrderState::Cancelled => "已撤单",
        LiveOrderState::Rejected => "已拒绝",
        LiveOrderState::Failed => "失败",
        LiveOrderState::Unknown => "未知",
    }
}

fn run_label(state: ExecutionRunState) -> &'static str {
    match state {
        ExecutionRunState::Previewed => "已预览",
        ExecutionRunState::RiskChecked => "交易检查通过",
        ExecutionRunState::SubmittingFirstLeg => "首腿提交中",
        ExecutionRunState::FirstLegPartial => "首腿部分成交",
        ExecutionRunState::SubmittingSecondLeg => "次腿提交中",
        ExecutionRunState::SecondLegSubmitted => "双腿提交，等待最终结果",
        ExecutionRunState::Hedged => "已对冲",
        ExecutionRunState::UnwindRequired => "需要补偿",
        ExecutionRunState::Unwinding => "补偿中",
        ExecutionRunState::FailedSafe => "失败，需处置",
        ExecutionRunState::Closed => "执行已收口",
    }
}

fn close_label(state: CloseRunStatus) -> &'static str {
    match state {
        CloseRunStatus::Submitted => "已提交，等待成交",
        CloseRunStatus::Succeeded => "本次平仓已成交",
        CloseRunStatus::PartiallySubmitted => "部分提交",
        CloseRunStatus::UnwindRequired => "需要补偿",
        CloseRunStatus::CompensationSubmitted => "补偿待确认",
        CloseRunStatus::Compensated => "补偿已完成",
        CloseRunStatus::CompensationFailed => "补偿失败",
        CloseRunStatus::ManuallyResolved => "人工终结",
        CloseRunStatus::Failed => "平仓失败",
    }
}

fn close_leg_label(state: CloseLegStatus) -> &'static str {
    match state {
        CloseLegStatus::Submitted => "已提交",
        CloseLegStatus::Accepted => "已受理",
        CloseLegStatus::PartiallyFilled => "部分成交",
        CloseLegStatus::Filled => "已成交",
        CloseLegStatus::CancelRequested => "撤单待确认",
        CloseLegStatus::Cancelled => "已撤单",
        CloseLegStatus::Rejected => "已拒绝",
        CloseLegStatus::Failed => "失败",
        CloseLegStatus::Skipped => "未提交",
    }
}
