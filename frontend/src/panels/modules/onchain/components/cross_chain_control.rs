use super::super::data::{
    authorization_key, next_recheck_position, next_submit_position, OnchainCrossChainData,
};
use super::super::format::{chain_label, cost_percent_label, raw_amount_label, usd};
use leptos::prelude::*;
use shared_types::{
    OnchainCrossChainBuildResponse, OnchainCrossChainLegKind as Kind,
    OnchainCrossChainLegRunStatus as LegStatus, OnchainCrossChainRecheckRequest,
    OnchainCrossChainRun, OnchainCrossChainRunStatus as RunStatus, OnchainCrossChainSubmitRequest,
    ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE,
};

mod accounting;
mod recovery;
mod disposition;
mod recovery_plans;

pub(super) fn cross_chain_control(
    data: OnchainCrossChainData,
    clock: RwSignal<i64>,
) -> impl IntoView {
    view! {
        <Show when=move || data.build.with(Option::is_some)
            || data.recovery.with(|state| !state.rows.is_empty() || state.read_problem.is_some() || state.recovery_problem.is_some())>
            <section class="onchain-cross-chain-execution" aria-label="跨链执行与到账记录">
                <header>
                    <h3>"跨链执行与到账"</h3>
                    <button type="button" class="row-action"
                        disabled=move || data.refreshing.get() || data.authorizing.get() || data.submitting.get() || data.building.get() || data.rechecking.get()
                        on:click=move |_| data.refresh.run(())>
                        {move || if data.refreshing.get() { "刷新中…" } else { "刷新记录" }}
                    </button>
                </header>
                {move || data.recovery.with(|state| state.read_problem.clone()).map(|problem| view! {
                    <p class="cross-chain-notice is-warning" role="status">"运行记录读取失败，保留上次结果："{problem}</p>
                })}
                {move || data.recovery.with(|state| state.recovery_problem.clone()).map(|problem| view! {
                    <p class="cross-chain-notice is-danger" role="alert">{problem}</p>
                })}
                {move || data.recovery.with(|state| state.problem.clone()).map(|problem| view! {
                    <p class="cross-chain-notice is-danger" role="alert">"上次操作反馈："{problem}</p>
                })}
                {move || data.recovery.with(|state| state.pending_submission.is_some() || state.pending_authorization.is_some()).then(|| view! {
                    <p class="cross-chain-notice is-warning" role="status">"正在核对请求结果，尚未确认失败。"</p>
                })}
                {move || data.build.get().and_then(Result::ok).map(|build| authorization_form(build, data, clock))}
                {move || {
                    let state = data.recovery.get();
                    (state.rows.len() > 1).then(|| {
                        let options = state.rows.iter().map(|run| view! {
                            <option value=run.run_id.clone() selected=Some(&run.run_id) == state.selected_id.as_ref()>
                                {format!("{} → {} · {} · {}", chain_label(&run.build.source_chain),
                                    chain_label(&run.build.peer_chain), status_label(run.status), run.run_id)}
                            </option>
                        }).collect_view();
                        view! {
                            <label class="cross-chain-run-picker">"运行记录"
                                <select on:change=move |event| data.recovery.update(|state| state.selected_id = Some(event_target_value(&event)))>
                                    {options}
                                </select>
                            </label>
                        }
                    })
                }}
                {move || data.recovery.with(|state| state.selected().cloned()).map(|run| run_panel(run, data, clock))}
            </section>
        </Show>
    }
}

