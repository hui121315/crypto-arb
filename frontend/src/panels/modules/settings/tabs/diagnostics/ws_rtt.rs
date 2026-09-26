use super::*;

/// 顶部 WS/RTT 口径解释面板：按 venue 列出私有 WS 状态行（与顶部 WS slot
/// 同一数据来源 `VenueOperationHealthSnapshot`），并把订单耗时和 HTTP RTT
/// 的来源切开。
pub(super) fn ws_rtt_explain_panel(state: LoadState<VenueOperationHealthSnapshot>) -> AnyView {
    let (snapshot, snapshot_problem) = match state {
        LoadState::Ready(snapshot) => (snapshot, None),
        LoadState::Stale {
            value: snapshot,
            problem,
        } => (snapshot, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取连接状态与耗时失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取连接状态与耗时"</div> }.into_any();
        }
    };
    let app_rows = app_ws_broadcast_rows(&snapshot.rows);
    let rows = private_ws_status_rows(snapshot.rows);
    let summary = ws_scope_summary(&rows, &app_rows);
    let body = if rows.is_empty() && app_rows.is_empty() {
        empty_table_row(5, "尚未读到账户连接和后台连接的状态")
    } else {
        app_rows
            .into_iter()
            .chain(rows)
            .map(ws_rtt_row)
            .collect_view()
            .into_any()
    };
    view! {
        <>
            <div class="settings-summary-line">
                <strong>"连接状态与耗时"</strong>
                <span>{summary}</span>
            </div>
            {snapshot_problem.map(|problem| view! {
                <em class="settings-message is-error">
                    {operation_snapshot_problem_message(&problem)}
                </em>
            })}
            <em>{ws_rtt_explanation_copy()}</em>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"来源"</th>
                            <th>"频道"</th>
                            <th>"状态"</th>
                            <th>"距上次更新"</th>
                            <th>"说明"</th>
                        </tr>
                    </thead>
                    <tbody>{body}</tbody>
                </table>
            </div>
        </>
    }
    .into_any()
}

pub(super) fn ws_rtt_explanation_copy() -> &'static str {
    "账户连接反映交易所的账户和订单推送是否正常。后台连接反映浏览器是否已连接并订阅成功，以及推送积压和漏收次数，不能只靠连接人数判断。\
    订单耗时是从创建订单到最后一次更新的时间，不是网络延迟。\
    交易所请求耗时从发出请求算到收到响应头，不包含本机排队、限速等待和读取完整结果的时间；没有测量数据时不估算。"
}

pub(super) fn app_ws_broadcast_rows(rows: &[VenueOperationHealth]) -> Vec<VenueOperationHealth> {
    let mut rows = rows
        .iter()
        .filter(|row| {
            VenueOperationKind::parse(&row.operation) == VenueOperationKind::AppWsBroadcast
        })
        .cloned()
        .collect::<Vec<_>>();
    rows.sort_by(operation_health_order);
    rows
}

/// 与后端 `system_health::slots::is_private_ws_row` 同口径：顶部 WS slot
/// 只统计这些行。
pub(super) fn private_ws_status_rows(rows: Vec<VenueOperationHealth>) -> Vec<VenueOperationHealth> {
    let mut rows = rows
        .into_iter()
        .filter(|row| {
            row.supported != Some(false)
                && VenueOperationKind::parse(&row.operation).is_private_ws_status_row()
        })
        .collect::<Vec<_>>();
    rows.sort_by(operation_health_order);
    rows
}

pub(super) fn ws_rtt_summary(rows: &[VenueOperationHealth]) -> String {
    let channels = rows.len();
    let disconnected = rows
        .iter()
        .filter(|row| row.status != VenueOperationStatus::Ok)
        .count();
    if channels == 0 {
        return "账户推送状态待确认".to_owned();
    }
    format!("{channels} 个频道 · {disconnected} 个待检查")
}

pub(super) fn ws_scope_summary(
    private_rows: &[VenueOperationHealth],
    app_rows: &[VenueOperationHealth],
) -> String {
    let app_lag_events = app_rows
        .iter()
        .filter_map(|row| row.requested)
        .fold(0_u64, u64::saturating_add);
    let app_skipped = app_rows
        .iter()
        .filter_map(|row| row.rows)
        .fold(0_u64, u64::saturating_add);
    let app_recent = app_rows
        .iter()
        .filter(|row| row.status != VenueOperationStatus::Ok)
        .count();
    format!(
        "账户连接 {} · 后台推送 {} 个频道 · 积压 {} 次 · 漏收 {} 条 · 近期异常 {}",
        ws_rtt_summary(private_rows),
        app_rows.len(),
        app_lag_events,
        app_skipped,
        app_recent
    )
}

fn ws_rtt_row(row: VenueOperationHealth) -> impl IntoView {
    let freshness = ws_freshness_label(&row);
    let message = operation_health_message(&row);
    let title = operation_health_title(&row);
    let kind = VenueOperationKind::parse(&row.operation);
    let channel = format!("{} · {}", kind.label_zh(), row.operation);
    view! {
        <tr>
            <td>{row.venue}</td>
            <td>{channel}</td>
            <td><span class=status_pill_class(row.status)>{status_label(row.status)}</span></td>
            <td>{freshness}</td>
            <td><em title=title>{message}</em></td>
        </tr>
    }
}

pub(super) fn ws_freshness_label(row: &VenueOperationHealth) -> String {
    match row.freshness_ms {
        Some(freshness_ms) if freshness_ms >= 0 => {
            format!("{:.1}s 前", freshness_ms as f64 / 1000.0)
        }
        _ => "—".to_owned(),
    }
}
