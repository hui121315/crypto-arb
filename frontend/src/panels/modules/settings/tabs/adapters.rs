use crate::api::rest::{TradingAdapterOption, TradingAdaptersResponse};
use crate::panels::shared::execution_environment_label;
use crate::state::load_state::LoadState;
use crate::state::module_runtime::ModuleRuntimeState;
use super::super::runtime::{action_health, PaneState};
use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionEnvironment};

use super::super::data::{settings_state, use_trading_adapters, TradingAdapterSelectAction};
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

pub(in crate::panels::modules::settings) fn execution_environment_panel(
    runtime: super::super::runtime::EnvironmentRuntime,
    pane: PaneState,
) -> impl IntoView {
    let refresh_nonce = runtime.refresh;
    let adapters = runtime.adapters;
    use_trading_adapters(refresh_nonce, adapters);
    let select_action = runtime.select;
    pane.track(move || ModuleRuntimeState::combine([
        ModuleRuntimeState::from_load_state(&adapters.get()),
        action_health(select_action.journal, &select_action.state.get()),
    ]));
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
            {super::super::data::settings_recovery_panel(select_action.journal, select_action.recheck)}
            {environment_controls(adapters, select_action, live_confirmation)}
            <div class="settings-actions">
                <button type="button" class="row-action" disabled=move || select_action.state.get().is_pending()
                    on:click=move |_| super::super::data::bump_refresh(refresh_nonce)>"刷新执行环境"</button>
            </div>
            <details class="settings-environment-evidence">
                <summary>"环境与交易所能力"</summary>
                {move || adapter_table(settings_state(adapters), &table_for_view)}
            </details>
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
    adapters: RwSignal<LoadState<TradingAdaptersResponse>>,
    action: TradingAdapterSelectAction,
    live_confirmation: RwSignal<bool>,
) -> AnyView {
    let ready = Memo::new(move |_| matches!(adapters.get(), LoadState::Ready(_)));
    let is_live = Memo::new(move |_| {
        adapters.with(|state| {
            state
                .value()
                .is_some_and(|r| r.current_environment == ExecutionEnvironment::Live)
        })
    });
    let live_option = Memo::new(move |_| {
        adapters.with(|state| {
            state
                .value()
                .and_then(|r| environment_option(r, ExecutionEnvironment::Live))
                .cloned()
        })
    });
    let live_enabled = Memo::new(move |_| {
        ready.get() && live_option.with(|option| option.as_ref().is_some_and(|o| o.enabled))
    });
    let current_label = move || {
        adapters.with(|state| {
            state.value().map_or("状态待确认", |r| {
                execution_environment_label(r.current_environment)
            })
        })
    };
    let adapter_label = move || {
        adapters.with(|state| {
            state.value().map_or_else(
                || "未读取当前环境".into(),
                |r| format!("adapter {}", r.current),
            )
        })
    };
    let action_pending = Memo::new(move |_| action.journal.busy.get());
    let unresolved = Memo::new(move |_| action.journal.locked());
    let receipt = Memo::new(move |_| action.state.get());

    view! {
        <div class="settings-environment-control">
            <div class="settings-environment-state">
                <span class=move || if is_live.get() || !ready.get() { "status-pill pending" } else { "status-pill ready" }>
                    {current_label}
                </span>
                <div>
                    <strong>"执行环境"</strong>
                    <em>{adapter_label}</em>
                </div>
            </div>
            <div class="settings-environment-actions">
                <Show when=move || is_live.get() fallback=move || view! {
                    <Show when=move || live_confirmation.get() fallback=move || view! {
                        <button class="row-action" type="button"
                            disabled=move || action_pending.get() || unresolved.get() || !live_enabled.get()
                            title=move || live_option.get().and_then(|option| option.disabled_reason).unwrap_or_default()
                            on:click=move |_| { if live_enabled.get_untracked() { live_confirmation.set(true); } }>
                            "启用实盘"
                        </button>
                    }>
                        <button class="danger-action" type="button"
                            disabled=move || action_pending.get() || unresolved.get() || !live_enabled.get()
                            on:click=move |_| {
                                if !live_enabled.get_untracked() { return; }
                                if let Some(option) = live_option.get_untracked() {
                                    live_confirmation.set(false);
                                    action.submit.run(option.id);
                                }
                            }>"确认启用实盘"</button>
                        <button class="row-action" type="button" disabled=move || action_pending.get()
                            on:click=move |_| live_confirmation.set(false)>"取消"</button>
                    </Show>
                }>
                        <button
                            class="row-action"
                            type="button"
                            disabled=move || action_pending.get() || unresolved.get() || !ready.get()
                            on:click=move |_| {
                                if !ready.get_untracked() { return; }
                                let option = adapters.with_untracked(|state| state.value().and_then(|r| environment_option(r, ExecutionEnvironment::Paper)).filter(|o| o.enabled).cloned());
                                let Some(option) = option else { return; };
                                live_confirmation.set(false);
                                action.submit.run(option.id);
                            }
                        >
                            "切回模拟"
                        </button>
                </Show>
            </div>
        </div>
        {move || match adapters.get() {
            LoadState::Error(problem) | LoadState::Stale { problem, .. } => Some(view! { <em class="settings-message is-error" role="alert">{problem_message("执行环境待确认，请刷新", &problem)}</em> }),
            _ => None,
        }}
        <em class="settings-message" role="status">
            {move || {
                use crate::state::action_state::ActionState;
                match receipt.get() {
                    ActionState::Idle => "每笔订单仍需通过权限、运行状态与减仓交易检查".to_owned(),
                    ActionState::Pending { label, .. } => label,
                    ActionState::Accepted { .. } => "上次切换已受理，结果待核对".to_owned(),
                    ActionState::Succeeded { .. } => "上次切换已完成；当前环境以上方读取结果为准".to_owned(),
                    ActionState::Failed { .. } if unresolved.get() => "上次切换结果待核对，暂不能再次切换".to_owned(),
                    ActionState::Failed { problem, .. } => format!("上次切换失败：{}", problem.message),
                }
            }}
        </em>
        <Show when=move || !matches!(receipt.get(), crate::state::action_state::ActionState::Idle)>
            <details class="settings-environment-evidence">
                <summary>"操作数据依据"</summary>
                <p class="settings-message">{move || action_message("", &receipt.get())}</p>
            </details>
        </Show>
    }
    .into_any()
}

fn environment_option(
    response: &TradingAdaptersResponse,
    environment: ExecutionEnvironment,
) -> Option<&TradingAdapterOption> {
    response
        .options
        .iter()
        .find(|option| option.environment == environment)
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
        assert!(status.contains("运行状态数据依据"));
        assert!(!status.contains("权限验证完整"));
        assert!(!status.contains("可下单"));
    }
}
