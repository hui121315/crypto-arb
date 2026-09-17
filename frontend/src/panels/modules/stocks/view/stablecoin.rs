use super::*;
mod native_topup;

pub(super) fn panel(asset: String, data: StockData) -> impl IntoView {
    body(asset, data.preflight, data.keyed, data.pending, data.clock)
}

pub(super) fn recovery_panel(data: StockData) -> impl IntoView {
    let d = data.preflight.stablecoin;
    let visible = Memo::new(move |_| !d.plans.get().is_empty() || d.store_problem.get().is_some());
    move || {
        visible.get().then(||view! {
        <section class="stock-section stock-stablecoin" aria-label="已恢复稳定币兑换">
            <header><h3>"稳定币兑换记录"</h3></header>
            {move ||d.problem.get().map(|p|view!{<p class="stock-rfq-note" role="alert">{p}</p>})}
            {move ||d.store_problem.get().map(|p|view!{<p class="stock-rfq-note" role="alert">{p}</p>})}
            <For each=move ||d.plans.get() key=|p|(p.plan_id.clone(),p.revision)
                children=move |p|plan_row(p,data.preflight,data.preflight.pending,data.pending,data.clock)/>
        </section>
    })
    }
}

fn body(
    asset: String,
    preflight: super::super::data::PreflightData,
    keyed: RwSignal<bool>,
    pending: RwSignal<bool>,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let d = preflight.stablecoin;
    let selected = asset.clone();
    let visible = Memo::new(move |_| {
        d.preview.get().filter(|p| {
            p.request.asset == selected
                && p.request.wallet_address == preflight.wallet.get().trim()
                && p.request.input_usdt == d.input.get().trim()
                && p.request.target_usdc == d.target.get().trim()
                && p.request.keyed == keyed.get()
        })
    });
    view! {<div class="stock-stablecoin" aria-label="链上稳定币补库试算">
        <header><h4>"USDT 补充 USDC"</h4><span>"Solana 钱包"</span></header>
        <form class="stock-stablecoin-form" on:submit=move |e|{e.prevent_default();d.read.run((asset.clone(),keyed.get_untracked()));}>
            <label><span>"投入 / USDT"</span><input type="text" inputmode="decimal" autocomplete="off" placeholder="最多 6 位小数" aria-label="兑换投入 USDT"
                value=move ||d.input.get() prop:value=move ||d.input.get() on:input=move |e|d.input.set(event_target_value(&e))/></label>
            <label><span>"希望补入 / USDC"</span><input type="text" inputmode="decimal" autocomplete="off" placeholder="本次缺口" aria-label="希望补入 USDC"
                value=move ||d.target.get() prop:value=move ||d.target.get() on:input=move |e|d.target.set(event_target_value(&e))/></label>
            <button type="submit" class="row-action" disabled=move ||preflight.pending.get() ||pending.get()
                ||preflight.wallet.get().trim().is_empty() ||d.input.get().trim().is_empty() ||d.target.get().trim().is_empty()>
                {move ||if preflight.pending.get(){"处理中…"}else{"试算兑换"}}</button>
        </form>
        {move ||d.problem.get().map(|p|view!{<p class="stock-rfq-note" role="alert">{p}</p>})}
        {move ||d.store_problem.get().map(|p|view!{<p class="stock-rfq-note" role="alert">{p}</p>})}
        {move ||visible.get().map(|p|{
            let current=p.current(clock.get());
            let saved=d.plans.get().iter().any(|plan|plan.preview.request==p.request && plan.preview.checked_at_ms==p.checked_at_ms);
            let can_save=p.can_reserve(clock.get()) && !saved && d.store_problem.get().is_none();
            let for_save=p.clone();
            let network=p.cost.as_ref().and_then(|c|c.network_fee_lamports.clone());
            let required=p.cost.as_ref().and_then(|c|c.total_native_required_lamports(clock.get()).map(|n|n.to_string()).or_else(||c.wallet_required_lamports.clone()));
            let fee=p.quote.fee_bps.and_then(|v|stock_chain_quantity(&v.to_string(),2)).map(|v|format!("{v}%")).unwrap_or_else(||"未知".into());
            let fee_asset=match p.quote.fee_mint.as_deref(){Some(STOCK_SOLANA_USDT)=>"USDT",Some(shared_types::stocks::comparison::SOLANA_USDC)=>"USDC",_=>"币种未知"};
            view!{<div class="stock-stablecoin-result" data-current=current.to_string()>
                <p class="stock-quote-meta">{if current{"本次试算 · 未兑换"}else{"历史试算 · 请重新读取"}}</p>
                <dl class="stock-direction-values">
                    <div><dt>"最低到账 / USDC"</dt><dd>{p.minimum_usdc}</dd></div>
                    <div><dt>"含 SOL 预算后缺口 / USDC"</dt><dd>{p.shortfall_usdc}</dd></div>
                    <div><dt>"扣 SOL 补回预算后 / USDC"</dt><dd>{p.after_native_cost_usdc.unwrap_or_else(||"待核实".into())}</dd></div>
                    <div><dt>"钱包可用 / USDT"</dt><dd>{raw_amount(p.wallet.stock_raw,6)}</dd></div>
                    <div><dt>"网络费 / SOL"</dt><dd>{raw_amount(network,9)}</dd></div>
                    <div><dt>"周转备款 / SOL"</dt><dd>{raw_amount(required,9)}</dd></div>
                </dl>
                <details><summary>"兑换费用与限制"</summary>
                    <p class="stock-rfq-note">{format!("路由 {} · Provider 费率 {fee} / {fee_asset} · 使用最低到账，不再次扣同一笔路由费用",p.quote.router)}</p>
                    <ul>{p.blockers.into_iter().map(|b|view!{<li>{b}</li>}).collect_view()}</ul>
                </details>
                <button type="button" class="row-action stock-stablecoin-save" disabled=move ||!can_save ||preflight.pending.get() ||pending.get()
                    on:click=move |_|d.save.run(for_save.clone())>{if saved{"原报价已保存"}else{"保存兑换计划"}}</button>
            </div>}
        })}
        <For each=move ||d.plans.get() key=|p|(p.plan_id.clone(),p.revision)
            children=move |p|plan_row(p,preflight,preflight.pending,pending,clock)/>
        <p class="stock-rfq-note">"实盘兑换需要 Jupiter API Key 与原钱包签名配置。未提交的预留可取消；提交后只核对原交易。交易所兑换和转入是独立资金操作。"</p>
    </div>}
}

