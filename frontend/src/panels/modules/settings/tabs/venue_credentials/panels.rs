use super::*;

#[path = "panels/account_state.rs"]
mod account_state;
#[path = "panels/operation_health.rs"]
mod operation_health;

use operation_health::trading_runtime_evidence_panel;

pub(super) fn account_state_evidence_panel(
    state: LoadState<shared_types::AccountStateSnapshot>,
    venue_id: &str,
) -> AnyView {
    account_state::account_state_evidence_panel(state, venue_id)
}

pub(super) fn venue_options(
    state: LoadState<VenueCredentialsResponse>,
    selected: RwSignal<String>,
) -> AnyView {
    let response = match state {
        LoadState::Ready(response)
        | LoadState::Stale {
            value: response, ..
        } => response,
        LoadState::Error(problem) => {
            let message = venue_options_error_label(&problem);
            let title = message.clone();
            return view! { <option value="" title=title>{message}</option> }.into_any();
        }
        LoadState::Loading => return view! { <option value="">"读取中"</option> }.into_any(),
    };
    response
        .venues
        .into_iter()
        .map(move |row| {
            let venue = row.venue.clone();
            view! { <option value=row.venue selected=move || selected.get() == venue>{row.label}</option> }
        })
        .collect_view()
        .into_any()
}
pub(super) fn venue_options_error_label(problem: &ApiProblem) -> String {
    problem_message("读取交易所列表失败", problem)
}

pub(super) fn credentials_panel(
    state: LoadState<VenueCredentialsResponse>,
    venue_id: &str,
    table: &TableRuntimeHandle<VenueCredentialField>,
) -> AnyView {
    let (response, stale_problem) = match state {
        LoadState::Ready(response) => (response, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取凭证状态失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取凭证状态"</div> }.into_any();
        }
    };
    let secret_storage = response.secret_storage.clone();
    let selected_status = selected_credential_status(Some(response), venue_id);
    if selected_status.is_none() {
        return view! { <div class="empty-cell">"请选择交易所"</div> }.into_any();
    };
    let total = table.total;
    let current_page = table.current_page;
    let visible_rows = table
        .runtime
        .get()
        .rows
        .into_iter()
        .map(field_row)
        .collect_view();
    view! {
        <>
            {credentials_stale_message(stale_problem.as_ref()).map(|message| view! {
                <em class="settings-message is-error">{message}</em>
            })}
            {secret_storage_panel(secret_storage)}
            {static_capability_evidence_panel(selected_status.clone())}
            {validation_evidence_panel(selected_status)}
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"字段"</th>
                            <th>"环境变量"</th>
                            <th>"来源"</th>
                            <th>"状态"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {visible_rows}
                    </tbody>
                </table>
            </div>
            {move || {
                (total.get() > CREDENTIAL_FIELDS_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, CREDENTIAL_FIELDS_PAGE_SIZE)}
                })
            }}
        </>
    }
    .into_any()
}

pub(super) fn credentials_stale_message(problem: Option<&ApiProblem>) -> Option<String> {
    problem.map(|problem| problem_message("凭证状态刷新失败，显示上次结果", problem))
}

pub(super) fn secret_storage_panel(status: SecretStorageStatus) -> AnyView {
    let mode = secret_storage_mode_label(status.mode);
    let health = secret_storage_health_label(status.health);
    let health_class = secret_storage_health_class(status.health);
    let encrypted = if status.encrypted {
        "已加密"
    } else {
        "未加密"
    };
    let persistence = if status.persistent {
        "可持久化"
    } else {
        "仅本进程"
    };
    let atomic = if status.atomic_write {
        "原子写入"
    } else {
        "非原子"
    };
    let path = status.path.unwrap_or_else(|| "-".to_owned());
    let warning = status.warning.unwrap_or_default();
    let last_error = status.last_error.unwrap_or_default();

    view! {
        <div class="runtime-health-panel" data-secret-storage-health=health>
            <div class="runtime-health-head">
                <div>
                    <strong>"Secret 存储"</strong>
                    <em>{status.message}</em>
                    {(!warning.is_empty()).then(|| view! { <em>{warning}</em> })}
                    {(!last_error.is_empty()).then(|| view! {
                        <em class="settings-message is-error">{"后端错误："}{last_error}</em>
                    })}
                </div>
                <span class=health_class>{health}</span>
            </div>
            // 六项存储事实带标签展示——裸值盒（"Runtime / 仅本进程 / 未加密…"）
            // 无法判断每个值回答的是什么问题。
            <div class="storage-facts">
                {storage_fact("存储模式", mode.to_owned())}
                {storage_fact("缓存介质", status.label)}
                {storage_fact("持久化", persistence.to_owned())}
                {storage_fact("加密", encrypted.to_owned())}
                {storage_fact("写入", atomic.to_owned())}
                {storage_fact("路径", path)}
            </div>
        </div>
    }
    .into_any()
}

