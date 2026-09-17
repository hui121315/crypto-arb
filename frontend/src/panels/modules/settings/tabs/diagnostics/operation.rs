use super::*;

pub(super) fn operation_health_panel(
    state: LoadState<VenueOperationHealthSnapshot>,
    table: &TableRuntimeHandle<VenueOperationHealth>,
    query: RwSignal<String>,
    status_filter: RwSignal<HealthStatusFilter>,
) -> AnyView {
    let (snapshot, snapshot_problem) = match state {
        LoadState::Ready(snapshot) => (snapshot, None),
        LoadState::Stale {
            value: snapshot,
            problem,
        } => (snapshot, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取运行状态失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取运行状态"</div> }.into_any();
        }
    };
    let attention = snapshot.attention_count;
    let source_count = snapshot.rows.len();
    let normalized_query = normalized_search_query(&query.get());
    let selected_filter = status_filter.get();
    let row_count = table.total.get();
    let runtime = table.runtime.get();
    let total = table.total;
    let current_page = table.current_page;
    let visible_rows = runtime
        .rows
        .into_iter()
        .map(operation_health_row)
        .collect_view();
    view! {
        <>
            <div class="settings-summary-line">
                <strong>"运行态矩阵"</strong>
                <span>{operation_health_summary(attention, row_count, source_count, &normalized_query, selected_filter)}</span>
            </div>
            {snapshot_problem.map(|problem| view! {
                <em class="settings-message is-error">
                    {operation_snapshot_problem_message(&problem)}
                </em>
            })}
            <div class="api-base-editor adapter-editor">
                <label>
                    <span>"搜索状态"</span>
                    <input
                        prop:value=move || query.get()
                        placeholder="交易所 / 操作 / 来源 / 问题"
                        on:input=move |ev| query.set(event_target_value(&ev))
                    />
                </label>
                <label>
                    <span>"状态过滤"</span>
                    <select
                        prop:value=move || status_filter.get().as_key()
                        on:change=move |ev| {
                            status_filter.set(HealthStatusFilter::from_key(&event_target_value(&ev)));
                        }
                    >
                        <option value="all">"全部"</option>
                        <option value="attention">"需关注"</option>
                        <option value="blocked">"阻断"</option>
                        <option value="warn">"观察"</option>
                        <option value="unknown">"待验证"</option>
                        <option value="ok">"正常"</option>
                        <option value="unsupported">"不支持"</option>
                    </select>
                </label>
                <button
                    type="button"
                    class="row-action"
                    disabled=move || query.get().trim().is_empty()
                    on:click=move |_| query.set(String::new())
                >
                    "清空"
                </button>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"操作"</th>
                            <th>"状态"</th>
                            <th>"配置状态"</th>
                            <th>"能力支持"</th>
                            <th>"当前可用"</th>
                            <th>"来源"</th>
                            <th>"样本"</th>
                            <th>"说明"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {
                            if row_count == 0 {
                                operation_health_empty_row(source_count, &normalized_query, selected_filter)
                            } else {
                                visible_rows.into_any()
                            }
                        }
                    </tbody>
                </table>
            </div>
            {move || {
                (total.get() > HEALTH_PAGE_SIZE).then(|| view! {
                    {page_controls(total, current_page, HEALTH_PAGE_SIZE)}
                })
            }}
        </>
    }
    .into_any()
}

pub(super) fn operation_health_row(row: VenueOperationHealth) -> impl IntoView {
    let sample = operation_sample(&row);
    let message = operation_health_message(&row);
    let operation = operation_health_operation_label(&row);
    let title = operation_health_title(&row);
    let configured = operation_configured_label(&row);
    let capability = operation_capability_label(&row);
    let usable = operation_usable_label(&row);
    view! {
        <tr>
            <td>{row.venue}</td>
            <td>{operation}</td>
            <td><span class=status_pill_class(row.status)>{status_label(row.status)}</span></td>
            <td>{configured}</td>
            <td>{capability}</td>
            <td>{usable}</td>
            <td>{row.source}</td>
            <td>{sample}</td>
            <td title=title><em>{message}</em></td>
        </tr>
    }
}

pub(super) fn operation_health_operation_label(row: &VenueOperationHealth) -> String {
    let kind = VenueOperationKind::parse(&row.operation);
    format!(
        "{} / {} · {}",
        kind.class().label_zh(),
        kind.label_zh(),
        row.operation
    )
}

pub(super) fn prioritized_operation_health_rows(
    mut rows: Vec<VenueOperationHealth>,
) -> Vec<VenueOperationHealth> {
    rows.sort_by(operation_health_order);
    rows
}

pub(super) fn filtered_operation_health_rows(
    rows: Vec<VenueOperationHealth>,
    query: &str,
    status_filter: HealthStatusFilter,
) -> Vec<VenueOperationHealth> {
    let query = normalized_search_query(query);
    prioritized_operation_health_rows(rows)
        .into_iter()
        .filter(|row| status_filter.matches(row.status) && operation_health_matches(row, &query))
        .collect()
}

pub(super) fn operation_health_matches(row: &VenueOperationHealth, query: &str) -> bool {
    query.is_empty()
        || searchable_operation_health_text(row)
            .to_ascii_lowercase()
            .contains(query)
}

pub(super) fn operation_health_summary(
    attention: usize,
    visible: usize,
    total: usize,
    query: &str,
    status_filter: HealthStatusFilter,
) -> String {
    if query.is_empty() && status_filter == HealthStatusFilter::All {
        return format!("最差优先 · 需关注 {attention} / {total} 条");
    }
    format!(
        "最差优先 · {} · 匹配 {visible} / {total} 条 · 需关注 {attention}",
        status_filter.label()
    )
}

pub(super) fn operation_health_empty_row(
    total: usize,
    query: &str,
    status_filter: HealthStatusFilter,
) -> AnyView {
    if total == 0 {
        return empty_table_row(9, "暂无运行态状态");
    }
    if query.is_empty() && status_filter == HealthStatusFilter::All {
        return empty_table_row(9, "暂无运行态状态");
    }
    empty_table_row(9, "没有匹配的运行态状态")
}

pub(super) fn operation_health_order(
    left: &VenueOperationHealth,
    right: &VenueOperationHealth,
) -> Ordering {
    status_rank(right.status)
        .cmp(&status_rank(left.status))
        .then_with(|| left.venue.cmp(&right.venue))
        .then_with(|| left.operation.cmp(&right.operation))
}

pub(super) fn status_rank(status: VenueOperationStatus) -> u8 {
    match status {
        VenueOperationStatus::Blocked => 5,
        VenueOperationStatus::Warn => 4,
        VenueOperationStatus::Unknown => 3,
        VenueOperationStatus::Unsupported => 2,
        VenueOperationStatus::Ok => 1,
    }
}
