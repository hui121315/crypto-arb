use super::*;
use shared_types::{
    OnchainCrossChainRecoveryAuthorizeRequest, OnchainCrossChainRecoveryPlan as Plan,
    OnchainCrossChainRecoveryPlanStatus as Status, ONCHAIN_RECOVERY_RESERVATION_PHRASE,
};

pub(super) fn panel(
    run_id: String,
    data: OnchainCrossChainData,
    clock: RwSignal<i64>,
) -> impl IntoView {
    move || {
        let plans = data.recovery.with(|state| {
            state
                .plans
                .iter()
                .filter(|p| p.preview.source_run_id == run_id)
                .cloned()
                .collect::<Vec<_>>()
        });
        (!plans.is_empty()).then(|| {
            let rows = plans.into_iter().map(|plan| row(plan, data, clock)).collect_view();
            view! { <section class="cross-chain-accounting" aria-label="已保存处置计划">
                <header><div><h3>"已保存处置计划"</h3><span>"预留不发送交易"</span></div></header>
                {rows}
                <p class="cross-chain-accounting-scope">"预留范围为本产品跨链执行模块的同链钱包；未在链上冻结资产，尚未接入其他交易模块的统一资金占用。报价过期或取消后释放。"</p>
            </section> }
        })
    }
}

pub(super) fn row(plan: Plan, data: OnchainCrossChainData, clock: RwSignal<i64>) -> impl IntoView {
    let status = plan.status;
    let deadline = plan.preview.valid_until_ms.unwrap_or(0);
    let reserve_id = plan.plan_id.clone();
    let cancel_id = plan.plan_id.clone();
    let input = &plan.preview.input;
    let target = &plan.preview.target.asset;
    let minimum = plan
        .preview
        .minimum_output_amount_raw
        .as_deref()
        .map(|raw| raw_amount_label(raw, target.decimals, &target.symbol))
        .unwrap_or_else(|| "待核实".into());
    let fee = |amount: Option<f64>| {
        amount
            .map(|v| {
                if v > 0.0 && v < 0.000001 {
                    "<$0.000001".into()
                } else {
                    format!("${v:.6}")
                }
            })
            .unwrap_or_else(|| "未知".into())
    };
    view! {
        <details class="cross-chain-accounting-detail" open=matches!(status, Status::Reserved | Status::AwaitingAuthorization)>
            <summary>{format!("{} {} → {} · ", input.amount_exact, input.asset.symbol, target.symbol)}
                {move || status_label(status, deadline, clock.get())}</summary>
            <dl class="cross-chain-transactions"><div><dt>"计划"</dt><dd><code>{plan.plan_id}</code></dd></div>
                <div><dt>"最低到账"</dt><dd>{minimum}</dd></div>
                <div><dt>"费用估值"</dt><dd>{format!("报价 {} · Gas {}", fee(plan.preview.fee_usd), fee(plan.preview.gas_usd))}</dd></div>
                <div><dt>"来源钱包"</dt><dd><code>{input.wallet.clone()}</code></dd></div>
                <div><dt>"来源链"</dt><dd>{chain_label(&input.chain)}</dd></div>
            </dl>
            <div class="cross-chain-actions">
                <button type="button" class="row-action"
                    disabled=move || { status != Status::AwaitingAuthorization || clock.get() >= deadline || data.recovery_mutating.get() || data.recovery_previewing.get() }
                    on:click=move |_| data.reserve_recovery.run(OnchainCrossChainRecoveryAuthorizeRequest {
                        plan_id: reserve_id.clone(), idempotency_key: format!("recovery-reserve-{reserve_id}"),
                        confirmation: ONCHAIN_RECOVERY_RESERVATION_PHRASE.into(),
                    })>"确认计划并预留"</button>
                <button type="button" class="row-action"
                    disabled=move || { matches!(status, Status::Expired | Status::Cancelled) || clock.get() >= deadline || data.recovery_mutating.get() }
                    on:click=move |_| data.cancel_recovery.run(cancel_id.clone())>"取消计划 / 释放预留"</button>
            </div>
        </details>
    }
}

fn status_label(status: Status, deadline: i64, now_ms: i64) -> &'static str {
    if status == Status::Cancelled {
        "已取消"
    } else if now_ms >= deadline || status == Status::Expired {
        "已过期，未提交"
    } else if status == Status::Reserved {
        "已预留，未提交"
    } else {
        "待确认"
    }
}
