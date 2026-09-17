use super::*;

pub(super) fn row_view(
    idx: usize,
    row: &OpportunityRow,
    context: OpportunityRowContext,
) -> impl IntoView {
    let OpportunityRowContext {
        rows,
        selected_idx,
        on_select,
        on_evidence,
        on_open,
    } = context;
    let row = Arc::clone(row);
    let selected = Arc::clone(&row);
    let key_selected = Arc::clone(&row);
    let evidence = Arc::clone(&row);
    let opened = Arc::clone(&row);
    let enabled = row.execution_eligible;
    let title = execution_title(row.as_ref());
    let reason_title = title.clone();
    let reason = execution_reason(row.as_ref());
    let value_class = if enabled { "positive" } else { "muted" };
    let one_cycle_class = if enabled {
        evidence_profit_class(row.cost_verified, row.one_cycle_net_bps)
    } else {
        "muted"
    };
    let net_decision_class = if enabled {
        "opportunity-net-decision"
    } else {
        "opportunity-net-decision is-observation"
    };
    let net_decision_label = if enabled { "净利" } else { "测算" };
    view! {
        <tr
            id=format!("opportunity-row-{idx}")
            class=if enabled { "execution-ready" } else { "observation-only" }
            class:selected=move || selected_idx.get() == idx
            aria-selected=move || (selected_idx.get() == idx).to_string()
            tabindex=move || if selected_idx.get() == idx { 0 } else { -1 }
            on:click=move |_| {
                selected_idx.set(idx);
                on_select.run((idx, Arc::clone(&selected)));
            }
            on:keydown=move |ev| {
                let key = ev.key();
                if let Some(target_idx) = row_navigation_target(
                    key.as_str(),
                    idx,
                    rows.with(|rows| rows.len().min(OPPORTUNITY_PAGE_SIZE)),
                ) {
                    ev.prevent_default();
                    if let Some(target_row) = rows.with(|rows| rows.get(target_idx).cloned()) {
                        selected_idx.set(target_idx);
                        on_select.run((target_idx, target_row));
                        focus_opportunity_row(target_idx);
                    }
                } else if key == "Enter" || key == " " {
                    ev.prevent_default();
                    selected_idx.set(idx);
                    on_select.run((idx, Arc::clone(&key_selected)));
                }
            }
        >
            <td>
                <div class="market-cell">
                    <strong>{row.pair.clone()}</strong>
                    <div class="market-cell-meta">
                        <span class="market-badge">{row.strategy_label.clone()}</span>
                        <span class="market-risk"><span>"风险"</span><RiskBadge risk=row.risk.clone()/></span>
                    </div>
                </div>
            </td>
            <td>
                <div class="route-cell">
                    <strong>{row.long_venue.clone()}"→"{row.short_venue.clone()}</strong>
                    <LegRouteLine
                        venue=row.long_venue.clone()
                        leg=row.long_leg.clone()
                        price=row.long_price.clone()
                        evidence=row.long_market_evidence.clone()
                    />
                    <LegRouteLine
                        venue=row.short_venue.clone()
                        leg=row.short_leg.clone()
                        price=row.short_price.clone()
                        evidence=row.short_market_evidence.clone()
                    />
                </div>
            </td>
            <td class="decision-metric"><strong class=value_class>{row.gross_one_cycle.clone()}</strong></td>
            <td title=row.cost_detail()>
                <div class="market-cell">
                    <span>{row.round_trip_cost.clone()}</span>
                    <small>{row.cost_evidence_label()}</small>
                </div>
            </td>
            <td class="decision-metric">
                <span class=net_decision_class>
                    <small>{net_decision_label}</small>
                    <strong class=one_cycle_class>{row.one_cycle_net.clone()}</strong>
                </span>
            </td>
            <td title=row.cycle_detail()>{row.cycle_label()}</td>
            <td title=row.depth_detail()>
                <div class="market-cell">
                    <span>{row.opportunity_size_label()}</span>
                    <small>{row.depth_evidence_label()}</small>
                </div>
            </td>
            <td>
                <div class="row-action-stack">
                    {if enabled {
                        view! {
                            <button
                                type="button"
                                class="row-action"
                                title=title
                                tabindex=move || if selected_idx.get() == idx { 0 } else { -1 }
                                on:keydown=move |ev| ev.stop_propagation()
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    selected_idx.set(idx);
                                    on_open.run((idx, Arc::clone(&opened)));
                                }
                            >
                                "构建对冲"
                            </button>
                        }.into_any()
                    } else {
                        view! {
                            <span class="observation-state" title=title>"仅观察"</span>
                        }.into_any()
                    }}
                    {reason.map(|text| {
                        view! { <small class="row-action-reason" title=reason_title>{text}</small> }
                    })}
                    <button
                        type="button"
                        class="opportunity-row-evidence"
                        aria-controls="opportunity-detail-panel"
                        aria-current=move || (selected_idx.get() == idx).to_string()
                        tabindex=move || if selected_idx.get() == idx { 0 } else { -1 }
                        on:keydown=move |ev| ev.stop_propagation()
                        on:click=move |ev| {
                            ev.stop_propagation();
                            on_evidence.run((idx, Arc::clone(&evidence)));
                        }
                    >
                        {move || if selected_idx.get() == idx { "当前证据" } else { "查看证据" }}
                    </button>
                </div>
            </td>
        </tr>
    }
}