fn plan_row(
    p: StockStablecoinPlan,
    preflight: super::super::data::PreflightData,
    pending: RwSignal<bool>,
    other_pending: RwSignal<bool>,
    clock: RwSignal<i64>,
) -> impl IntoView {
    let data = preflight.stablecoin;
    let frozen = p.clone();
    let phase = Memo::new(move |_| frozen.phase_at(clock.get()));
    let confirmed = RwSignal::new(false);
    let confirmation = StockStablecoinSubmitRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
        confirm_live: true,
    };
    let recheck = StockPlanCancelRequest {
        plan_id: p.plan_id.clone(),
    };
    let cooldown = p.submission.as_ref().map_or(0, |s| s.next_recheck_at_ms);
    let can_check = p.submission.as_ref().is_some_and(|s| s.receipt.is_none());
    let receipt = p.submission.as_ref().and_then(|s| s.receipt.clone());
    let problem = p.submission.as_ref().and_then(|s| s.problem.clone());
    let original_signature = p.submission.as_ref().map(|s| {
        s.transaction_id
            .as_ref()
            .unwrap_or(&s.wallet_signature)
            .clone()
    });
    let request = StockPlanRevisionRequest {
        plan_id: p.plan_id.clone(),
        revision: p.revision,
    };
    let topup_plan = p.clone();
    view! {<section class="stock-stablecoin-plan" aria-label="已保存兑换计划">
        <header><strong>{format!("{} USDT → 至少 {} USDC",p.request.conversion.input_usdt,p.preview.minimum_usdc)}</strong>
            <span>{move ||match phase.get() {
                StockStablecoinPlanPhase::Reserved=>"已预留 · 未兑换",
                StockStablecoinPlanPhase::SubmissionUnknown=>"已提交 · 待核对到账",
                StockStablecoinPlanPhase::Completed=>"兑换已到账",
                StockStablecoinPlanPhase::Failed=>"链上失败 · 已核对回滚",
                StockStablecoinPlanPhase::NeedsReview=>"收支不符 · 保留占用",
                StockStablecoinPlanPhase::Cancelled=>"已取消 · 未兑换",
                StockStablecoinPlanPhase::Expired=>"报价已过期 · 未兑换"}}</span></header>
        <p class="stock-rfq-note">{format!("{} · Solana · {}",p.request.conversion.asset,p.request.conversion.wallet_address)}</p>
        <details class="stock-stablecoin-record"><summary>"计划记录"</summary><p class="stock-rfq-note">{p.plan_id}</p><p class="stock-rfq-note">{format!("版本 {} · 到期后不会自动换报价或重新预留",p.revision)}</p>
            {original_signature.map(|s|view!{<p class="stock-rfq-note">{format!("原交易签名：{s}")}</p>})}</details>
        {problem.map(|s|view!{<p class="stock-rfq-note" role="status">{s}</p>})}
        {receipt.map(|r|receipt_row(r,p.request.conversion.target_usdc.clone()))}
        {(topup_plan.phase==StockStablecoinPlanPhase::Completed).then(||native_topup::panel(topup_plan,preflight,pending,other_pending,clock))}
        {move ||(phase.get()==StockStablecoinPlanPhase::Reserved).then(||view!{
            <div class="stock-stablecoin-actions">
                <label><input type="checkbox" aria-label="确认本次稳定币实盘兑换"
                    prop:checked=move ||confirmed.get() on:change=move |e|confirmed.set(event_target_checked(&e))
                    disabled=move ||pending.get() ||other_pending.get() ||data.store_problem.get().is_some()/><span>"确认本次实盘兑换"</span></label>
                <button type="button" class="row-action stock-stablecoin-submit"
                    disabled=move ||!confirmed.get() ||pending.get() ||other_pending.get() ||data.store_problem.get().is_some()
                    on:click={let confirmation=confirmation.clone();move |_|{
                        if confirmed.get_untracked() && phase.get_untracked()==StockStablecoinPlanPhase::Reserved {
                            confirmed.set(false);data.submit.run(confirmation.clone());
                        }
                    }}>"提交兑换"</button>
                <button type="button" class="row-action stock-stablecoin-cancel" disabled=move ||pending.get() ||other_pending.get()
                    on:click={let request=request.clone();move |_|data.cancel.run(request.clone())}>"取消预留"</button>
            </div>
        })}
        {can_check.then(||view!{<button type="button" class="row-action stock-stablecoin-recheck"
            disabled=move ||pending.get() ||other_pending.get() ||clock.get()<cooldown
            on:click=move |_|data.recheck.run(recheck.clone())>{move ||if clock.get()<cooldown{"稍后核对"}else{"核对原交易"}}</button>})}
    </section>}
}

