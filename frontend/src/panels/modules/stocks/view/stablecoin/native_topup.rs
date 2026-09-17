use super::*;

pub(super) fn panel(
    p: StockStablecoinPlan,
    preflight: super::super::super::data::PreflightData,
    pending: RwSignal<bool>,
    other_pending: RwSignal<bool>,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let data = preflight.stablecoin;
    let report = p.native_accounting();
    let gap = report
        .as_ref()
        .ok()
        .map(|r| (-r.net_native_lamports).max(0));
    let held = p.clone();
    let available = Memo::new(move |_| {
        gap.is_some_and(|n| n > 0) && !held.holds_funds(clock.get()) && held.native_topups.len() < 8
    });
    let request = StockPlanRevisionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
    };
    let target = p
        .request
        .conversion
        .amounts_raw()
        .ok()
        .map(|(_, n)| i128::from(n));
    let status = match &report {
        Ok(r) if r.net_native_lamports >= 0 && target.is_some_and(|n| r.retained_usdc_raw >= n) => {
            "成本已核清 · SOL 无缺口"
        }
        Ok(_) => "USDC 已到账 · SOL 待补回",
        Err(_) => "补回待核对 · 保留占用",
    };
    view! {<div class="stock-stablecoin-result" aria-label="兑换后的 SOL 补回">
        <header><h4>"兑换后的 SOL 补回"</h4><span>{status}</span></header>
        <dl class="stock-direction-values">
            <div><dt>"当前可留 / USDC"</dt><dd>{raw_amount(report.as_ref().ok().map(|r|r.retained_usdc_raw.to_string()),6)}</dd></div>
            <div><dt>"已花补回成本 / USDC"</dt><dd>{raw_amount(report.as_ref().ok().map(|r|r.spent_usdc_raw.to_string()),6)}</dd></div>
            <div><dt>"尚需补回 / SOL"</dt><dd>{raw_amount(gap.map(|n|n.to_string()),9)}</dd></div>
        </dl>
        {report.err().map(|s|view!{<p class="stock-rfq-note" role="status">{s}</p>})}
        {move ||available.get().then({let request=request.clone();move ||view!{
            <button type="button" class="row-action" disabled=move ||pending.get() ||other_pending.get() ||data.store_problem.get().is_some()
                on:click={let request=request.clone();move |_|data.prepare_topup.run(request.clone())}>"试算并预留 SOL 补回"</button>
        }})}
        {p.native_topups.into_iter().enumerate().map(|(index,r)|row(p.plan_id.clone(),p.revision,index,r,preflight,pending,other_pending,clock)).collect_view()}
    </div>}
}

