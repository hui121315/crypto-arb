use crate::api::rest::{TradingAdapterOption, TradingAdaptersResponse};
use crate::panels::shared::execution_environment_label;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionEnvironment};

use super::super::data::{
    settings_state, use_trading_adapter_select_action, use_trading_adapters,
    TradingAdapterSelectAction,
};
use super::{action_message, problem_cell, problem_message};
use crate::panels::modules::pagination::{page_controls, use_table_runtime, TableRuntimeHandle};

#[path = "adapters/rows.rs"]
mod rows;
mod venue_capabilities;

use rows::{adapter_row, capability_text};
#[cfg(test)]
use rows::{adapter_status, credential_label};
use venue_capabilities::venue_capabilities_table;

const ADAPTER_PAGE_SIZE: usize = 12;
const ADAPTER_PAGE_STORAGE_KEY: &str = "crossline.settings.adapters.page";

pub(crate) fn execution_environment_panel() -> impl IntoView {
    let refresh_nonce = RwSignal::new(0_u64);
    let adapters = use_trading_adapters(refresh_nonce);
    let select_action = use_trading_adapter_select_action(refresh_nonce);
    let live_confirmation = RwSignal::new(false);
    let adapter_rows = Memo::new(move |_| {
        let state = settings_state(adapters);
        adapter_options(&state)
    });
    let adapter_key = Memo::new(move |_| {
        let state = settings_state(adapters);
        adapter_dataset_key(state.value())
    });
    let table = use_table_runtime(
        ADAPTER_PAGE_STORAGE_KEY,
        adapter_key,
        adapter_rows,
        ADAPTER_PAGE_SIZE,
    );
    let table_for_view = table;

    view! {
        <div class="settings-stack">
            {move || adapter_table(
                settings_state(adapters),
                &table_for_view,
                select_action,
                live_confirmation,
            )}
        </div>
    }
}

fn adapter_options(state: &LoadState<TradingAdaptersResponse>) -> Vec<TradingAdapterOption> {
    state
        .value()
        .map(|response| response.options.clone())
        .unwrap_or_default()
}

fn adapter_dataset_key(response: Option<&TradingAdaptersResponse>) -> String {
    response.map_or_else(
        || "adapters:none".into(),
        |response| {
            let ids = response
                .options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>()
                .join("|");
            format!("adapters:{}:{ids}", response.current)
        },
    )
}

fn adapter_table(
    state: LoadState<TradingAdaptersResponse>,
    table: &TableRuntimeHandle<TradingAdapterOption>,
    select_action: TradingAdapterSelectAction,
    live_confirmation: RwSignal<bool>,
) -> AnyView {
    let (response, stale_problem) = match state {
        LoadState::Ready(response) => (response, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取执行环境诊断失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取执行环境诊断"</div> }.into_any();
        }
    };
    let current = response.current;
    let current_environment = response.current_environment;
    let live_option = response
        .options
        .iter()
        .find(|option| option.id == "live")
        .cloned();
    let venue_table = venue_capabilities_table(&response.venues);
    let total = table.total;
    let current_page = table.current_page;
    let runtime = table.runtime;
    let current_for_rows = current.clone();
    let visible_rows = move || {
        runtime
            .get()
            .rows
            .into_iter()
            .map(|row| adapter_row(row, &current_for_rows))
            .collect_view()
    };
    view! {
        <>
            {environment_controls(
                &current,
                current_environment,
                live_option,
                select_action,
                live_confirmation,
            )}
            {adapter_stale_message(stale_problem.as_ref()).map(|message| view! {
                <em class="settings-message is-error">{message}</em>
            })}
            <div class="table-wrap">
                <table class="clean-table settings-table" data-settings-table="execution-environment">
                    <thead>
                        <tr>
                            <th>"执行环境"</th>
                            <th>"模式"</th>
                            <th>"凭证"</th>
                            <th>"能力"</th>
                            <th>"状态"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {visible_rows}
                    </tbody>
                </table>
            </div>
            {move || {
                (total.get() > ADAPTER_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, ADAPTER_PAGE_SIZE)}
                })
            }}
            {venue_table}
        </>
    }
    .into_any()
}

