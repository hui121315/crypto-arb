use super::*;
use shared_types::{OnchainCrossChainDisposition, OnchainCrossChainDispositionAction as Action};
use shared_types::{OnchainCrossChainRecoveryPreview, OnchainCrossChainRecoveryPreviewRequest};

pub(super) fn summary(plan: &OnchainCrossChainDisposition) -> impl IntoView {
    let blocked = !plan.blockers.is_empty();
    let observed = plan
        .receipts_observed_at_ms
        .and_then(crate::panels::modules::timestamp::local_date_hm)
        .unwrap_or_else(|| "待核实".into());
    let assets = plan.remaining_assets.iter().map(|row| {
        let next = match row.action {
            Action::Keep => "已在原钱包，保留原报价币",
            Action::QuoteSwap => "留在当前链，重新询价换回原报价币",
            Action::QuoteBridge => "重新询价跨链返回原钱包及原报价币",
            Action::ReviewWallet => "钱包不同，先核对归属及转回路径",
        };
        view! {
            <div class="cross-chain-net-asset">
                <div><strong>{row.change.asset.symbol.clone()}</strong><small>{chain_label(&row.change.chain)}</small></div>
                <span>{row.change.amount_exact.clone()}</span>
                <details><summary>{next}</summary>
                    <dl><div><dt>"钱包"</dt><dd><code>{row.change.wallet.clone()}</code></dd></div>
                        <div><dt>"合约"</dt><dd><code>{row.change.asset.address.clone()}</code></dd></div></dl>
                </details>
            </div>
        }
    }).collect_view();
    let target = plan.original_capital.as_ref().map(|target| view! {
        <details class="cross-chain-accounting-detail"><summary>{format!("原投入 {} {} · {}", target.amount_exact, target.asset.symbol, chain_label(&target.chain))}</summary>
            <dl><div><dt>"钱包"</dt><dd><code>{target.wallet.clone()}</code></dd></div>
                <div><dt>"合约"</dt><dd><code>{target.asset.address.clone()}</code></dd></div></dl>
        </details>
    });
    let blockers = plan
        .blockers
        .iter()
        .map(|problem| view! { <li>{problem.clone()}</li> })
        .collect_view();
    view! {
        <section class="cross-chain-accounting cross-chain-disposition" aria-label="本次剩余资金与处置建议">
            <header><div><h3>"本次剩余资金"</h3><span>{if blocked { "待核齐原交易" } else { "只读处置建议 · 未询价" }}</span></div>
                <small>{format!("回执时间 {observed}")}</small>
            </header>
            <div class="cross-chain-net-assets">{assets}</div>
            {target}
            {blocked.then(|| view! { <ul class="cross-chain-notice is-warning">{blockers}</ul> })}
            {(!blocked && plan.remaining_assets.is_empty()).then(|| view! { <p>"已核实收支中没有正的剩余资金。"</p> })}
            <p class="cross-chain-accounting-scope">"原投入加已核实收支，已扣后续花费；不是当前钱包余额，也不代表美元盈亏。"</p>
            <p class="cross-chain-notice">"后续交易仍需核对当前可用余额、钱包占用、新报价、最低到账和全部费用，并重新获得实盘授权。原套利路径保持停止。"</p>
        </section>
    }
}