fn row(
    id: String,
    revision: u64,
    index: usize,
    row: StockStablecoinNativeTopup,
    preflight: super::super::super::data::PreflightData,
    pending: RwSignal<bool>,
    other_pending: RwSignal<bool>,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let data = preflight.stablecoin;
    let fixed = row.clone();
    let current = Memo::new(move |_| fixed.current(clock.get()));
    let confirmed = RwSignal::new(false);
    let submit = StockStablecoinTopupSubmitRequest {
        plan_id: id.clone(),
        revision,
        index,
        confirm_live: true,
    };
    let cancel = StockTopupRecheckRequest {
        plan_id: id.clone(),
        index,
    };
    let check = StockTopupRecheckRequest { plan_id: id, index };
    let s = row.terms.submission;
    let cooldown = s.as_ref().map_or(0, |s| s.next_recheck_at_ms);
    let needs_check = s.as_ref().is_some_and(|s| s.receipt.is_none());
    let receipt = s.as_ref().and_then(|s| s.receipt.clone());
    let status = if row.cancelled_at_ms.is_some() {
        "已取消 · 未发送"
    } else if let Some(r) = &receipt {
        if r.succeeded && r.within_plan && r.problems.is_empty() {
            "已核对回执"
        } else if !r.succeeded {
            "失败 · 保留实际扣费"
        } else {
            "收支异常 · 保留占用"
        }
    } else if s.is_some() {
        "已提交 · 只核对原交易"
    } else {
        "已保存 · 未发送"
    };
    view! {<div class="stock-stablecoin-plan" aria-label="SOL 补回记录">
        <header><strong>{format!("SOL 补回 #{}",index+1)}</strong><span>{status}</span></header>
        <dl class="stock-direction-values">
            <div><dt>"投入 / USDC"</dt><dd>{raw_amount(Some(row.terms.valuation.quote.input_raw),6)}</dd></div>
            <div><dt>"补回目标 / SOL"</dt><dd>{raw_amount(Some(row.terms.valuation.native_lamports),9)}</dd></div>
            <div><dt>"扣费后至少 / SOL"</dt><dd>{raw_amount(row.terms.valuation.replenishment.map(|p|p.minimum_credit_lamports),9)}</dd></div>
        </dl>
        {s.as_ref().and_then(|s|s.problem.clone()).map(|s|view!{<p class="stock-rfq-note">{s}</p>})}
        {receipt.map(|r|view!{<dl class="stock-direction-values" aria-label="SOL 补回实际收支">
            <div><dt>"实际 USDC 变化"</dt><dd>{raw_amount(stablecoin_change(&r,shared_types::stocks::comparison::SOLANA_USDC).map(|n|n.to_string()),6)}</dd></div>
            <div><dt>"实际 SOL 变化"</dt><dd>{raw_amount(Some(r.wallet_native_change_lamports),9)}</dd></div>
            <div><dt>"实际网络费 / SOL"</dt><dd>{raw_amount(Some(r.network_fee_lamports),9)}</dd></div>
        </dl>})}
        {s.map(|s|view!{<details><summary>"原补回交易"</summary><p class="stock-rfq-note">{s.transaction_id.unwrap_or(s.wallet_signature)}</p></details>})}
        {move ||current.get().then(||view!{<div class="stock-stablecoin-actions">
            <label><input type="checkbox" aria-label="确认本次 SOL 补回" prop:checked=move ||confirmed.get()
                on:change=move |e|confirmed.set(event_target_checked(&e)) disabled=move ||pending.get() ||other_pending.get()/><span>"确认本次 SOL 补回"</span></label>
            <button type="button" class="row-action" disabled=move ||!confirmed.get() ||pending.get() ||other_pending.get() ||data.store_problem.get().is_some()
                on:click={let submit=submit.clone();move |_|{if confirmed.get_untracked() && current.get_untracked(){confirmed.set(false);data.submit_topup.run(submit.clone());}}}>"提交 SOL 补回"</button>
            <button type="button" class="row-action" disabled=move ||pending.get() ||other_pending.get()
                on:click={let cancel=cancel.clone();move |_|data.cancel_topup.run(cancel.clone())}>"取消补回预留"</button>
        </div>})}
        {needs_check.then(||view!{<button type="button" class="row-action" disabled=move ||pending.get() ||other_pending.get() ||clock.get()<cooldown
            on:click=move |_|data.recheck_topup.run(check.clone())>"核对原补回交易"</button>})}
        {move ||(!current.get() && status=="已保存 · 未发送").then(||view!{<p class="stock-rfq-note">"补回报价已过期 · 未发送"</p>})}
    </div>}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_stablecoin_topup_ui_keeps_confirmation_recovery_and_actual_costs_separate() {
        let snapshots: serde_json::Value = std::env::var("STOCK_STABLECOIN_TOPUP_CAPTURE_PATH")
            .ok()
            .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
            .unwrap_or_else(fixture);
        Owner::new().with(||{
            let draft=super::super::super::super::data::PreflightData::fixture();
            let pending=RwSignal::new(false);
            for state in ["ready","pending","completed"] {
                let snapshot:StockMarketSnapshot=serde_json::from_value(snapshots[state].clone()).unwrap();
                let p=snapshot.stablecoin_plans[0].clone();let clock=RwSignal::new(p.updated_at_ms);
                let html=super::super::plan_row(p.clone(),draft,pending,pending,clock).to_html();
                assert!(!html.contains("提交兑换"));
                if state=="ready" {
                    assert!(html.contains("确认本次 SOL 补回"));assert!(html.contains("提交 SOL 补回"));assert!(html.contains("取消补回预留"));
                    assert!(!html.contains(" checked"));
                    let last=p.native_topups.last().unwrap();clock.set(last.terms.valuation.replenishment.as_ref().unwrap().valid_until_ms);
                    let expired=super::super::plan_row(p.clone(),draft,pending,pending,clock).to_html();
                    assert!(expired.contains("补回报价已过期"));assert!(!expired.contains("提交 SOL 补回"));assert!(expired.contains("试算并预留 SOL 补回"));
                } else if state=="pending" {
                    assert!(html.contains("核对原补回交易"));assert!(!html.contains("提交 SOL 补回"));assert!(!html.contains("成本已核清"));
                } else {
                    for text in ["成本已核清","9.94","0.01","SOL 补回实际收支"] {assert!(html.contains(text),"{text}");}
                    assert!(!html.contains("核对原补回交易"));assert!(!html.contains("提交 SOL 补回"));
                }
                if let Ok(path)=std::env::var("STOCK_STABLECOIN_RENDER_PATH") {
                    let path=std::path::Path::new(&path).with_file_name(format!("stocks-market-stablecoin-topup-{state}.html"));
                    super::super::super::tests::write_stock_html(path.to_str().unwrap(),&format!("<main class=\"stock-arbitrage-page stock-main\"><section class=\"stock-section stock-stablecoin\">{html}</section></main>"));
                }
            }
        });
    }

    fn fixture() -> serde_json::Value {
        let mut preview: StockStablecoinPreview = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/api/src/services/backpack_stocks/stablecoin_store/fixtures/preview.json"
        )))
        .unwrap();
        preview.request.target_usdc = "9.5".into();
        preview.cost.as_mut().unwrap().wallet_debit_lamports = Some("7000".into());
        let mut main = StockStablecoinPlan {
            plan_id: "local-topup-view".into(),
            request: StockStablecoinPlanRequest {
                request_id: "local-topup-view".into(),
                conversion: preview.request.clone(),
                preview_at_ms: preview.checked_at_ms,
                transaction_fingerprint: preview
                    .cost
                    .as_ref()
                    .unwrap()
                    .transaction_fingerprint
                    .clone(),
            },
            preview,
            phase: StockStablecoinPlanPhase::Completed,
            revision: 3,
            updated_at_ms: 2000,
            submission: None,
            native_topups: vec![],
        };
        let owner = main.request.conversion.wallet_address.clone();
        let receipt = StockChainReceipt {
            transaction_id: "local-main".into(),
            slot: 13,
            succeeded: true,
            fee_payer: owner.clone(),
            network_fee_lamports: "7000".into(),
            wallet_native_change_lamports: "-7000".into(),
            asset_changes: vec![
                StockChainAssetChange {
                    mint: STOCK_SOLANA_USDT.into(),
                    decimals: 6,
                    raw_change: "-10000000".into(),
                },
                StockChainAssetChange {
                    mint: shared_types::stocks::comparison::SOLANA_USDC.into(),
                    decimals: 6,
                    raw_change: "9950000".into(),
                },
            ],
            within_plan: true,
            problems: vec![],
        };
        let submitted = StockChainSubmission {
            submitted_at_ms: 1100,
            wallet_signature: "local-main".into(),
            transaction_id: Some("local-main".into()),
            provider_transaction_id: None,
            provider_acknowledged: true,
            receipt: Some(receipt),
            recheck_attempts: 1,
            next_recheck_at_ms: 1200,
            search_before: None,
            problem: None,
        };
        main.submission = Some(submitted.clone());
        let mut quote = main.preview.quote.clone();
        quote.input_mint = shared_types::stocks::comparison::SOLANA_USDC.into();
        quote.output_mint = STOCK_WRAPPED_SOL.into();
        quote.input_raw = "10000".into();
        quote.output_raw = "14000".into();
        quote.minimum_output_raw = "14000".into();
        quote.requested_at_ms = 2000;
        quote.received_at_ms = 2000;
        quote.expires_at_ms = None;
        let terms = StockNativeTopup {
            source_revision: 3,
            prepared_at_ms: 2000,
            wallet: main.preview.wallet.clone(),
            submission: None,
            valuation: StockNativeValuation {
                native_lamports: "7000".into(),
                quote,
                replenishment: Some(StockNativeReplenishment {
                    wallet_address: owner.clone(),
                    transaction: main
                        .preview
                        .cost
                        .as_ref()
                        .unwrap()
                        .transaction
                        .clone()
                        .unwrap(),
                    transaction_fingerprint: "local-topup".into(),
                    network_fee_lamports: "7000".into(),
                    wallet_outflow_lamports: "7000".into(),
                    wallet_required_lamports: "897880".into(),
                    minimum_credit_lamports: "7000".into(),
                    simulation_slot: 13,
                    checked_at_ms: 2000,
                    valid_until_ms: 7000,
                }),
            },
        };
        main.native_topups.push(StockStablecoinNativeTopup {
            terms,
            cancelled_at_ms: None,
        });
        main.revision = 4;
        let mut pending = main.clone();
        pending.revision = 5;
        let mut s = submitted.clone();
        s.receipt = None;
        s.wallet_signature = "local-topup".into();
        s.transaction_id = None;
        pending.native_topups[0].terms.submission = Some(s);
        let mut completed = pending.clone();
        completed.revision = 6;
        let mut r = submitted.receipt.unwrap();
        r.transaction_id = "local-topup".into();
        r.wallet_native_change_lamports = "7000".into();
        r.asset_changes = vec![
            StockChainAssetChange {
                mint: shared_types::stocks::comparison::SOLANA_USDC.into(),
                decimals: 6,
                raw_change: "-10000".into(),
            },
            StockChainAssetChange {
                mint: STOCK_WRAPPED_SOL.into(),
                decimals: 9,
                raw_change: "0".into(),
            },
        ];
        completed.native_topups[0]
            .terms
            .submission
            .as_mut()
            .unwrap()
            .receipt = Some(r);
        serde_json::json!({"ready":StockMarketSnapshot {stablecoin_plans:vec![main],..Default::default()},
            "pending":StockMarketSnapshot {stablecoin_plans:vec![pending],..Default::default()},"completed":StockMarketSnapshot {stablecoin_plans:vec![completed],..Default::default()}})
    }
}