fn environment_controls(
    current: &str,
    environment: ExecutionEnvironment,
    live_option: Option<TradingAdapterOption>,
    action: TradingAdapterSelectAction,
    live_confirmation: RwSignal<bool>,
) -> AnyView {
    let is_live = environment == ExecutionEnvironment::Live;
    let live_enabled = live_option.as_ref().is_some_and(|option| option.enabled);
    let disabled_reason = live_option
        .and_then(|option| option.disabled_reason)
        .unwrap_or_else(|| "先补齐至少一个交易所凭证字段组".to_owned());
    let current_label = execution_environment_label(environment);
    let adapter_label = format!("adapter {current}");
    let action_pending = Memo::new(move |_| action.state.get().is_pending());

    view! {
        <div class="settings-environment-control">
            <div class="settings-environment-state">
                <span class=if is_live { "status-pill pending" } else { "status-pill ready" }>
                    {current_label}
                </span>
                <div>
                    <strong>"执行环境"</strong>
                    <em>{adapter_label}</em>
                </div>
            </div>
            <div class="settings-environment-actions">
                {if is_live {
                    view! {
                        <button
                            class="row-action"
                            type="button"
                            disabled=move || action_pending.get()
                            on:click=move |_| {
                                live_confirmation.set(false);
                                action.submit.run("mock".to_owned());
                            }
                        >
                            "切回模拟"
                        </button>
                    }
                    .into_any()
                } else {
                    view! {
                        <Show
                            when=move || live_confirmation.get()
                            fallback=move || view! {
                                <button
                                    class="row-action"
                                    type="button"
                                    disabled=move || action_pending.get() || !live_enabled
                                    title=disabled_reason.clone()
                                    on:click=move |_| live_confirmation.set(true)
                                >
                                    "启用实盘"
                                </button>
                            }
                        >
                            <button
                                class="danger-action"
                                type="button"
                                disabled=move || action_pending.get() || !live_enabled
                                on:click=move |_| {
                                    live_confirmation.set(false);
                                    action.submit.run("live".to_owned());
                                }
                            >
                                "确认启用实盘"
                            </button>
                            <button
                                class="row-action"
                                type="button"
                                disabled=move || action_pending.get()
                                on:click=move |_| live_confirmation.set(false)
                            >
                                "取消"
                            </button>
                        </Show>
                    }
                    .into_any()
                }}
            </div>
        </div>
        <em class="settings-message">
            {move || action_message("每笔订单仍需通过权限、运行态与减仓预检", &action.state.get())}
        </em>
    }
    .into_any()
}

fn adapter_stale_message(problem: Option<&ApiProblem>) -> Option<String> {
    problem.map(|problem| problem_message("执行环境诊断刷新失败，显示上次结果", problem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_stale_message_keeps_request_and_retry_context() {
        let problem = ApiProblem::new("RATE_LIMITED", "adapter refresh slow")
            .with_status(429)
            .with_request_id(Some("req-adapter".into()))
            .with_retry_after_ms(Some(3_000));

        let message = adapter_stale_message(Some(&problem)).unwrap_or_default();

        assert!(message.contains("执行环境诊断刷新失败，显示上次结果"));
        assert!(message.contains("HTTP 429"));
        assert!(message.contains("request_id req-adapter"));
        assert!(message.contains("retry 3000ms"));
    }

    #[test]
    fn adapter_stale_message_is_absent_without_problem() {
        assert!(adapter_stale_message(None).is_none());
    }

    #[test]
    fn live_adapter_copy_separates_configured_fields_from_readiness() {
        let option = TradingAdapterOption {
            id: "live_router".to_owned(),
            label: "实盘".to_owned(),
            environment: ExecutionEnvironment::Live,
            enabled: true,
            credentials_available: true,
            capabilities: Default::default(),
            disabled_reason: None,
        };

        assert_eq!(credential_label(&option), "字段组已补齐");
        let status = adapter_status(&option);
        assert!(status.contains("运行态证据"));
        assert!(!status.contains("权限验证完整"));
        assert!(!status.contains("可下单"));
    }
}
