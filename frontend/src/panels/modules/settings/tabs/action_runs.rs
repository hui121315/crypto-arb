//! 动作账本（ActionRun）设置标签页装配：拉取/选择/分页运行态。
//! 表格与详情渲染见 `rows.rs`，状态/文案派生见 `labels.rs`。

#[path = "action_runs/labels.rs"]
mod labels;
#[path = "action_runs/rows.rs"]
mod rows;

use crate::panels::modules::pagination::use_table_runtime;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::{store_choice, stored_choice};
use leptos::prelude::*;
use shared_types::ActionRun;
use std::cmp::Reverse;

use super::super::data::{settings_state, use_action_run_detail, use_action_runs};

const ACTION_RUN_PAGE_SIZE: usize = 12;
const ACTION_RUN_PAGE_STORAGE_KEY: &str = "crossline.settings.actionRuns.page";
const ACTION_RUN_SELECTED_STORAGE_KEY: &str = "crossline.settings.actionRuns.selected";

pub(in crate::panels::modules::settings) fn action_runs_tab() -> impl IntoView {
    let refresh_nonce = RwSignal::new(0_u64);
    let selected_id = RwSignal::new(stored_action_run_id());
    let runs = use_action_runs(refresh_nonce);
    let detail = use_action_run_detail(selected_id, refresh_nonce);
    let action_rows = Memo::new(move |_| {
        let state = settings_state(runs);
        sorted_action_runs(&state)
    });
    let action_key = Memo::new(move |_| "action-runs".to_owned());
    let table = use_table_runtime(
        ACTION_RUN_PAGE_STORAGE_KEY,
        action_key,
        action_rows,
        ACTION_RUN_PAGE_SIZE,
    );
    let refresh = move |_| refresh_nonce.update(|value| *value = value.wrapping_add(1));
    Effect::new(move |_| {
        selected_id.with(|id| {
            store_choice(
                ACTION_RUN_SELECTED_STORAGE_KEY,
                id.as_deref().unwrap_or_default(),
            )
        });
    });

    view! {
        <div class="settings-stack">
            <div class="settings-actions">
                <button class="row-action" on:click=refresh>"刷新"</button>
            </div>
            {rows::action_run_table(runs, table, selected_id)}
            {rows::action_run_detail(detail, selected_id)}
        </div>
    }
}

fn stored_action_run_id() -> Option<String> {
    stored_choice(ACTION_RUN_SELECTED_STORAGE_KEY, non_empty_choice)
}

fn non_empty_choice(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn sorted_action_runs(state: &LoadState<Vec<ActionRun>>) -> Vec<ActionRun> {
    let mut runs = state.value().cloned().unwrap_or_default();
    runs.sort_by_key(|run| Reverse((run.started_at_ms, run.id.clone())));
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_run_selection_parser_rejects_empty_choice() {
        assert_eq!(non_empty_choice(" act-1 "), Some("act-1".to_owned()));
        assert_eq!(non_empty_choice("  "), None);
    }

    #[test]
    fn action_run_storage_keys_are_namespaced() {
        assert!(ACTION_RUN_PAGE_STORAGE_KEY.starts_with("crossline.settings."));
        assert!(ACTION_RUN_SELECTED_STORAGE_KEY.starts_with("crossline.settings."));
        assert_ne!(ACTION_RUN_PAGE_STORAGE_KEY, ACTION_RUN_SELECTED_STORAGE_KEY);
    }
}
