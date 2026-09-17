use leptos::prelude::*;
use shared_types::StrategyKindInfo;

use super::super::data::{futures_chips, FuturesFilter};
use crate::panels::modules::strategy_kinds::strategy_option_target_index;
use crate::state::load_state::LoadState;
use crate::state::strategy_kinds::{strategy_kinds_view, StrategyKindsNote};

pub(in crate::panels::modules::futures) fn strategy_chips(
    filter: RwSignal<FuturesFilter>,
    kinds: RwSignal<LoadState<Vec<StrategyKindInfo>>>,
) -> impl IntoView {
    view! {
        <div class="futures-strategy-control">
            {move || {
                let raw_state = kinds.get();
                let allow_current_fallback = !matches!(&raw_state, LoadState::Ready(_));
                let state = strategy_kinds_view(&raw_state);
                let mut strategies = futures_chips(&state.rows);
                if strategies.is_empty() && allow_current_fallback {
                    strategies.push(filter.get().strategy);
                }
                let tab_values = StoredValue::new(strategies.clone());
                let tab_node_refs = strategies
                    .iter()
                    .map(|_| NodeRef::<leptos::html::Button>::new())
                    .collect::<Vec<_>>();
                let tab_refs = StoredValue::new(tab_node_refs.clone());
                view! {
                    <>
                        <div
                            class="strategy-chips futures-strategy-tabs"
                            role="tablist"
                            aria-label="套利策略"
                            aria-orientation="horizontal"
                        >
                            {strategies.into_iter().zip(tab_node_refs).enumerate().map(|(index, (strategy, node_ref))| {
                                view! {
                                    <button
                                        node_ref=node_ref
                                        type="button"
                                        role="tab"
                                        aria-selected=move || (filter.get().strategy == strategy).to_string()
                                        tabindex=move || if filter.get().strategy == strategy { 0 } else { -1 }
                                        class=move || if filter.get().strategy == strategy { "active" } else { "" }
                                        on:click=move |_| filter.update(|value| value.strategy = strategy)
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
                                        {strategy.label()}
                                    </button>
                                }
                            }).collect_view()}
                        </div>
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
