use super::*;

pub(super) fn venue_runtime_health_panel(state: LoadState<VenueRuntimeHealthSnapshot>) -> AnyView {
    let snapshot = match state {
        LoadState::Ready(snapshot) => snapshot,
        LoadState::Stale { value, problem } => {
            return view! {
                <>
                    {runtime_matrix(&value)}
                    <em class="settings-message is-error">
                        {operation_snapshot_problem_message(&problem)}
                    </em>
                </>
            }
            .into_any();
        }
        LoadState::Error(problem) => return problem_cell("读取交易运行状态失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取交易运行状态"</div> }.into_any();
        }
    };
    runtime_matrix(&snapshot)
}

fn runtime_matrix(snapshot: &VenueRuntimeHealthSnapshot) -> AnyView {
    let summary = format!(
        "{} 家交易所 · 当前可用 {} / {} 项 · 需关注 {} 项",
        snapshot.venue_count,
        snapshot.currently_usable_count,
        snapshot.operation_count,
        snapshot.attention_count
    );
    let rows = snapshot
        .venues
        .iter()
        .map(|venue| {
            view! {
                <tr>
                    <td><strong>{venue.venue.clone()}</strong></td>
                    {runtime_operation_cell(venue.public_rest.as_ref())}
                    {runtime_operation_cell(venue.private_rest.as_ref())}
                    {runtime_operation_cell(venue.private_ws.as_ref())}
                    {runtime_operation_cell(venue.place_order.as_ref())}
                    {runtime_operation_cell(venue.cancel_order.as_ref())}
                    {runtime_operation_cell(venue.order_stream.as_ref())}
                    {runtime_operation_cell(venue.finality.as_ref())}
                </tr>
            }
        })
        .collect_view();
    view! {
        <section class="settings-section" data-settings-table="venue-runtime-health">
            <div class="settings-summary-line">
                <strong>"交易运行状态中心"</strong>
                <span>{summary}</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table">
                    <thead>
                        <tr>
                            <th>"交易所"</th>
                            <th>"公共 API"</th>
                            <th>"私有 API"</th>
                            <th>"私有 WS"</th>
                            <th>"下单"</th>
                            <th>"撤单"</th>
                            <th>"订单流"</th>
                            <th>"最终结果"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {if snapshot.venues.is_empty() {
                            empty_table_row(8, "暂无交易运行状态")
                        } else {
                            rows.into_any()
                        }}
                    </tbody>
                </table>
            </div>
        </section>
    }
    .into_any()
}

fn runtime_operation_cell(operation: Option<&VenueRuntimeOperationHealth>) -> AnyView {
    let Some(operation) = operation else {
        return view! { <td><span class="status-pill status-unknown">"无数据依据"</span></td> }
            .into_any();
    };
    let title = runtime_operation_title(operation);
    let sample = runtime_sample(operation);
    view! {
        <td title=title>
            <span class=status_pill_class(operation.status)>{status_label(operation.status)}</span>
            <small class="runtime-cell-meta">{sample}</small>
        </td>
    }
    .into_any()
}

fn runtime_sample(operation: &VenueRuntimeOperationHealth) -> String {
    match (operation.rows, operation.requested) {
        (Some(rows), Some(requested)) => format!("{rows}/{requested}"),
        (Some(rows), None) => format!("{rows} 条"),
        (None, Some(requested)) => format!("0/{requested}"),
        (None, None) if operation.currently_usable => "实时".to_owned(),
        (None, None) => "待验证".to_owned(),
    }
}

fn runtime_operation_title(operation: &VenueRuntimeOperationHealth) -> String {
    let mut parts = vec![format!("source {}", operation.source)];
    if let Some(freshness_ms) = operation.freshness_ms {
        parts.push(format!("freshness {freshness_ms}ms"));
    }
    if let Some(latency_ms) = operation.latency_ms {
        parts.push(format!("request latency {latency_ms}ms"));
    }
    if let Some(latency_p95_ms) = operation.latency_p95_ms {
        parts.push(format!("p95 {latency_p95_ms}ms"));
    }
    if let Some(problem) = operation.problem.as_ref() {
        parts.push(problem_context(problem));
    } else if let Some(error) = operation.last_error.as_ref() {
        parts.push(error.clone());
    }
    parts.join(" · ")
}

fn problem_context(problem: &shared_types::ApiProblem) -> String {
    let mut text = format!("{}: {}", problem.code, problem.message);
    if let Some(request_id) = problem.request_id.as_deref() {
        text.push_str(&format!(" · request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        text.push_str(&format!(" · retry {retry_after_ms}ms"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_health_title_keeps_latency_problem_and_request_context() {
        let row = runtime_row();
        let title = runtime_operation_title(&row);
        assert!(title.contains("request latency 12ms"));
        assert!(title.contains("p95 20ms"));
        assert!(title.contains("RATE_LIMITED"));
        assert!(title.contains("request_id req-eg"));
        assert_eq!(runtime_sample(&row), "3/4");
    }

    fn runtime_row() -> VenueRuntimeOperationHealth {
        let legacy = VenueOperationHealth {
            venue: "okx".into(),
            operation: "private_read".into(),
            status: VenueOperationStatus::Warn,
            source: "http_metrics".into(),
            message: "rate limited".into(),
            supported: Some(true),
            configured: Some(true),
            requested: Some(4),
            rows: Some(3),
            freshness_ms: Some(5),
            retry_after_ms: Some(1_000),
            latency_ms: Some(12),
            latency_p95_ms: Some(20),
            error: Some("rate limited".into()),
            evidence: None,
            problem: Some(
                shared_types::ApiProblem::new("RATE_LIMITED", "rate limited")
                    .with_request_id(Some("req-eg".into())),
            ),
            observed_at_ms: 10,
        };
        VenueRuntimeOperationHealth::from_operation_health(
            shared_types::VenueRuntimeOperation::PrivateRest,
            &legacy,
        )
    }
}