fn authorization_form(
    build: OnchainCrossChainBuildResponse,
    data: OnchainCrossChainData,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let id = build.build_id.clone();
    let key = authorization_key(&id);
    let key_for_disabled = key.clone();
    let blockers = build
        .blockers
        .iter()
        .chain(build.warnings.iter())
        .map(|message| {
            view! {
                <li>{message.clone()}</li>
            }
        })
        .collect_view();
    let permitted = build.submit_ready
        && !build.monitor_only
        && build.blockers.is_empty()
        && build.quote_usd_valuation.is_some();
    view! {
            <div class="cross-chain-authorization" hidden=move || data.recovery.with(|state| state.rows.iter().any(|run| run.build.build_id == id))>
                <div class="cross-chain-plan-summary">
                    <strong>{format!("{} → {} → {}", chain_label(&build.source_chain), chain_label(&build.peer_chain), chain_label(&build.source_chain))}</strong>
                    <span>{format!("桥费 {} · 链费 {} · 汇率风险预留 {}", build.bridge_fee_usd.map_or_else(|| "待确认".into(), usd),
                        build.gas_usd.map_or_else(|| "待确认".into(), usd), cost_percent_label(f64::from(build.stablecoin_risk_bps)))}</span>
                    <span>{build.quote_usd_valuation.as_ref().map_or_else(
                        || "美元汇率待确认，净收益待估值".to_owned(),
                        |rate| format!("美元估值 · 1 {} = ${:.6} · {} {}", rate.asset, rate.usd_bid, rate.venue.to_uppercase(), rate.symbol)
                    )}</span>
                    <span>{format!("已选独立费用：授权 {} 笔 · 补库 {} 笔 · 计入后净回报 {}", build.approval_costs.len(), build.replenishment_costs.len(), build.net_return_bps.map_or_else(|| "待核算".into(), cost_percent_label))}</span>
                </div>
                <ul class="cross-chain-notes">{blockers}</ul>
                <p class="cross-chain-notice is-warning">"实盘资金操作 · 四步并非同时成交，跨链期间价格可能变化。"</p>
                <label>"本次授权短语"
                    <input type="text" autocomplete="off" spellcheck="false"
                        placeholder=ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE
                        prop:value=move || data.confirmation.get()
                        on:input=move |event| data.confirmation.set(event_target_value(&event)) />
                </label>
                <div class="cross-chain-actions">
                    <span class="cross-chain-validity">{move || if clock.get() >= build.valid_until_ms { "预览已过期，需重新生成" } else { "预览有效" }}</span>
                    <button type="button" class="row-action"
                        disabled=move || data.authorizing.get() || data.submitting.get() || data.building.get() || data.rechecking.get()
                            || data.confirmation.get() != ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE
                            || !data.recovery.with(|state| state.can_authorize(&key_for_disabled, clock.get())
                                && (state.pending_authorization.as_ref() == Some(&key_for_disabled)
                                    || permitted && clock.get() < build.valid_until_ms))
                        on:click=move |_| data.authorize.run(data.confirmation.get_untracked())>
                        {move || if data.authorizing.get() { "授权中…" } else if data.recovery.with(|state| state.pending_authorization.as_ref() == Some(&key)) { "重试同一授权" } else { "确认本次授权" }}
                    </button>
                </div>
            </div>
    }
}

fn wallet_receipt(
    label: &'static str,
    receipt: Option<&shared_types::OnchainWalletReceipt>,
) -> impl IntoView {
    let status = receipt.map_or("尚未核验", |r| match r.status {
        shared_types::OnchainChainSettlementStatus::Complete => "收支已核验",
        shared_types::OnchainChainSettlementStatus::Pending => "收支核验中",
        shared_types::OnchainChainSettlementStatus::ReviewRequired => "需要复核",
    });
    let fee = receipt.and_then(|r| r.network_cost.as_ref()).map_or_else(
        || "网络费待确认".into(),
        |cost| {
            let amount = cost.total_fee_exact.as_deref().unwrap_or("待确认");
            let paid_by_wallet = receipt.is_some_and(|r| {
                if r.basis.chain == "solana" {
                    cost.payer == r.basis.wallet
                } else {
                    cost.payer.eq_ignore_ascii_case(&r.basis.wallet)
                }
            });
            let payer = if cost.payer.is_empty() {
                "付款方待确认"
            } else if paid_by_wallet {
                "本钱包支付"
            } else {
                "其他地址支付，不重复计入本钱包链费"
            };
            format!("网络费 {amount} {} · {payer}", cost.asset)
        },
    );
    let native = receipt.and_then(|r| {
        r.additional_native_change_raw
            .as_deref()
            .filter(|v| *v != "0")
            .map(|v| {
                let sign = if v.starts_with('-') { "-" } else { "+" };
                let symbol = r
                    .network_cost
                    .as_ref()
                    .map_or("原生币", |cost| cost.asset.as_str());
                let amount = raw_amount_label(
                    v.trim_start_matches('-'),
                    if r.basis.chain == "solana" { 9 } else { 18 },
                    symbol,
                );
                format!("额外原生币变动：{sign}{amount}（不含网络费）")
            })
    });
    view! { <div><strong>{label}</strong><span>{status}</span><span>{fee}</span>{native.map(|v| view! { <span>{v}</span> })}</div> }
}

