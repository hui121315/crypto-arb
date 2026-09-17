use super::*;

pub(super) fn ws_panel(
    state: LoadState<shared_types::ExchangeWsVenuesResponse>,
    venue_id: &str,
) -> AnyView {
    let (response, stale_problem) = match state {
        LoadState::Ready(response) => (response, None),
        LoadState::Stale { value, problem } => (value, Some(problem)),
        LoadState::Error(problem) => return problem_cell("读取 WS 能力失败", &problem),
        LoadState::Loading => {
            return view! { <div class="empty-cell">"正在读取 WS 能力"</div> }.into_any();
        }
    };
    let stale_message = ws_stale_message(stale_problem.as_ref());
    let Some(row) = ws_venue_from_response(response, venue_id) else {
        return view! {
            <>
                {stale_message.map(|message| view! {
                    <em class="settings-message is-error">{message}</em>
                })}
                <div class="empty-cell">"未找到该交易所 WS 能力"</div>
            </>
        }
        .into_any();
    };
    let private_endpoint = row.private_endpoint.unwrap_or_else(|| "-".to_string());
    let trade_endpoint = row.trade_endpoint.unwrap_or_else(|| "-".to_string());
    view! {
        <>
            {stale_message.map(|message| view! {
                <em class="settings-message is-error">{message}</em>
            })}
            <div class="ws-venue-panel">
                <div class="ws-venue-head">
                    <div>
                        <strong>{row.label}</strong>
                        <em>{row.note}</em>
                        <em>"静态 WS 能力，不代表当前连接、权限或订单状态流已验证。"</em>
                    </div>
                    <span class="num">{row.docs.len()} " docs"</span>
                </div>
                <div class="ws-endpoints">
                    <span>{ws_endpoint_label("行情 WS", &row.public_endpoint)}</span>
                    <span>{ws_endpoint_label("通知 WS", &private_endpoint)}</span>
                    <span>{ws_endpoint_label("交易 WS", &trade_endpoint)}</span>
                </div>
                <div class="ws-cap-grid">
                    {ws_chip("账户流", row.account_stream, false)}
                    {ws_chip("仓位流", row.position_stream, false)}
                    {ws_chip("成交流", row.fill_stream, false)}
                    {ws_chip("订单流", row.order_stream, false)}
                    {ws_chip("下单", row.place_order, true)}
                    {ws_chip("撤单", row.cancel_order, true)}
                    {ws_chip("平仓", row.close_position, true)}
                    {ws_chip("状态", row.order_status, false)}
                </div>
            </div>
        </>
    }
    .into_any()
}

pub(super) fn ws_endpoint_label(role: &str, endpoint: &str) -> String {
    format!("{role} · {endpoint}")
}

pub(super) fn ws_stale_message(problem: Option<&ApiProblem>) -> Option<String> {
    problem.map(|problem| problem_message("WS 能力刷新失败，显示上次结果", problem))
}
