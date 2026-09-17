use super::*;
use crate::panels::modules::execution::selection::ExecutionVenuePair;
use crate::panels::shared::execution_environment_label;
use shared_types::{
    normalized_venue_name, venue_family, ExecutionEnvironment, TradingStatusResponse,
    VenueRuntimeHealth, VenueRuntimeHealthSnapshot, VenueRuntimeOperationHealth,
};

pub(super) fn ticket_venue_health_panel(
    trading_state: &LoadState<TradingStatusResponse>,
    runtime_state: &LoadState<VenueRuntimeHealthSnapshot>,
    pair: Option<ExecutionVenuePair>,
) -> AnyView {
    let environment = trading_state.value().map(|status| status.environment);
    let environment_label = environment.map_or("环境未读取", execution_environment_label);
    let runtime_snapshot = runtime_state.value().cloned();
    let messages = state_messages(trading_state, runtime_state);
    let Some(pair) = pair else {
        return view! {
            <section class="settings-section" data-settings-table="ticket-venue-health">
                <div class="settings-summary-line">
                    <strong>"当前双腿运行态"</strong>
                    <span>{environment_label}</span>
                </div>
                <div class="empty-cell">"尚未从机会扫描或期货套利构建 HedgeTicket。"</div>
                <em class="settings-message">
                    "选择机会后显示 long/short venue 事实；每次提交仍由 HedgeTicket 双腿预检裁决。"
                </em>
                {messages}
            </section>
        }
        .into_any();
    };
    let summary = format!(
        "{} · {} · {}",
        environment_label, pair.pair, pair.opportunity_id
    );
    let long = runtime_snapshot
        .as_ref()
        .and_then(|snapshot| selected_venue_health(snapshot, &pair.long_venue));
    let short = runtime_snapshot
        .as_ref()
        .and_then(|snapshot| selected_venue_health(snapshot, &pair.short_venue));
    let authority = ticket_authority_message(environment);

    view! {
        <section class="settings-section" data-settings-table="ticket-venue-health">
            <div class="settings-summary-line">
                <strong>"当前双腿运行态"</strong>
                <span>{summary}</span>
            </div>
            <div class="table-wrap">
                <table class="clean-table settings-table runtime-health-table">
                    <thead>
                        <tr>
                            <th>"腿 / venue"</th>
                            <th>"私有 API"</th>
                            <th>"读取挂单"</th>
                            <th>"下单"</th>
                            <th>"撤单"</th>
                            <th>"订单流"</th>
                            <th>"终态"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {ticket_leg_row("Long", pair.long_venue, long)}
                        {ticket_leg_row("Short", pair.short_venue, short)}
                    </tbody>
                </table>
            </div>
            <em class="settings-message">{authority}</em>
            {messages}
        </section>
    }
    .into_any()
}

fn ticket_leg_row(
    leg: &'static str,
    venue: String,
    health: Option<&VenueRuntimeHealth>,
) -> impl IntoView {
    view! {
        <tr>
            <td><strong>{leg}</strong><em>{venue}</em></td>
            {ticket_operation_cell(health.and_then(|row| row.private_rest.as_ref()))}
            {ticket_operation_cell(health.and_then(|row| row.open_orders.as_ref()))}
            {ticket_operation_cell(health.and_then(|row| row.place_order.as_ref()))}
            {ticket_operation_cell(health.and_then(|row| row.cancel_order.as_ref()))}
            {ticket_operation_cell(health.and_then(|row| row.order_stream.as_ref()))}
            {ticket_operation_cell(health.and_then(|row| row.finality.as_ref()))}
        </tr>
    }
}

fn ticket_operation_cell(operation: Option<&VenueRuntimeOperationHealth>) -> AnyView {
    let Some(operation) = operation else {
        return view! { <td><span class="status-pill status-unknown">"无证据"</span></td> }
            .into_any();
    };
    let title = ticket_operation_title(operation);
    view! {
        <td title=title>
            <span class=status_pill_class(operation.status)>{status_label(operation.status)}</span>
            <small class="runtime-cell-meta">
                {if operation.currently_usable { "当前可用" } else { "不可用/未证明" }}
            </small>
        </td>
    }
    .into_any()
}