fn run_panel(
    run: OnchainCrossChainRun,
    data: OnchainCrossChainData,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let legs = run.legs.iter().map(|progress| {
        let contract = run.build.legs.iter().find(|leg| leg.position == progress.position);
        let input = progress.actual_input_amount_raw.as_deref().map(|raw| contract.map_or_else(|| raw.to_owned(), |leg| raw_amount_label(raw, leg.input_decimals, &leg.from_asset)));
        let submitted = progress.submitted_input_amount_raw.as_deref().map(|raw| contract.map_or_else(|| raw.to_owned(), |leg| raw_amount_label(raw, leg.input_decimals, &leg.from_asset)));
        let output = progress.actual_output_amount_raw.as_deref().map(|raw| contract.map_or_else(|| raw.to_owned(), |leg| raw_amount_label(raw, leg.output_decimals, &leg.to_asset)));
        let route = contract.map_or_else(|| "链身份待确认".into(), |leg| format!("{} → {}", chain_label(&leg.from_chain), chain_label(&leg.to_chain)));
        let transactions = progress.source_transaction_id.iter().map(|hash| ("提交交易", hash))
            .chain(progress.destination_transaction_id.iter().map(|hash| ("到账交易", hash)))
            .map(|(label, hash)| view! { <div><dt>{label}</dt><dd><code>{hash.clone()}</code></dd></div> }).collect_view();
        view! {
            <li class="cross-chain-progress-leg" data-position=progress.position data-status=leg_status_label(progress.status)>
                <div class="cross-chain-leg-title"><strong>{format!("{} · {}", progress.position, kind_label(progress.kind))}</strong><small>{route}</small></div>
                <span class=if matches!(progress.status, LegStatus::Paused | LegStatus::Failed) {
                    "cross-chain-leg-status is-warning"
                } else { "cross-chain-leg-status" }>{leg_status_label(progress.status)}</span>
                <dl class="cross-chain-leg-amounts">
                    <div><dt>"提交数量"</dt><dd>{submitted.unwrap_or_else(|| "尚未提交".into())}</dd></div>
                    <div><dt>"实际扣款"</dt><dd>{input.unwrap_or_else(|| "尚未确认".into())}</dd></div>
                    <div><dt>{if progress.bridge_recovery.is_some() { "原路径到账" } else { "实际到账" }}</dt><dd>{output.unwrap_or_else(|| if progress.bridge_recovery.is_some() { "路径已中止".into() } else { "尚未确认".into() })}</dd></div>
                </dl>
                <div class="cross-chain-receipts">
                    {wallet_receipt("源链收支", progress.source_receipt.as_ref())}
                    {progress.bridge_execution.as_ref().map(|_| wallet_receipt("目标链收支", progress.destination_receipt.as_ref()))}
                </div>
                {progress.bridge_recovery.as_ref().map(recovery::summary)}
                <dl class="cross-chain-transactions">{transactions}</dl>
                {progress.problem.clone().map(|problem| view! { <p class="cross-chain-notice is-warning">{problem}</p> })}
            </li>
        }
    }).collect_view();
    let submit_run = run.clone();
    let disabled_run = run.clone();
    let label_run = run.clone();
    let recheck_request =
        next_recheck_position(&run).map(|position| OnchainCrossChainRecheckRequest {
            run_id: run.run_id.clone(),
            expected_position: position,
        });
    let status = run.status;
    let deadline = run.authorization.valid_until_ms;
    let verification_run = run.clone();
    let recovery_preview_controls = disposition::controls(&run, data, clock);
    let recovery_plans_panel = recovery_plans::panel(run.run_id.clone(), data, clock);
    view! {
        <div class="cross-chain-run">
            <div class="cross-chain-run-heading">
                <strong>{move || if status == RunStatus::AuthorizedAwaitingSubmit && clock.get() >= deadline { "授权已过期" } else { status_label(status) }}</strong>
                <code>{run.run_id}</code>
            </div>
            {move || verification_label(&verification_run, clock.get()).map(|label| view! {
                <p class="cross-chain-notice" role="status">{label}</p>
            })}
            {run.accounting.as_ref().map(accounting::summary)}
            {run.accounting.as_ref().and_then(|accounting| accounting.disposition.as_ref()).map(disposition::summary)}
            {recovery_preview_controls}
            {recovery_plans_panel}
            <ol class="cross-chain-progress">{legs}</ol>
            {run.problem.map(|problem| view! { <p class="cross-chain-notice is-warning">{problem}</p> })}
            <div class="cross-chain-actions">
                <p>{run.next_action}</p>
                {recheck_request.map(|request| {
                    let request_for_disabled = request.clone();
                    view! {
                        <button type="button" class="row-action"
                            disabled=move || data.rechecking.get() || data.submitting.get() || data.authorizing.get() || data.building.get()
                                || !data.recovery.with(|state| state.can_recheck(&request_for_disabled))
                            on:click=move |_| data.recheck.run(request.clone())>
                            {move || if data.rechecking.get() { "恢复核验中…" } else { "重新核验到账" }}
                        </button>
                    }
                })}
                <button type="button" class="row-action"
                    disabled=move || data.submitting.get() || data.authorizing.get() || data.building.get() || data.rechecking.get()
                        || next_submit_position(&disabled_run, clock.get()).is_none_or(|position|
                            !data.recovery.with(|state| state.can_submit(&OnchainCrossChainSubmitRequest {
                                run_id: disabled_run.run_id.clone(), expected_position: position,
                            }, clock.get())))
                    on:click=move |_| {
                        if let Some(position) = next_submit_position(&submit_run, clock.get_untracked()) {
                            data.submit_next_leg.run(OnchainCrossChainSubmitRequest {
                                run_id: submit_run.run_id.clone(), expected_position: position,
                            });
                        }
                    }>
                    {move || if data.submitting.get() { "提交中…".into() } else {
                        next_submit_position(&label_run, clock.get()).map_or_else(|| "当前无可提交步骤".into(), |position| format!("重报价并提交第 {position} 步"))
                    }}
                </button>
            </div>
        </div>
    }
}

