use leptos::prelude::*;
use shared_types::StrategyKindInfo;

use super::super::data::OpportunityFilter;
use crate::panels::modules::strategy_kinds::{
    p0_strategy_chip_options, strategy_option_target_index,
};
use crate::panels::modules::strategy_scope::p0_strategy_label;
use crate::state::load_state::LoadState;
use crate::state::strategy_kinds::{strategy_kinds_view, StrategyKindsNote};

pub(in crate::panels::modules::opportunities) fn strategy_chips(
    filter: RwSignal<OpportunityFilter>,
    kinds: RwSignal<LoadState<Vec<StrategyKindInfo>>>,
) -> impl IntoView {
    view! {
        <div
            class="strategy-chips"
            role="tablist"
            aria-label="策略范围"
            aria-orientation="horizontal"
        >
            {move || {
                let raw_state = kinds.get();
                let allow_current_fallback = !matches!(&raw_state, LoadState::Ready(_));
                let state = strategy_kinds_view(&raw_state);
                let active_kind = filter.get_untracked().strategy;
                let mut options = p0_strategy_chip_options(&state.rows);
                if options.is_empty() && allow_current_fallback {
                    options.push(active_kind.and_then(|kind| {
                        p0_strategy_label(kind).map(|label| (Some(kind), label))
                    }).unwrap_or((None, "全部")));
                }
                let tab_values = StoredValue::new(
                    options.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
                );
                let tab_node_refs = options
                    .iter()
                    .map(|_| NodeRef::<leptos::html::Button>::new())
                    .collect::<Vec<_>>();
                let tab_refs = StoredValue::new(tab_node_refs.clone());
                view! {
                    <>
                        {options.into_iter().zip(tab_node_refs).enumerate().map(|(index, ((kind, label), node_ref))| {
                            view! {
                                <button
                                    node_ref=node_ref
                                    type="button"
                                    role="tab"
                                    aria-selected=move || (filter.get().strategy == kind).to_string()
                                    tabindex=move || if filter.get().strategy == kind { 0 } else { -1 }
                                    class=move || if filter.get().strategy == kind { "active" } else { "" }
                                    on:click=move |_| filter.update(|value| value.strategy = kind)
                                    on:keydown=move |event| {
                                        let key = event.key();
                                        let next_index = tab_values.with_value(|values| {
                                            strategy_option_target_index(&key, index, values.len())
                                        });
                                        let Some(next_index) = next_index else { return };
                                        event.prevent_default();
                                        let next = tab_values.with_value(|values| values.get(next_index).copied());
                                        if let Some(next) = next {
                                            filter.update(|value| value.strategy = next);
                                        }
                                        let next_ref = tab_refs.with_value(|refs| refs.get(next_index).cloned());
                                        if let Some(button) = next_ref.and_then(|node_ref| node_ref.get()) {
                                            let _ = button.focus();
                                        }
                                    }
                                >
                                    {label}
                                </button>
                            }
                        }).collect_view()}
                        {strategy_note(state.note)}
                    </>
                }
            }}
        </div>
    }
}

fn strategy_note(note: Option<StrategyKindsNote>) -> Option<impl IntoView> {
    note.map(|note| {
        let class_name = if note.is_error {
            "settings-message is-error"
        } else {
            "settings-message"
        };
        view! { <em class=class_name>{note.text}</em> }
    })
}
