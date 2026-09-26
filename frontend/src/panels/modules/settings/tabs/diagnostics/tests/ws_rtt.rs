use super::*;

#[test]
fn private_ws_status_rows_keep_only_private_ws_kinds() {
    let rows = vec![
        operation_row(
            "binance",
            "private_ws_order_stream",
            VenueOperationStatus::Ok,
        ),
        operation_row("okx", "order_write", VenueOperationStatus::Ok),
        operation_row("bybit", "private_ws_session", VenueOperationStatus::Warn),
    ];

    let filtered = private_ws_status_rows(rows);

    assert_eq!(filtered.len(), 2);
    assert!(filtered
        .iter()
        .all(|row| VenueOperationKind::parse(&row.operation).is_private_ws_status_row()));
    // 最差优先：Warn 排在 Ok 前。
    assert_eq!(filtered[0].venue, "bybit");
}

#[test]
fn private_ws_status_rows_drop_unsupported_like_top_bar_slot() {
    let mut unsupported = operation_row(
        "gate",
        "private_ws_order_stream",
        VenueOperationStatus::Unsupported,
    );
    unsupported.supported = Some(false);

    let filtered = private_ws_status_rows(vec![unsupported]);

    assert!(filtered.is_empty());
}

#[test]
fn ws_rtt_summary_counts_channels_and_disconnected() {
    let rows = vec![
        operation_row(
            "binance",
            "private_ws_order_stream",
            VenueOperationStatus::Ok,
        ),
        operation_row("bybit", "private_ws_session", VenueOperationStatus::Blocked),
    ];

    assert_eq!(ws_rtt_summary(&rows), "channels 2 · 非正常 1");
    assert_eq!(ws_rtt_summary(&[]), "无私有 WS 数据依据 · 顶部 WS 显示缺失");
}

#[test]
fn app_ws_rows_and_scope_summary_keep_lag_counts_visible() {
    let private = vec![operation_row(
        "binance",
        "private_ws_session",
        VenueOperationStatus::Ok,
    )];
    let mut orders = operation_row("app", "app_ws_broadcast:orders", VenueOperationStatus::Warn);
    orders.requested = Some(2);
    orders.rows = Some(7);
    let mut system = operation_row("app", "app_ws_broadcast:system", VenueOperationStatus::Ok);
    system.requested = Some(1);
    system.rows = Some(3);
    let all = vec![orders, system, private[0].clone()];

    let app = app_ws_broadcast_rows(&all);

    assert_eq!(app.len(), 2);
    assert!(app.iter().all(|row| {
        VenueOperationKind::parse(&row.operation) == VenueOperationKind::AppWsBroadcast
    }));
    let summary = ws_scope_summary(&private, &app);
    assert!(summary.contains("AppWS channels 2"));
    assert!(summary.contains("lag 3"));
    assert!(summary.contains("丢帧 10"));
    assert!(summary.contains("近期异常 1"));
}

#[test]
fn ws_rtt_explanation_separates_order_elapsed_from_transport_rtt() {
    let copy = ws_rtt_explanation_copy();

    assert!(copy.contains("订单最终结果耗时"));
    assert!(copy.contains("PrivateWS"));
    assert!(copy.contains("AppWS"));
    assert!(copy.contains("不使用 subscriber count 代理"));
    assert!(copy.contains("app_ws_broadcast"));
    assert!(copy.contains("lag/丢帧累计"));
    assert!(copy.contains("不代表网络 RTT"));
    assert!(copy.contains("缺行时不推断网络 RTT"));
    assert!(copy.contains("HTTP RTT"));
    assert!(copy.contains("send() 到响应头返回"));
    assert!(copy.contains("不含 HostGate/singleflight/RateLimiter 本地等待和响应体处理"));
    assert!(copy.contains("http_rest operation-health"));
    assert!(copy.contains("latencyMs/latencyP95Ms"));
}

#[test]
fn ws_freshness_label_formats_seconds_or_dash() {
    let mut row = operation_row(
        "binance",
        "private_ws_order_stream",
        VenueOperationStatus::Ok,
    );
    row.freshness_ms = Some(2_500);
    assert_eq!(ws_freshness_label(&row), "2.5s 前");

    row.freshness_ms = None;
    assert_eq!(ws_freshness_label(&row), "—");
}