fn storage_fact(label: &'static str, value: String) -> impl IntoView {
    view! {
        <span class="storage-fact">
            <i>{label}</i>
            <b>{value}</b>
        </span>
    }
}

pub(super) fn field_row(field: VenueCredentialField) -> impl IntoView {
    let status = if field.configured || field.required {
        credential_field_label(field.configured)
    } else {
        "未填写（可选）"
    };
    let source = credential_field_source_label(field.source);
    view! {
        <tr>
            <td>{field.label}</td>
            <td>{field.env_key}</td>
            <td>{source}</td>
            <td>{status}</td>
        </tr>
    }
}

pub(super) fn runtime_health_panel(
    state: LoadState<VenueOperationHealthSnapshot>,
    venue_id: &str,
    table: &TableRuntimeHandle<VenueOperationHealth>,
) -> AnyView {
    let (snapshot, stale_problem) = match state {
        LoadState::Ready(snapshot) => (snapshot, None),
        LoadState::Stale {
            value: snapshot,
            problem,
        } => (snapshot, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取运行状态验证失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取运行状态验证"</div> }.into_any();
        }
    };
    let stale_message = runtime_stale_message(stale_problem.as_ref());
    let generated_at_ms = snapshot.generated_at_ms;
    let selection = selected_runtime_health_rows(snapshot, venue_id);
    let runtime = table.runtime.get();
    let current_page = table.current_page;
    let total = table.total;
    let visible_count = runtime.budget.rendered_rows;
    let status_class = runtime_summary_class(selection.total, selection.attention);
    let status_text = runtime_summary_status(selection.total, selection.attention);
    let summary = runtime_header_summary(
        venue_id,
        selection.total,
        selection.attention,
        visible_count,
        generated_at_ms,
    );
    let rows = runtime.rows.iter().map(runtime_health_row).collect_view();

    view! {
        <div class="runtime-health-panel">
            <div class="runtime-health-head">
                <div>
                    <strong>"运行状态验证"</strong>
                    <em>{summary}</em>
                </div>
                <span class=status_class>{status_text}</span>
            </div>
            {trading_runtime_evidence_panel(venue_id, &selection.trading_evidence)}
            {stale_message.map(|message| view! { <div class="empty-cell">{message}</div> })}
            <div class="table-wrap">
                <table class="clean-table settings-table runtime-health-table">
                    <thead>
                        <tr>
                            <th>"操作"</th>
                            <th>"状态"</th>
                            <th>"来源"</th>
                            <th>"样本"</th>
                            <th>"数据依据 / 错误"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {if selection.total == 0 {
                            runtime_health_empty_row()
                        } else {
                            rows.into_any()
                        }}
                    </tbody>
                </table>
            </div>
            {move || {
                (total.get() > RUNTIME_HEALTH_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, RUNTIME_HEALTH_PAGE_SIZE)}
                })
            }}
        </div>
    }
    .into_any()
}

pub(super) fn runtime_health_row(row: &VenueOperationHealth) -> impl IntoView {
    let status_class = operation_status_class(row.status);
    let status = operation_status_label(row.status);
    let operation = runtime_operation_label(&row.operation);
    let operation_key = row.operation.clone();
    let venue = row.venue.clone();
    let source = row.source.clone();
    let freshness = freshness_label(row.freshness_ms);
    let sample = runtime_sample(row);
    let message = runtime_health_message(row);
    let evidence = runtime_evidence_summary(row);
    let title = runtime_evidence_detail(row);

    view! {
        <tr>
            <td><strong>{operation}</strong><em>{operation_key} " · " {venue}</em></td>
            <td><span class=status_class>{status}</span></td>
            <td>{source}<em>{freshness}</em></td>
            <td>{sample}</td>
            <td title=title>{message}<em>{evidence}</em></td>
        </tr>
    }
}

pub(super) fn runtime_stale_message(problem: Option<&ApiProblem>) -> Option<String> {
    problem.map(|problem| problem_message("运行状态验证刷新失败，显示上次结果", problem))
}

pub(super) fn runtime_health_empty_row() -> AnyView {
    view! {
        <tr>
            <td colspan="5" class="empty-cell">"当前交易所暂无运行状态数据依据；请先保存凭证或查看诊断页全局矩阵。"</td>
        </tr>
    }
    .into_any()
}
