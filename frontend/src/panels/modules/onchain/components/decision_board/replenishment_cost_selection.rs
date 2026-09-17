use super::OnchainData;
use leptos::prelude::*;
use shared_types::{
    OnchainExecutionReplenishmentCost, OnchainReplenishmentRun, OnchainReplenishmentRunStatus,
};

pub(super) fn selection(data: OnchainData) -> impl IntoView {
    let selected = data.execution.selected_replenishment;
    let locked = Signal::derive(move || {
        data.execution.building_execution.get() || data.execution.submitting_execution.get()
    });
    let invalidate = Callback::new(move |()| data.execution.execution_build.set(None));
    selection_for(data, selected, locked, invalidate, false)
}

pub(super) fn selection_for(data: OnchainData, selected: RwSignal<Vec<String>>, locked: Signal<bool>, invalidate: Callback<()>, chain_only: bool) -> impl IntoView {
    view! {
        <details class="onchain-ticket-evidence onchain-cost-selection">
            <summary><span>"补库费用"</span><strong>{move || {
                let count = selected.get().len();
                if count == 0 { "未归集".to_owned() } else { format!("已选 {count} 笔") }
            }}</strong></summary>
            {move || {
                let mut rows = data.replenishment.runs.get();
                if let Some(Ok(current)) = data.replenishment.run.get() {
                    rows.retain(|row| row.run_id != current.run_id);
                    rows.insert(0, current);
                }
                rows.retain(|row| row.status == OnchainReplenishmentRunStatus::Completed && (!chain_only || row.plan.legs.iter().all(|l| l.direction == shared_types::OnchainTransferDirection::WithdrawToChain)));
                let mut options = rows.iter().map(|run| (run.run_id.clone(), run_label(run))).collect::<Vec<_>>();
                for id in selected.get() {
                    if !options.iter().any(|(key, _)| key == &id) {
                        options.push((id.clone(), format!("{id} · 历史记录待读取")));
                    }
                }
                if options.is_empty() {
                    return view! { <small>"暂无已完成补库记录"</small> }.into_any();
                }
                options.into_iter().map(|(id, label)| choice(id, label, selected, locked, invalidate)).collect_view().into_any()
            }}
        </details>
    }
}

fn choice(
    id: String,
    label: String,
    selected: RwSignal<Vec<String>>,
    locked: Signal<bool>,
    invalidate: Callback<()>,
) -> impl IntoView {
    let checked_id = id.clone();
    let disabled_id = id.clone();
    let title = id.clone();
    view! {
        <label class="onchain-cost-choice" title=title>
            <input type="checkbox"
                prop:checked=move || selected.get().contains(&checked_id)
                disabled=move || locked.get() || (selected.get().len() >= 8 && !selected.get().contains(&disabled_id))
                on:change=move |event| {
                    change_selection(selected, &id, event_target_checked(&event), locked.get_untracked(), invalidate);
                }/>
            <span>{label}</span>
        </label>
    }
}

pub(super) fn change_selection(
    selected: RwSignal<Vec<String>>,
    id: &str,
    checked: bool,
    locked: bool,
    invalidate: Callback<()>,
) {
    if locked {
        return;
    }
    selected.update(|ids| {
        ids.retain(|value| value != id);
        if checked && ids.len() < 8 {
            ids.push(id.to_owned());
        }
    });
    invalidate.run(());
}

fn run_label(run: &OnchainReplenishmentRun) -> String {
    let legs = run
        .plan
        .legs
        .iter()
        .map(|leg| {
            format!(
                "{} {} {}",
                leg.venue.to_uppercase(),
                leg.transfer_amount_exact.as_deref().unwrap_or("未知"),
                leg.asset
            )
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let time = crate::panels::modules::timestamp::local_date_hm(run.updated_at_ms)
        .unwrap_or_else(|| "时间未知".into());
    format!("{legs} · {time} · 全额计入本次")
}

pub(super) fn scope(costs: &[OnchainExecutionReplenishmentCost]) -> String {
    if costs.is_empty() {
        return "未归集".into();
    }
    let fees = costs
        .iter()
        .flat_map(|cost| &cost.fees)
        .map(|fee| format!("{} {}", fee.amount_exact, fee.asset))
        .collect::<Vec<_>>()
        .join(" · ");
    format!("{} 笔 · {fees}", costs.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replenishment_allocation_ui_selection_is_explicit_bounded_and_invalidates_old_build() {
        Owner::new().with(|| {
            let selected = RwSignal::new(Vec::new());
            let invalidated = RwSignal::new(false);
            let invalidate = Callback::new(move |()| invalidated.set(true));
            assert!(selected.get_untracked().is_empty());
            change_selection(selected, "cost-1", true, false, invalidate);
            assert_eq!(selected.get_untracked(), ["cost-1"]);
            assert!(invalidated.get_untracked());
            change_selection(selected, "cost-1", true, false, invalidate);
            assert_eq!(selected.get_untracked().len(), 1);
            change_selection(selected, "cost-1", false, true, invalidate);
            assert_eq!(selected.get_untracked().len(), 1);
            change_selection(selected, "cost-1", false, false, invalidate);
            assert!(selected.get_untracked().is_empty());
            for n in 0..10 {
                change_selection(selected, &format!("cost-{n}"), true, false, invalidate);
            }
            assert_eq!(selected.get_untracked().len(), 8);
            let long = format!("历史记录 {} · 全额计入本次", "1234567890".repeat(6));
            let html = choice(
                "cost-1".into(),
                long,
                selected,
                Signal::derive(|| false),
                invalidate,
            )
            .to_html();
            assert!(html.contains("type=\"checkbox\""));
            if let Ok(path) = std::env::var("CROSSLINE_REPLENISHMENT_CHOICES_HTML") {
                std::fs::write(path, html).unwrap();
            }
        });
    }
}
