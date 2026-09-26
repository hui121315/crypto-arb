use super::{replenishment_cost_selection::change_selection, OnchainData, PreviewContext};
use leptos::prelude::*;
use shared_types::{
    OnchainComparisonConfig, OnchainComparisonDirection, OnchainExecutionApprovalCost,
    OnchainTokenApprovalRunStatus as Status, OnchainTokenApprovalSubmitResponse as Run,
};

pub(super) fn selection(
    data: OnchainData,
    config: Memo<OnchainComparisonConfig>,
    direction: PreviewContext,
) -> impl IntoView {
    let selected = data.execution.selected_approvals;
    let locked = Signal::derive(move || {
        data.execution.building_execution.get() || data.execution.submitting_execution.get()
    });
    let invalidate = Callback::new(move |()| data.execution.execution_build.set(None));
    selection_for(data, selected, locked, invalidate, Callback::new(move |run: Run| config.with(|config| matches_market(&run, config, direction.get()))))
}

pub(super) fn selection_for(data: OnchainData, selected: RwSignal<Vec<String>>, locked: Signal<bool>, invalidate: Callback<()>, market: Callback<Run, bool>) -> impl IntoView {
    view! {
        <details class="onchain-ticket-evidence onchain-cost-selection">
            <summary on:click=move |_| data.execution.refresh_approval_history.run(())>
                <span>"授权费用"</span><strong>{move || {
                    let count = selected.get().len();
                    if count == 0 { "未归集".to_owned() } else {format!("已选 {count} 笔")}
                }}</strong>
            </summary>
            {move || {
                let (mut rows, mut owners) = match data.execution.approval_history.get() {
                    Some(Ok(history)) => (history.rows, history.cost_owners),
                    Some(Err(problem)) => return view! { <small class="is-warning">{format!("授权记录读取失败：{problem}")}</small> }.into_any(),
                    None => return view! { <small>"正在读取授权记录"</small> }.into_any(),
                };
                if let Some(Ok(current)) = data.execution.approval_submit.get() {
                    if !rows.iter().any(|r| r.run_id == current.run_id && r.updated_at_ms >= current.updated_at_ms) {
                        rows.retain(|r| r.run_id != current.run_id);
                        rows.insert(0,current);
                    }
                }
                if let Some(Ok(execution)) = data.execution.execution_submit.get() {
                    for cost in execution.approval_costs { owners.insert(cost.run.run_id, execution.run_id.clone()); }
                }
                rows.retain(|r| selected.get().contains(&r.run_id) || market.run(r.clone()));
                let mut choices = rows.into_iter().map(|run| {
                    let owner = owners.get(&run.run_id);
                    let usable = eligible(&run) && owner.is_none() && market.run(run.clone());
                    let note = owner.map(|id| format!("已归入执行 {id}")).unwrap_or_else(|| if usable {"全额计入本次".into()} else {"费用或归属待核对".into()});
                    (run.run_id.clone(), label(&run), note, usable)
                }).collect::<Vec<_>>();
                for id in selected.get() {
                    if !choices.iter().any(|r| r.0 == id) {
                        choices.push((id, "历史授权记录未读到".into(), "请取消选择后重新读取".into(), false));
                    }
                }
                if choices.is_empty() { return view! { <small>"当前钱包与代币暂无授权费用记录"</small> }.into_any(); }
                choices.into_iter().map(|(id,label,note,usable)| choice(id,label,note,usable,selected,locked,invalidate)).collect_view().into_any()
            }}
        </details>
    }
}

fn matches_market(
    run: &Run,
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
) -> bool {
    let (address, decimals) = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            (&config.quote_mint, config.quote_decimals)
        }
        OnchainComparisonDirection::BuyCexSellOnchain => (&config.base_mint, config.base_decimals),
    };
    run.fee_receipts.first().is_some_and(|r| {
        r.basis.chain.eq_ignore_ascii_case(&config.chain)
            && r.basis.wallet.eq_ignore_ascii_case(&config.wallet_address)
            && r.basis
                .assets
                .first()
                .is_some_and(|a| a.address.eq_ignore_ascii_case(address) && a.decimals == decimals)
    })
}

fn eligible(run: &Run) -> bool {
    matches!(run.status, Status::Completed | Status::Failed)
        && !run.transaction_ids.is_empty()
        && run.transaction_ids.len() == run.fee_receipts.len()
        && !run.receipt_check_pending()
        && run.fee_receipts.iter().all(|r| {
            r.asset_changes_raw == [Some("0".into())]
                && r.additional_native_change_raw.as_deref() == Some("0")
                && r.status != shared_types::OnchainChainSettlementStatus::Pending
                && r.network_cost
                    .as_ref()
                    .is_some_and(|c| c.total_fee_exact.is_some() && c.problem.is_none())
        })
}

