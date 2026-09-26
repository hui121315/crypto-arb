use super::*;
use shared_types::{OnchainCrossChainLeg, OnchainCrossChainLegProgress};

pub(super) fn panel(initial: OnchainCrossChainRun, data: OnchainCrossChainData, clock: RwSignal<i64>) -> impl IntoView {
    let id = initial.run_id.clone();
    let run = Memo::new(move |_| data.recovery.with(|state| state.rows.iter().find(|run| run.run_id == id).cloned())
        .unwrap_or_else(|| initial.clone()));
    let next = Memo::new(move |_| run.with(|run| next_submit_position(run, clock.get())));
    let recheck = Memo::new(move |_| run.with(|run| next_recheck_position(run).map(|position|
        OnchainCrossChainRecheckRequest { run_id: run.run_id.clone(), expected_position: position })));
    let pending_position = Memo::new(move |_| data.recovery.with(|state| state.pending_submission.as_ref()
        .filter(|pending| run.with(|run| run.run_id == pending.request.run_id))
        .map(|pending| pending.request.expected_position)));
    view! {
        <div class="cross-chain-run">
            <div class="cross-chain-run-heading">
                <strong>{move || run.with(|run| if run.status == RunStatus::AuthorizedAwaitingSubmit && clock.get() >= run.authorization.valid_until_ms {
                    "授权已过期"
                } else { status_label(run.status) })}</strong>
                <code>{move || run.with(|run| run.run_id.clone())}</code>
                <a class="row-action" href=move || run.with(|run| crate::panels::routing::settlement_review_href(
                    shared_types::review::settlements::SettlementSource::CrossChain, &run.run_id))>"查看收支复盘"</a>
            </div>
            <div class="cross-chain-actions cross-chain-next-action">
                <p>{move || pending_position.get().map_or_else(|| run.with(|run| run.next_action.clone()),
                    |position| format!("第 {position} 步提交结果未确认，正在核对原记录。"))}</p>
                {move || recheck.get().map(|request| {
                    let current = request.clone();
                    view! {
                        <button type="button" class="row-action"
                            disabled=move || data.rechecking.get() || data.submitting.get() || data.authorizing.get() || data.building.get()
                                || !data.recovery.with(|state| state.can_recheck(&current))
                            on:click=move |_| data.recheck.run(request.clone())>
                            {move || if data.rechecking.get() { "恢复核对中…" } else { "重新核对到账" }}
                        </button>
                    }
                })}
                <button type="button" class="row-action cross-chain-submit-step"
                    hidden=move || run.with(|run| run.status == RunStatus::Completed)
                    disabled=move || data.submitting.get() || data.authorizing.get() || data.building.get() || data.rechecking.get()
                        || next.get().is_none_or(|position| !data.recovery.with(|state| state.can_submit(&OnchainCrossChainSubmitRequest {
                            run_id: run.with(|run| run.run_id.clone()), expected_position: position,
                        }, clock.get())))
                    on:click=move |_| {
                        if let Some(position) = run.with_untracked(|run| next_submit_position(run, clock.get_untracked())) {
                            data.submit_next_leg.run(OnchainCrossChainSubmitRequest {
                                run_id: run.with_untracked(|run| run.run_id.clone()), expected_position: position,
                            });
                        }
                    }>
                    {move || if data.submitting.get() { "提交中…".into() } else if let Some(position) = pending_position.get() {
                        format!("等待第 {position} 步处理结果")
                    } else {
                        next.get().map_or_else(|| "当前无可提交步骤".into(), |position| format!("重报价并提交第 {position} 步"))
                    }}
                </button>
            </div>
            {move || run.with(|run| verification_label(run, clock.get())).map(|label| view! {
                <p class="cross-chain-notice" role="status">{label}</p>
            })}
            {move || run.with(|run| run.problem.clone()).map(|problem| view! {
                <p class="cross-chain-notice is-warning">{problem}</p>
            })}
            <div class="cross-chain-step-summary">
                <strong>"执行路径"</strong>
                <span>{move || run.with(|run| format!("已完成 {} / {} 步", run.legs.iter().filter(|leg| leg.status == LegStatus::Completed).count(), run.legs.len()))}</span>
            </div>
            <ol class="cross-chain-progress">
                <For each=move || run.with(|run| run.legs.clone()) key=|leg| leg.position
                    children=move |initial| {
                        let position = initial.position;
                        let progress = Memo::new(move |_| run.with(|run| run.legs.iter().find(|leg| leg.position == position).cloned()).unwrap_or_else(|| initial.clone()));
                        let contract = Memo::new(move |_| run.with(|run| run.build.legs.iter().find(|leg| leg.position == position).cloned()));
                        progress_row(progress, contract)
                    } />
            </ol>
            {move || run.with(|run| run.accounting.clone()).map(|value| accounting::summary(&value))}
            {move || run.with(|run| run.accounting.as_ref().and_then(|accounting| accounting.disposition.clone())).map(|value| disposition::summary(&value))}
            {move || run.with(|run| run.status == RunStatus::Completed && run.accounting.is_none()).then(|| view! {
                <p class="cross-chain-notice is-warning" role="status">"资产路径已完成；费用与净收益仍待核对。"</p>
            })}
        </div>
    }
}

