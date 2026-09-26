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
    let usable = move || row.try_get().is_some_and(|row| {
        context.quote_ready_ids.try_with(|ids| ids.contains(&row.id)).unwrap_or(false)
    });
    let selected = Memo::new(move |_| {
        current
            .get()
            .is_some_and(|(_, row)| context.selected_id.get() == row.id)
    });
    let tab_stop = Memo::new(move |_| selected.try_get().unwrap_or(false) || current.get().is_some_and(|(idx, _)| {
        idx == 0 && !context.rows.with(|rows| rows.iter().any(|row| row.id == context.selected_id.get()))
    }));
    let select = move |callback: Callback<(usize, OpportunityRow)>| {
        if let Some(Some(value)) = current.try_get_untracked() {
            callback.run(value);
        }
    };
    view! {
        <tr
            id=move || current.try_get().flatten().map(|(idx, _)| format!("opportunity-row-{idx}"))
            class=move || if usable() && row.try_get().is_some_and(|row| row.execution_eligible) { "execution-ready" } else { "observation-only" }
            class:selected=move || selected.try_get().unwrap_or(false)
            aria-selected=move || selected.try_get().unwrap_or(false).to_string()
            tabindex=move || if tab_stop.try_get().unwrap_or(false) { 0 } else { -1 }
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
            <td>{move || row.try_get().map(|row| {
                view! {
                    <div class="market-cell">
                        <strong>{row.pair.clone()}</strong>
                        <div class="market-cell-meta">
                            <span class="market-badge">{row.strategy_label.clone()}</span>
                            <span class="market-risk"><span>"风险"</span><RiskBadge risk=row.risk.clone()/></span>
                        </div>
                    </div>
                }
            })}</td>
            <td>{move || row.try_get().map(|row| {
                view! {
                    <div class="route-cell">
                        <strong>{row.long_venue.clone()}"→"{row.short_venue.clone()}</strong>
                        <LegRouteLine venue=row.long_venue.clone() leg=row.long_leg.clone() price=row.long_price.clone() evidence=row.long_market_evidence.clone()/>
                        <LegRouteLine venue=row.short_venue.clone() leg=row.short_leg.clone() price=row.short_price.clone() evidence=row.short_market_evidence.clone()/>
                    </div>
                }
            })}</td>
            <td class="decision-metric" data-label="毛边际"><strong class="muted">{move || row.try_get().map(|row| row.gross_one_cycle.clone())}</strong></td>
            <td data-label="完整成本" title=move || row.try_get().map(|row| row.cost_detail())>
                <div class="market-cell">
                    <span>{move || row.try_get().map(|row| row.round_trip_cost.clone())}</span>
                    <small>{move || row.try_get().map(|row| row.cost_evidence_label())}</small>
                </div>
            </td>
            <td class="decision-metric">
                <span class="opportunity-net-decision" class:is-observation=move || !usable() || !row.try_get().is_some_and(|row| row.execution_eligible)>
                    <small>{move || if !usable() { "上次测算" } else if row.try_get().is_some_and(|row| row.execution_eligible) { "净利" } else { "测算" }}</small>
                    <strong class=move || {
                        row.try_get().filter(|row| usable() && row.execution_eligible)
                            .map_or("muted", |row| evidence_profit_class(row.cost_verified, row.one_cycle_net_bps))
                    }>{move || row.try_get().map(|row| row.one_cycle_net.clone())}</strong>
                </span>
            </td>
            <td data-label="兑现周期" title=move || row.try_get().map(|row| row.cycle_detail())>{move || row.try_get().map(|row| row.cycle_label())}</td>
            <td data-label="规模" title=move || row.try_get().map(|row| row.depth_detail())>
                <div class="market-cell"><span>{move || row.try_get().map(|row| row.opportunity_size_label())}</span><small>{move || row.try_get().map(|row| row.depth_evidence_label())}</small></div>
            </td>
            <td>
                <div class="row-action-stack">
                    <button type="button" class="row-action"
                        disabled=move || !usable() || !row.try_get().is_some_and(|row| row.execution_eligible)
                        title=move || {
                            if !usable() { "快照正在加载、已过期或读取失败；恢复后可构建".to_owned() }
                            else { row.try_get().map(|row| execution_title(&row)).unwrap_or_default() }
                        }
                        tabindex=move || if selected.try_get().unwrap_or(false) { 0 } else { -1 }
                        on:keydown=move |ev| ev.stop_propagation()
                        on:click=move |ev| { ev.stop_propagation(); select(context.on_open); }
                    >{move || if row.try_get().is_some_and(|row| row.execution_eligible) { "构建对冲" } else { "仅观察" }}</button>
                    {move || row.try_get().and_then(|row| {
                        execution_reason(&row).map(|reason| view! { <small class="row-action-reason" title=execution_title(&row)>{reason}</small> })
                    })}
                    <button type="button" class="opportunity-row-evidence" aria-controls="opportunity-detail-panel"
                        aria-current=move || selected.try_get().unwrap_or(false).to_string()
                        tabindex=move || if selected.try_get().unwrap_or(false) { 0 } else { -1 }
                        on:keydown=move |ev| ev.stop_propagation()
                        on:click=move |ev| { ev.stop_propagation(); select(context.on_evidence); }
                    >{move || if selected.try_get().unwrap_or(false) { "当前数据依据" } else { "查看数据依据" }}</button>
                </div>
            </td>
        </tr>
    }
}