fn label(run: &Run) -> String {
    let fees = run
        .fee_receipts
        .iter()
        .map(|r| {
            r.network_cost
                .as_ref()
                .map(|c| {
                    format!(
                        "{} {}",
                        c.total_fee_exact.as_deref().unwrap_or("待核对"),
                        c.asset
                    )
                })
                .unwrap_or_else(|| "费用待核对".into())
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let token = run
        .fee_receipts
        .first()
        .and_then(|r| r.basis.assets.first())
        .map(|a| a.symbol.as_str())
        .unwrap_or("代币未知");
    let time = crate::panels::modules::timestamp::local_date_hm(run.updated_at_ms)
        .unwrap_or_else(|| "时间未知".into());
    let chain = run.fee_receipts.first().map(|r| r.basis.chain.as_str()).unwrap_or("链未知");
    format!(
        "{chain} · {token} 授权 · {fees} · {time}{}",
        if run.status == Status::Failed {
            " · 授权失败，费用已发生"
        } else {
            ""
        }
    )
}

fn choice(
    id: String,
    label: String,
    note: String,
    usable: bool,
    selected: RwSignal<Vec<String>>,
    locked: Signal<bool>,
    invalidate: Callback<()>,
) -> impl IntoView {
    let checked_id = id.clone();
    let disabled_id = id.clone();
    let title = format!("{id} · {note}");
    view! {
        <label class="onchain-cost-choice onchain-approval-cost-choice" title=title>
            <input type="checkbox"
                prop:checked=move || selected.get().contains(&checked_id)
                disabled=move || locked.get() || (!selected.get().contains(&disabled_id) && (!usable || selected.get().len() >= 8))
                on:change=move |event| {
                    let checked = event_target_checked(&event);
                    if !checked || usable { change_selection(selected, &id, checked, locked.get_untracked(), invalidate); }
                }/>
            <span>{label}<small>{note}</small></span>
        </label>
    }
}

pub(super) fn scope(costs: &[OnchainExecutionApprovalCost]) -> String {
    if costs.is_empty() {
        return "未归集".into();
    }
    let fees = costs
        .iter()
        .flat_map(|c| &c.run.fee_receipts)
        .filter_map(|r| r.network_cost.as_ref())
        .map(|c| {
            format!(
                "{} {}",
                c.total_fee_exact.as_deref().unwrap_or("待核对"),
                c.asset
            )
        })
        .collect::<Vec<_>>()
        .join(" + ");
    format!("{} 笔 · {fees} · 已从预计净收益扣除", costs.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> Run {
        serde_json::from_value(serde_json::json!({"runId":"approval-one","approvalId":"plan-one","status":"completed",
            "transactionIds":["0xabc"],"message":"confirmed","startedAtMs":1,"updatedAtMs":2,
            "feeReceipts":[{"basis":{"chain":"ethereum","wallet":"0xwallet","transactionId":"0xabc","requireSender":true,
                "assets":[{"symbol":"USDC","address":"0xtoken","decimals":6}]}, "status":"complete",
                "assetChangesRaw":["0"],"additionalNativeChangeRaw":"0","blockRef":"0xblock","observedAtMs":2,
                "networkCost":{"chain":"ethereum","transactionId":"0xabc","payer":"0xwallet","asset":"ETH",
                    "executionFeeExact":"0.000021","additionalFeeExact":"0","totalFeeExact":"0.000021",
                    "blockRef":"0xblock","source":"rpc","observedAtMs":2}}]})).unwrap()
    }

    #[test]
    fn approval_allocation_ui_exposes_failed_actual_fee_and_blocks_unknown_receipts() {
        let mut run = run();
        assert!(eligible(&run));
        run.status = Status::Failed;
        assert!(eligible(&run));
        assert!(label(&run).contains("授权失败，费用已发生"));
        assert!(label(&run).contains("0.000021 ETH"));
        run.fee_checks_exhausted = true;
        run.fee_receipts[0].network_cost = None;
        assert!(!eligible(&run));
        let mut config = OnchainComparisonConfig::default();
        config.chain = "ethereum".into();
        config.wallet_address = "0xwallet".into();
        config.quote_mint = "0xtoken".into();
        config.quote_decimals = 6;
        assert!(matches_market(
            &run,
            &config,
            OnchainComparisonDirection::BuyOnchainSellCex
        ));
        config.chain = "base".into();
        assert!(!matches_market(
            &run,
            &config,
            OnchainComparisonDirection::BuyOnchainSellCex
        ));
    }

    #[test]
    fn approval_allocation_ui_selection_keeps_fee_owner_visible_and_locked() {
        Owner::new().with(|| {
            let selected = RwSignal::new(Vec::new());
            let invalidated = RwSignal::new(false);
            let invalidate = Callback::new(move |()| invalidated.set(true));
            change_selection(selected, "approval-one", true, false, invalidate);
            assert_eq!(selected.get_untracked(), vec!["approval-one"]);
            assert!(invalidated.get_untracked());
            change_selection(selected, "second", true, true, invalidate);
            assert_eq!(selected.get_untracked(), vec!["approval-one"]);
            let current = run();
            let available = choice(
                "approval-one".into(),
                label(&current),
                "全额计入本次".into(),
                true,
                selected,
                Signal::derive(|| false),
                invalidate,
            )
            .to_html();
            let owned = choice(
                "approval-owned".into(),
                label(&current),
                format!("已归入执行 {}", "long-run-".repeat(10)),
                false,
                selected,
                Signal::derive(|| false),
                invalidate,
            )
            .to_html();
            assert!(owned.contains("disabled"));
            assert!(owned.contains("已归入执行"));
            assert!(available.contains("type=\"checkbox\""));
            if let Ok(path) = std::env::var("CROSSLINE_APPROVAL_CHOICES_HTML") {
                std::fs::write(path, format!("{available}{owned}")).unwrap();
            }
        });
    }
}