fn ticket_operation_title(operation: &VenueRuntimeOperationHealth) -> String {
    let mut evidence = vec![
        format!("source {}", operation.source),
        format!("checked_at {}", operation.observed_at_ms),
    ];
    if let Some(request_id) = operation.request_id.as_deref() {
        evidence.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = operation.retry_after_ms {
        evidence.push(format!("retry_after_ms {retry_after_ms}"));
    }
    if let Some(problem) = operation.problem.as_ref() {
        evidence.push(problem_message("运行态阻断", problem));
    } else if let Some(error) = operation.last_error.as_deref() {
        evidence.push(format!("error {error}"));
    }
    evidence.join(" · ")
}

fn selected_venue_health<'a>(
    snapshot: &'a VenueRuntimeHealthSnapshot,
    selected: &str,
) -> Option<&'a VenueRuntimeHealth> {
    let selected_exact = normalized_venue_name(selected);
    let selected_family = normalized_venue_name(venue_family(selected));
    snapshot
        .venues
        .iter()
        .find(|row| normalized_venue_name(&row.venue) == selected_exact)
        .or_else(|| {
            snapshot
                .venues
                .iter()
                .find(|row| normalized_venue_name(venue_family(&row.venue)) == selected_family)
        })
}

fn ticket_authority_message(environment: Option<ExecutionEnvironment>) -> &'static str {
    match environment {
        Some(ExecutionEnvironment::Paper) => {
            "模拟环境：实盘权限列仅作诊断；HedgeTicket 双腿预检仍会校验市场、账户与运行态。"
        }
        Some(ExecutionEnvironment::Live) => {
            "实盘环境：此矩阵不授予提交权限；HedgeTicket 双腿预检是最终提交权威。"
        }
        None => "环境尚未读取；禁止从本矩阵推断可提交性，等待 HedgeTicket 双腿预检。",
    }
}

fn state_messages(
    trading_state: &LoadState<TradingStatusResponse>,
    runtime_state: &LoadState<VenueRuntimeHealthSnapshot>,
) -> AnyView {
    let mut messages = Vec::new();
    if let Some(problem) = trading_state.problem() {
        messages.push(problem_message("执行环境刷新失败", problem));
    } else if matches!(trading_state, LoadState::Loading) {
        messages.push("正在读取执行环境".to_owned());
    }
    if let Some(problem) = runtime_state.problem() {
        messages.push(problem_message("双腿运行态刷新失败", problem));
    } else if matches!(runtime_state, LoadState::Loading) {
        messages.push("正在读取双腿运行态".to_owned());
    }
    messages
        .into_iter()
        .map(|message| view! { <em class="settings-message is-error">{message}</em> })
        .collect_view()
        .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_health_prefers_exact_then_family_match() {
        let snapshot = VenueRuntimeHealthSnapshot {
            venues: vec![
                VenueRuntimeHealth::new("hyperliquid", 10),
                VenueRuntimeHealth::new("hyperliquid:xyz", 11),
            ],
            ..Default::default()
        };

        assert_eq!(
            selected_venue_health(&snapshot, "HYPERLIQUID:XYZ").map(|row| row.venue.as_str()),
            Some("hyperliquid:xyz")
        );
        assert_eq!(
            selected_venue_health(&snapshot, "hyperliquid:other").map(|row| row.venue.as_str()),
            Some("hyperliquid")
        );
    }

    #[test]
    fn paper_and_live_copy_preserve_ticket_preflight_authority() {
        let paper = ticket_authority_message(Some(ExecutionEnvironment::Paper));
        let live = ticket_authority_message(Some(ExecutionEnvironment::Live));

        assert!(paper.contains("模拟环境"));
        assert!(paper.contains("HedgeTicket 双腿预检"));
        assert!(live.contains("实盘环境"));
        assert!(live.contains("最终提交权威"));
    }
}