fn receipt_row(r: StockChainReceipt, target: String) -> impl IntoView {
    let input = stablecoin_change(&r, STOCK_SOLANA_USDT)
        .and_then(|n| n.checked_neg())
        .map(|n| n.to_string());
    let output =
        stablecoin_change(&r, shared_types::stocks::comparison::SOLANA_USDC).map(|n| n.to_string());
    view! {<div class="stock-stablecoin-receipt" aria-label="稳定币实际收支">
        <dl class="stock-direction-values">
            <div><dt>"实际投入 / USDT"</dt><dd>{raw_amount(input,6)}</dd></div>
            <div><dt>"实际到账 / USDC"</dt><dd>{raw_amount(output,6)}</dd></div>
            <div><dt>"原补入目标 / USDC"</dt><dd>{target}</dd></div>
            <div><dt>"实际网络费 / SOL"</dt><dd>{raw_amount(Some(r.network_fee_lamports),9)}</dd></div>
            <div><dt>"钱包变化 / SOL"</dt><dd>{raw_amount(Some(r.wallet_native_change_lamports),9)}</dd></div>
            <div><dt>"最终确认 Slot"</dt><dd>{r.slot}</dd></div>
        </dl>
        <p class="stock-rfq-note">"SOL 变化已包含钱包承担的费用，不重复扣减；USDC 到账不代表完成 SOL 补回。"</p>
        <ul>{r.problems.into_iter().map(|p|view!{<li>{p}</li>}).collect_view()}</ul>
    </div>}
}

