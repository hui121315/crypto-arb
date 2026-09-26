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

    assert_eq!(ws_rtt_summary(&rows), "2 个频道 · 1 个待检查");
    assert_eq!(ws_rtt_summary(&[]), "账户推送状态待确认");
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
    assert!(summary.contains("后台推送 2 个频道"));
    assert!(summary.contains("积压 3 次"));
    assert!(summary.contains("漏收 10 条"));
    assert!(summary.contains("近期异常 1"));
}

#[test]
fn ws_rtt_explanation_separates_order_elapsed_from_transport_rtt() {
    let copy = ws_rtt_explanation_copy();

    assert!(copy.contains("订单耗时是从创建订单到最后一次更新"));
    assert!(copy.contains("账户连接"));
    assert!(copy.contains("后台连接"));
    assert!(copy.contains("不能只靠连接人数判断"));
    assert!(copy.contains("推送积压和漏收次数"));
    assert!(copy.contains("不是网络延迟"));
    assert!(copy.contains("没有测量数据时不估算"));
    assert!(copy.contains("发出请求算到收到响应头"));
    assert!(copy.contains("不包含本机排队、限速等待和读取完整结果"));
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
