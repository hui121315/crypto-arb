use super::*;

pub(super) fn row_view(
    current: Memo<Option<(usize, OpportunityRow)>>,
    initial: OpportunityRow,
    context: OpportunityRowContext,
) -> impl IntoView {
    let row = Memo::new(move |_| {
        current
            .get()
            .map(|(_, row)| row)
            .unwrap_or_else(|| initial.clone())
    });
    let selected = Memo::new(move |_| {
        current
            .get()
            .is_some_and(|(idx, _)| context.selected_idx.get() == idx)
    });
    let select = move |callback: Callback<(usize, OpportunityRow)>| {
        if let Some(Some(value)) = current.try_get_untracked() {
            callback.run(value);
        }
    };
    view! {
        <tr
            id=move || current.get().map(|(idx, _)| format!("opportunity-row-{idx}"))
            class=move || if row.get().execution_eligible { "execution-ready" } else { "observation-only" }
            class:selected=move || selected.get()
            aria-selected=move || selected.get().to_string()
            tabindex=move || if selected.get() { 0 } else { -1 }
            on:click=move |_| select(context.on_select)
            on:keydown=move |ev| {
                let Some(Some((idx, _))) = current.try_get_untracked() else { return };
                let key = ev.key();
                if let Some(target) = row_navigation_target(&key, idx, context.rows.with(|rows| rows.len().min(OPPORTUNITY_PAGE_SIZE))) {
                    ev.prevent_default();
                    if let Some(row) = context.rows.with(|rows| rows.get(target).cloned()) {
                        context.on_select.run((target, row));
                        focus_opportunity_row(target);
                    }
                } else if key == "Enter" || key == " " {
                    ev.prevent_default();
                    select(context.on_select);
                }
            }
        >
            <td>{move || {
                let row = row.get();
                view! {
                    <div class="market-cell">
                        <strong>{row.pair.clone()}</strong>
                        <div class="market-cell-meta">
                            <span class="market-badge">{row.strategy_label.clone()}</span>
                            <span class="market-risk"><span>"风险"</span><RiskBadge risk=row.risk.clone()/></span>
                        </div>
                        <small class="opportunity-mobile-route">{format!("{} → {}", row.long_venue, row.short_venue)}</small>
                    </div>
                }
            }}</td>
            <td>{move || {
                let row = row.get();
                view! {
                    <div class="route-cell">
                        <strong>{row.long_venue.clone()}"→"{row.short_venue.clone()}</strong>
                        <LegRouteLine venue=row.long_venue.clone() leg=row.long_leg.clone() price=row.long_price.clone() evidence=row.long_market_evidence.clone()/>
                        <LegRouteLine venue=row.short_venue.clone() leg=row.short_leg.clone() price=row.short_price.clone() evidence=row.short_market_evidence.clone()/>
                    </div>
                }
            }}</td>
            <td class="decision-metric"><strong class="muted">{move || row.get().gross_one_cycle.clone()}</strong></td>
            <td title=move || row.get().cost_detail()>
                <div class="market-cell">
                    <span>{move || row.get().round_trip_cost.clone()}</span>
                    <small>{move || row.get().cost_evidence_label()}</small>
                </div>
            </td>
            <td class="decision-metric">
                <span class="opportunity-net-decision" class:is-observation=move || !row.get().execution_eligible>
                    <small>{move || if row.get().execution_eligible { "净利" } else { "测算" }}</small>
                    <strong class=move || {
                        let row = row.get();
                        if row.execution_eligible { evidence_profit_class(row.cost_verified, row.one_cycle_net_bps) } else { "muted" }
                    }>{move || row.get().one_cycle_net.clone()}</strong>
                </span>
            </td>
            <td title=move || row.get().cycle_detail()>{move || row.get().cycle_label()}</td>
            <td title=move || row.get().depth_detail()>
                <div class="market-cell"><span>{move || row.get().opportunity_size_label()}</span><small>{move || row.get().depth_evidence_label()}</small></div>
            </td>
            <td>
                <div class="row-action-stack">
                    <button type="button" class="row-action"
                        disabled=move || !context.snapshot_usable.get() || !row.try_get().is_some_and(|row| row.execution_eligible)
                        title=move || {
                            if !context.snapshot_usable.get() { "快照正在加载、已过期或读取失败；恢复后可构建".to_owned() }
                            else { row.try_get().map(|row| execution_title(&row)).unwrap_or_default() }
                        }
                        tabindex=move || if selected.get() { 0 } else { -1 }
                        on:keydown=move |ev| ev.stop_propagation()
                        on:click=move |ev| { ev.stop_propagation(); select(context.on_open); }
                    >{move || if row.get().execution_eligible { "构建对冲" } else { "仅观察" }}</button>
                    {move || {
                        let row = row.get();
                        execution_reason(&row).map(|reason| view! { <small class="row-action-reason" title=execution_title(&row)>{reason}</small> })
                    }}
                    <button type="button" class="opportunity-row-evidence" aria-controls="opportunity-detail-panel"
                        aria-current=move || selected.get().to_string()
                        tabindex=move || if selected.get() { 0 } else { -1 }
                        on:keydown=move |ev| ev.stop_propagation()
                        on:click=move |ev| { ev.stop_propagation(); select(context.on_evidence); }
                    >{move || if selected.get() { "当前证据" } else { "查看证据" }}</button>
                </div>
            </td>
        </tr>
    }
}