fn progress_row(progress: Memo<OnchainCrossChainLegProgress>, contract: Memo<Option<OnchainCrossChainLeg>>) -> impl IntoView {
    view! {
        <li class="cross-chain-progress-leg" data-position=move || progress.with(|leg| leg.position)
            data-status=move || progress.with(|leg| leg_status_label(leg.status))>
            <div class="cross-chain-leg-title">
                <strong>{move || progress.with(|leg| format!("{} · {}", leg.position, kind_label(leg.kind)))}</strong>
                <small>{move || contract.with(|leg| leg.as_ref().map_or_else(|| "链身份待确认".into(), |leg| {
                    if leg.from_chain == leg.to_chain { chain_label(&leg.from_chain).to_owned() }
                    else { format!("{} → {}", chain_label(&leg.from_chain), chain_label(&leg.to_chain)) }
                }))}</small>
            </div>
            <span class=move || progress.with(|leg| match leg.status {
                LegStatus::Completed => "cross-chain-leg-status is-positive",
                LegStatus::Failed => "cross-chain-leg-status is-danger",
                LegStatus::RequoteRequired => "cross-chain-leg-status",
                _ => "cross-chain-leg-status is-warning",
            })>{move || progress.with(|leg| leg_status_label(leg.status))}</span>
            <div class="cross-chain-leg-output"><small>"实际到账"</small>
                <strong class="num">{move || progress.with(|leg| leg.actual_output_amount_raw.as_deref().map_or_else(|| {
                    if leg.status == LegStatus::RequoteRequired { "尚未提交".into() } else { "待确认".into() }
                }, |raw| amount(raw, contract.get().as_ref(), false)))}</strong>
            </div>
            <details class="cross-chain-leg-details">
                <summary>"交易与收支"</summary>
                {move || progress.with(|leg| {
                    let contract = contract.get();
                    let amount = |value: Option<&str>, input| value.map(|raw| amount(raw, contract.as_ref(), input));
                    let transactions = leg.source_transaction_id.iter().map(|hash| ("提交交易", hash))
                        .chain(leg.destination_transaction_id.iter().map(|hash| ("到账交易", hash)))
                        .map(|(label, hash)| view! { <div><dt>{label}</dt><dd><code>{hash.clone()}</code></dd></div> }).collect_view();
                    view! {
                        <dl class="cross-chain-leg-amounts">
                            <div><dt>"提交数量"</dt><dd>{amount(leg.submitted_input_amount_raw.as_deref(), true).unwrap_or_else(|| "尚未提交".into())}</dd></div>
                            <div><dt>"实际扣款"</dt><dd>{amount(leg.actual_input_amount_raw.as_deref(), true).unwrap_or_else(|| "尚未确认".into())}</dd></div>
                            <div><dt>{if leg.bridge_recovery.is_some() { "原路径到账" } else { "实际到账" }}</dt>
                                <dd>{amount(leg.actual_output_amount_raw.as_deref(), false).unwrap_or_else(|| if leg.bridge_recovery.is_some() { "路径已中止".into() } else { "尚未确认".into() })}</dd></div>
                        </dl>
                        <div class="cross-chain-receipts">
                            {wallet_receipt("源链收支", leg.source_receipt.as_ref())}
                            {matches!(leg.kind, Kind::OutboundBridge | Kind::ReturnBridge).then(|| wallet_receipt("目标链收支", leg.destination_receipt.as_ref()))}
                        </div>
                        {leg.bridge_recovery.as_ref().map(recovery::summary)}
                        <dl class="cross-chain-transactions">{transactions}</dl>
                    }
                })}
            </details>
            {move || progress.with(|leg| leg.problem.clone()).map(|problem| view! { <p class="cross-chain-notice is-warning">{problem}</p> })}
        </li>
    }
}

fn amount(raw: &str, contract: Option<&OnchainCrossChainLeg>, input: bool) -> String {
    contract.map_or_else(|| format!("{raw} 最小单位"), |leg| {
        if input { raw_amount_label(raw, leg.input_decimals, &leg.from_asset) }
        else { raw_amount_label(raw, leg.output_decimals, &leg.to_asset) }
    })
}