fn raw_amount(raw: Option<String>, decimals: u8) -> String {
    raw.and_then(|s| stock_chain_quantity(&s, decimals))
        .unwrap_or_else(|| "未知".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stock_stablecoin_ui_keeps_decimals_expiry_and_wallet_changes_visible() {
        Owner::new().with(|| {
            let mut draft=super::super::super::data::PreflightData::fixture();
            let saved=RwSignal::new(Vec::<StockStablecoinPlan>::new());
            draft.stablecoin.plans=Memo::new(move |_|saved.get());
            let d=draft.stablecoin;
            let keyed=RwSignal::new(false);let pending=RwSignal::new(false);let clock=RwSignal::new(1200);
            let render=||body("MU.US".into(),draft,keyed,pending,clock).to_html();
            assert!(render().contains("试算兑换"));assert!(render().contains("inputmode=\"decimal\""));
            assert!(render().contains("disabled"));
            let request=StockStablecoinRequest {asset:"MU.US".into(),wallet_address:"fixture".into(),input_usdt:"10.123456".into(),target_usdc:"10".into(),keyed:false};
            let wallet=StockWalletEvidence {owner:request.wallet_address.clone(),mint:STOCK_SOLANA_USDT.into(),stock_raw:None,
                usdc_raw:Some("0".into()),sol_lamports:None,checked_at_ms:1000,problems:vec![]};
            let quote=StockDexQuote {input_mint:STOCK_SOLANA_USDT.into(),output_mint:shared_types::stocks::comparison::SOLANA_USDC.into(),input_raw:"10123456".into(),
                output_raw:"9900000".into(),minimum_output_raw:"9800000".into(),router:"fixture".into(),fee_bps:None,fee_mint:None,requested_at_ms:1000,received_at_ms:1100,expires_at_ms:None};
            let p=shared_types::stocks::stablecoin_preview(request,wallet,quote,None,vec![],1200).unwrap();
            draft.wallet.set(p.request.wallet_address.clone());d.input.set(p.request.input_usdt.clone());d.target.set(p.request.target_usdc.clone());d.preview.set(Some(p.clone()));
            let html=render();
            for text in ["10.123456","最低到账 / USDC","9.8","0.2","待核实","本次试算 · 未兑换"] {assert!(html.contains(text),"missing {text}");}
            d.input.set("11".into());assert!(!render().contains("最低到账 / USDC"));d.input.set("10.123456".into());
            draft.wallet.set("another".into());assert!(!render().contains("最低到账 / USDC"));draft.wallet.set("fixture".into());
            clock.set(p.valid_until_ms);assert!(render().contains("历史试算"));
            keyed.set(true);assert!(!render().contains("最低到账 / USDC"));keyed.set(false);
            if let Ok(path)=std::env::var("STOCK_STABLECOIN_CAPTURE_PATH") {
                let mut p:StockStablecoinPreview=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                if let Ok(path)=std::env::var("STOCK_STABLECOIN_PLAN_CAPTURE_PATH") {
                    let s:StockMarketSnapshot=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                    let plan=s.stablecoin_plans[0].clone();
                    let html=plan_row(plan.clone(),draft,pending,pending,RwSignal::new(plan.preview.checked_at_ms)).to_html();
                    assert!(html.contains("取消预留"));assert!(html.contains("已预留 · 未兑换"));
                    assert!(html.contains("确认本次实盘兑换"));assert!(html.contains("提交兑换"));
                    let html=plan_row(plan.clone(),draft,pending,pending,RwSignal::new(plan.preview.valid_until_ms)).to_html();
                    assert!(html.contains("报价已过期"));assert!(!html.contains("取消预留"));
                    let mut cancelled=plan.clone();cancelled.phase=StockStablecoinPlanPhase::Cancelled;cancelled.revision=2;
                    assert!(!plan_row(cancelled,draft,pending,pending,RwSignal::new(plan.preview.checked_at_ms)).to_html().contains("取消预留"));
                    p=plan.preview.clone();saved.set(vec![plan]);
                }
                clock.set(p.checked_at_ms);draft.wallet.set(p.request.wallet_address.clone());d.input.set(p.request.input_usdt.clone());d.target.set(p.request.target_usdc.clone());d.preview.set(Some(p));
                if let Ok(path)=std::env::var("STOCK_STABLECOIN_RENDER_PATH") {
                    super::super::tests::write_stock_html(&path,&format!("<main class=\"stock-arbitrage-page stock-main\"><section class=\"stock-section\">{}</section></main>",render()));
                }
            }
        });
    }

    #[test]
    fn stock_stablecoin_receipt_ui_uses_actual_credit_and_never_offers_resubmission() {
        let capture: serde_json::Value = std::env::var("STOCK_STABLECOIN_EXECUTION_CAPTURE_PATH")
            .ok()
            .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
            .unwrap_or_else(receipt_fixture);
        Owner::new().with(||{
            let draft=super::super::super::data::PreflightData::fixture();
            let pending=RwSignal::new(false);
            for name in ["pending","completed"] {
                let snapshot:StockMarketSnapshot=serde_json::from_value(capture[name].clone()).unwrap();
                let plan=snapshot.stablecoin_plans[0].clone();
                let html=plan_row(plan.clone(),draft,pending,pending,RwSignal::new(plan.updated_at_ms+6000)).to_html();
                assert!(!html.contains("提交兑换"));assert!(!html.contains("取消预留"));
                if name=="pending" {
                    assert!(html.contains("核对原交易"));assert!(!html.contains("稳定币实际收支"));
                } else {
                    assert!(!html.contains("核对原交易"));
                    for text in ["兑换已到账","实际到账 / USDC","9.95","实际网络费 / SOL","0.000007","钱包变化 / SOL"] {
                        assert!(html.contains(text),"missing {text}");
                    }
                    let mut review=plan.clone();review.phase=StockStablecoinPlanPhase::NeedsReview;
                    let r=review.submission.as_mut().unwrap().receipt.as_mut().unwrap();
                    r.asset_changes.iter_mut().find(|a|a.mint==shared_types::stocks::comparison::SOLANA_USDC).unwrap().raw_change="1".into();
                    let html=plan_row(review,draft,pending,pending,RwSignal::new(plan.updated_at_ms+6000)).to_html();
                    assert!(html.contains("收支不符 · 保留占用"));assert!(html.contains("0.000001"));assert!(!html.contains("提交兑换"));
                }
                if let Ok(path)=std::env::var("STOCK_STABLECOIN_RENDER_PATH") {
                    let path=std::path::Path::new(&path).with_file_name(format!("stocks-market-stablecoin-{name}.html"));
                    super::super::tests::write_stock_html(path.to_str().unwrap(),&format!("<main class=\"stock-arbitrage-page stock-main\"><section class=\"stock-section stock-stablecoin\">{html}</section></main>"));
                }
            }
        });
    }

    fn receipt_fixture() -> serde_json::Value {
        let request = StockStablecoinRequest {
            asset: "MU.US".into(),
            wallet_address: "local-wallet".into(),
            input_usdt: "10".into(),
            target_usdc: "9.5".into(),
            keyed: false,
        };
        let wallet = StockWalletEvidence {
            owner: request.wallet_address.clone(),
            mint: STOCK_SOLANA_USDT.into(),
            stock_raw: Some("10000000".into()),
            usdc_raw: Some("0".into()),
            sol_lamports: Some("10000000".into()),
            checked_at_ms: 1000,
            problems: vec![],
        };
        let quote = StockDexQuote {
            input_mint: STOCK_SOLANA_USDT.into(),
            output_mint: shared_types::stocks::comparison::SOLANA_USDC.into(),
            input_raw: "10000000".into(),
            output_raw: "9950000".into(),
            minimum_output_raw: "9900000".into(),
            router: "local-fixture".into(),
            fee_bps: Some(10),
            fee_mint: Some(STOCK_SOLANA_USDT.into()),
            requested_at_ms: 1000,
            received_at_ms: 1000,
            expires_at_ms: None,
        };
        let preview =
            stablecoin_preview(request.clone(), wallet, quote, None, vec![], 1000).unwrap();
        let plan = StockStablecoinPlan {
            plan_id: "local-stablecoin-ui".into(),
            request: StockStablecoinPlanRequest {
                request_id: "local-stablecoin-ui".into(),
                conversion: request,
                preview_at_ms: 1000,
                transaction_fingerprint: "local-fixture".into(),
            },
            preview,
            phase: StockStablecoinPlanPhase::SubmissionUnknown,
            revision: 2,
            updated_at_ms: 1200,
            submission: Some(StockChainSubmission {
                submitted_at_ms: 1100,
                wallet_signature: "local-signature".into(),
                transaction_id: Some("local-signature".into()),
                provider_transaction_id: None,
                provider_acknowledged: true,
                receipt: None,
                recheck_attempts: 0,
                next_recheck_at_ms: 1100,
                search_before: None,
                problem: None,
            }),
            native_topups: vec![],
        };
        let mut completed = plan.clone();
        completed.phase = StockStablecoinPlanPhase::Completed;
        completed.revision = 3;
        completed.submission.as_mut().unwrap().receipt = Some(StockChainReceipt {
            transaction_id: "local-signature".into(),
            slot: 13,
            succeeded: true,
            fee_payer: "local-sponsor".into(),
            network_fee_lamports: "7000".into(),
            wallet_native_change_lamports: "0".into(),
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
        });
        serde_json::json!({"pending":StockMarketSnapshot {stablecoin_plans:vec![plan],..Default::default()},
            "completed":StockMarketSnapshot {stablecoin_plans:vec![completed],..Default::default()}})
    }
}