fn verification_label(run: &OnchainCrossChainRun, now_ms: i64) -> Option<String> {
    if !matches!(run.status, RunStatus::AwaitingSourceFinality | RunStatus::AwaitingDestinationEvidence | RunStatus::Paused) {
        return None;
    }
    let leg = run.active_leg()?;
    if leg.recovery_started_at_ms.is_some() {
        return Some(format!("只读核验 {}/{} 轮 · 不重发转账", leg.recovery_checks,
            shared_types::OnchainCrossChainLegProgress::RECOVERY_CHECK_LIMIT));
    }
    let deadline = run.automatic_check_deadline_ms()?;
    let remaining = deadline.saturating_sub(now_ms).max(0).saturating_add(59_999) / 60_000;
    Some(if remaining == 0 { "自动核验窗口已到 · 到账结果仍需核对".into() }
        else { format!("自动核验剩余 {remaining} 分钟 · 非到账承诺") })
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::SourceSwap => "源链买入",
        Kind::OutboundBridge => "资产跨链",
        Kind::TargetSwap => "目标链卖出",
        Kind::ReturnBridge => "资金回链",
    }
}
fn status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::AuthorizedAwaitingSubmit => "已授权，待提交",
        RunStatus::AuthorizationExpired => "授权已过期",
        RunStatus::Running => "执行中",
        RunStatus::AwaitingSourceFinality => "等待源链确认",
        RunStatus::AwaitingDestinationEvidence => "等待目标链到账",
        RunStatus::Paused => "已暂停",
        RunStatus::Compensating => "等待补偿处理",
        RunStatus::Completed => "资产路径已完成",
        RunStatus::Failed => "执行失败",
    }
}
fn leg_status_label(status: LegStatus) -> &'static str {
    match status {
        LegStatus::RequoteRequired => "待重报价",
        LegStatus::SubmissionClaimed => "已锁定提交",
        LegStatus::Submitted => "已提交",
        LegStatus::SourceConfirmed => "源链已确认",
        LegStatus::Completed => "到账已确认",
        LegStatus::Paused => "已暂停",
        LegStatus::Failed => "失败",
    }
}

#[cfg(test)]
#[path = "cross_chain_control_tests.rs"]
mod tests;
