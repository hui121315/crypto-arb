use leptos::prelude::*;

use super::super::data::FuturesFilter;

pub(in crate::panels::modules::futures) fn futures_filter_bar(
    filter: RwSignal<FuturesFilter>,
) -> impl IntoView {
    view! {
        <div class="futures-filter-bar">
            <label class="filter-search">
                <span>"品种 / 本页场所"</span>
                <input
                    type="search"
                    placeholder="BTC / BINANCE"
                    prop:value=move || filter.get().query
                    on:input=move |ev| update_query(filter, event_target_value(&ev))
                />
            </label>
            <div
                class="filter-stepper"
                role="group"
                aria-label=move || filter.get().strategy.filter_metric_label()
            >
                <span class="filter-stepper-label">
                    {move || filter.get().strategy.filter_metric_label()}
                </span>
                <div class="filter-stepper-control">
                    <button
                        type="button"
                        disabled=move || !can_decrease_metric(&filter.get())
                        aria-label="降低筛选阈值"
                        title="降低筛选阈值"
                        on:click=move |_| update_min_metric(filter, -1.0)
                    >
                        "−"
                    </button>
                    <output aria-live="polite">{move || filter_metric_text(&filter.get())}</output>
                    <button
                        type="button"
                        disabled=move || !can_increase_metric(&filter.get())
                        aria-label="提高筛选阈值"
                        title="提高筛选阈值"
                        on:click=move |_| update_min_metric(filter, 1.0)
                    >
                        "+"
                    </button>
                </div>
            </div>
            <button
                class="futures-filter-reset"
                type="button"
                disabled=move || !filter_constraints_active(&filter.get())
                on:click=move |_| filter.update(clear_filter_constraints)
            >
                "重置"
            </button>
        </div>
    }
}

fn update_query(filter: RwSignal<FuturesFilter>, query: String) {
    filter.update(|value| value.query = query);
}

fn update_min_metric(filter: RwSignal<FuturesFilter>, direction: f64) {
    filter.update(|value| {
        value.min_net_pct = (value.min_net_pct + direction * 0.05).clamp(0.0, 100.0);
    });
}

fn filter_metric_text(filter: &FuturesFilter) -> String {
    format!("{:.2}%", filter.min_net_pct)
}

fn can_decrease_metric(filter: &FuturesFilter) -> bool {
    filter.min_net_pct > 0.0
}

fn can_increase_metric(filter: &FuturesFilter) -> bool {
    filter.min_net_pct < 100.0
}

fn filter_constraints_active(filter: &FuturesFilter) -> bool {
    !filter.query.trim().is_empty() || filter.min_net_pct > 0.0
}

fn clear_filter_constraints(filter: &mut FuturesFilter) {
    filter.query.clear();
    filter.min_net_pct = 0.0;
}

#[cfg(test)]
mod tests {
    use super::super::super::data::StrategyFilter;
    use super::*;

    #[test]
    fn reset_keeps_strategy_and_clears_local_constraints() {
        let mut filter = FuturesFilter {
            strategy: StrategyFilter::SpotPerp,
            min_net_pct: 0.25,
            query: "BTC".to_owned(),
        };

        assert!(filter_constraints_active(&filter));
        clear_filter_constraints(&mut filter);

        assert_eq!(filter.strategy, StrategyFilter::SpotPerp);
        assert!(!filter_constraints_active(&filter));
    }

    #[test]
    fn one_shot_spread_uses_its_own_net_filter() {
        let filter = FuturesFilter {
            strategy: StrategyFilter::SpotCross,
            min_net_pct: 0.10,
            query: String::new(),
        };

        assert!(filter_constraints_active(&filter));
        assert_eq!(filter_metric_text(&filter), "0.10%");
        assert!(can_decrease_metric(&filter));
        assert!(can_increase_metric(&filter));
        assert!(!can_decrease_metric(&FuturesFilter::default()));
    }

    #[test]
    fn perp_cross_filters_on_native_single_event_net_profit() {
        let filter = FuturesFilter {
            strategy: StrategyFilter::PerpCross,
            min_net_pct: 0.15,
            query: String::new(),
        };

        assert_eq!(filter.strategy.filter_metric_label(), "本页最低费后边际");
        assert_eq!(filter_metric_text(&filter), "0.15%");
    }
}
