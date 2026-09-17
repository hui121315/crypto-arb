use leptos::prelude::*;

use super::super::data::OpportunityFilter;

pub(in crate::panels::modules::opportunities) fn opportunity_filter_bar(
    filter: RwSignal<OpportunityFilter>,
) -> impl IntoView {
    view! {
        <div class="filter-actions">
            <label class="filter-search">
                <span>"搜索"</span>
                <input
                    type="search"
                    placeholder="币种 / 交易所 / 路由"
                    prop:value=move || filter.get().query
                    on:input=move |ev| update_query(filter, event_target_value(&ev))
                />
            </label>
            <label class="filter-profit">
                <span>"最低净利"</span>
                <input
                    type="number"
                    aria-label="最低净利百分比"
                    min="0"
                    max="100"
                    step="0.01"
                    prop:value=move || filter.get().min_net_pct.to_string()
                    on:input=move |ev| update_min_net(filter, &event_target_value(&ev))
                />
                <em aria-hidden="true">"%"</em>
            </label>
        </div>
    }
}

fn update_query(filter: RwSignal<OpportunityFilter>, query: String) {
    filter.update(|value| value.query = query);
}

fn update_min_net(filter: RwSignal<OpportunityFilter>, value: &str) {
    let parsed = value.parse::<f64>().unwrap_or_default();
    filter.update(|filter| filter.min_net_pct = parsed.clamp(0.0, 100.0));
}