pub(super) fn controls(
    run: &OnchainCrossChainRun,
    data: OnchainCrossChainData,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let plan = run.accounting.as_ref().and_then(|a| a.disposition.as_ref());
    plan.filter(|plan| plan.blockers.is_empty()).map(|plan| {
        let run_id = run.run_id.clone();
        let updated = run.updated_at_ms;
        let rows = plan.remaining_assets.iter().enumerate().filter(|(_, row)| row.action != Action::ReviewWallet).map(|(index, row)| {
            let amount = RwSignal::new(row.change.amount_exact.clone());
            let id = run_id.clone();
            let request_id = run_id.clone();
            let request_matches = move || data.recovery_preview_request.with(|request| request.as_ref().is_some_and(|r|
                r.run_id == request_id && r.expected_run_updated_at_ms == updated && r.asset_index == index
                    && r.amount_exact.trim() == amount.get().trim()));
            let matches_result = request_matches.clone();
            view! {
                <div class="cross-chain-recovery-quote">
                    <label><span>{format!("{} · {}", row.change.asset.symbol, chain_label(&row.change.chain))}</span>
                        <input type="text" inputmode="decimal" aria-label="本次处置数量" prop:value=move || amount.get()
                            on:input=move |ev| amount.set(event_target_value(&ev)) />
                    </label>
                    <button type="button" class="row-action"
                        disabled=move || data.recovery_previewing.get() || data.submitting.get() || data.rechecking.get() || data.authorizing.get() || data.building.get()
                        on:click=move |_| data.preview_recovery.run(OnchainCrossChainRecoveryPreviewRequest {
                            run_id: id.clone(), expected_run_updated_at_ms: updated, asset_index: index, amount_exact: amount.get_untracked(),
                        })>{move || if data.recovery_previewing.get() && request_matches() { "预检中…" } else { "核对余额与新报价" }}</button>
                    {move || matches_result().then(|| data.recovery_preview.get()).flatten().map(|result| match result {
                        Ok(preview) => preview_result(preview, clock).into_any(),
                        Err(problem) => view! { <p class="cross-chain-notice is-warning">{problem.message}</p> }.into_any(),
                    })}
                </div>
            }
        }).collect_view();
        view! { <section class="cross-chain-accounting cross-chain-recovery-previews" aria-label="资金处置预检">{rows}</section> }
    })
}

pub(super) fn preview_result(
    preview: OnchainCrossChainRecoveryPreview,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let deadline = preview.valid_until_ms;
    let quote_ready = preview.quote_ready;
    let has_quote = preview.route_id.is_some();
    let keep = preview.provider == "none" && preview.balance_amount_raw.is_some();
    let label = move || {
        if deadline.is_some_and(|until| clock.get() >= until) {
            "预检已过期"
        } else if keep {
            "余额已核对 · 无需交易"
        } else if quote_ready {
            "报价已核对 · 未锁定资金"
        } else {
            "预检未通过 / 无需交易"
        }
    };
    let input = &preview.input.asset;
    let target = &preview.target.asset;
    let balance = preview
        .balance_amount_raw
        .as_deref()
        .map(|raw| raw_amount_label(raw, input.decimals, &input.symbol))
        .unwrap_or_else(|| "未核实".into());
    let expected = preview
        .expected_output_amount_raw
        .as_deref()
        .map(|raw| raw_amount_label(raw, target.decimals, &target.symbol))
        .unwrap_or_else(|| "未取得".into());
    let minimum = preview
        .minimum_output_amount_raw
        .as_deref()
        .map(|raw| raw_amount_label(raw, target.decimals, &target.symbol))
        .unwrap_or_else(|| "未取得".into());
    let cost = |value: Option<f64>| {
        value
            .map(|value| {
                if value > 0.0 && value < 0.000001 {
                    "<$0.000001".into()
                } else {
                    format!("${value:.6}")
                }
            })
            .unwrap_or_else(|| "未知".into())
    };
    let blockers = preview
        .blockers
        .iter()
        .map(|problem| view! { <li>{problem.clone()}</li> })
        .collect_view();
    view! {
        <div class="cross-chain-recovery-quote-result" role="status">
            <strong>{label}</strong>
            <dl class="cross-chain-leg-amounts">
                <div><dt>"当前链上余额"</dt><dd>{balance}</dd></div>
                <div><dt>"本次询价金额"</dt><dd>{format!("{} {}", preview.input.amount_exact, input.symbol)}</dd></div>
                <div><dt>"预计到账"</dt><dd>{expected}</dd></div>
                <div><dt>"最低到账"</dt><dd>{minimum}</dd></div>
                <div><dt>"报价费用估值"</dt><dd>{cost(preview.fee_usd)}</dd></div>
                <div><dt>"Gas 估值"</dt><dd>{cost(preview.gas_usd)}</dd></div>
            </dl>
            {has_quote.then(|| view! { <p>{format!("LI.FI → {} · {} · 预计耗时 {}", chain_label(&preview.target.chain), target.symbol,
                preview.estimated_duration_seconds.map(|seconds| format!("{seconds}s")).unwrap_or_else(|| "未知".into()))}</p> })}
            <ul class=if keep { "cross-chain-notice" } else { "cross-chain-notice is-warning" }>{blockers}</ul>
            <p class="cross-chain-accounting-scope">"最低到账已含报价滑点；费用估值单列，不再从到账数量重复扣减。资产尚未预留，未签名、未提交。"</p>
        </div>
    }
}
