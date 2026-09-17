use super::super::*;
use super::fixtures::*;
use crate::api::ws::{WsChannelState, WsStatus};

#[test]
fn private_ws_unknown_and_missing_evidence_degrade() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![operation_row(
            "okx",
            "private_ws_order_stream",
            VenueOperationStatus::Unknown,
        )],
        1_000,
    );

    assert_eq!(ws_label(Some(&snapshot)), "0可用/1配置");
    assert!(ws_degraded(Some(&snapshot), None));
    assert_eq!(ws_label(None), "无证据");
    assert!(ws_degraded(None, None));
}

#[test]
fn private_ws_ignores_unknown_operations() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![operation_row(
            "okx",
            "private_ws_made_up",
            VenueOperationStatus::Ok,
        )],
        1_000,
    );

    assert_eq!(ws_label(Some(&snapshot)), "无证据");
    assert!(ws_degraded(Some(&snapshot), None));
}

#[test]
fn private_ws_problem_overrides_healthy_label() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![operation_row(
            "okx",
            "private_ws_order_stream",
            VenueOperationStatus::Ok,
        )],
        1_000,
    );
    let problem = api_problem();

    assert_eq!(ws_label(Some(&snapshot)), "1可用/1配置");
    assert_eq!(
        ws_label_with_problem(Some(&snapshot), Some(&problem)),
        "异常"
    );
    assert!(ws_degraded(Some(&snapshot), Some(&problem)));
    assert!(ws_title(Some(&snapshot), Some(&problem)).contains("request_id req-1"));
}

#[test]
fn private_ws_title_prioritizes_status_then_order_stream() {
    let same_status = VenueOperationHealthSnapshot::new(
        vec![
            operation_row("okx", "private_ws_session", VenueOperationStatus::Warn),
            operation_row("okx", "private_ws_order_stream", VenueOperationStatus::Warn),
        ],
        1_000,
    );
    assert_eq!(
        most_severe_ws_operation(&same_status).map(|row| row.operation.as_str()),
        Some("private_ws_order_stream")
    );

    let worse_session = VenueOperationHealthSnapshot::new(
        vec![
            operation_row("okx", "private_ws_order_stream", VenueOperationStatus::Warn),
            operation_row("okx", "private_ws_session", VenueOperationStatus::Blocked),
        ],
        1_000,
    );
    assert_eq!(
        most_severe_ws_operation(&worse_session).map(|row| row.operation.as_str()),
        Some("private_ws_session")
    );
}

#[test]
fn private_ws_separates_configured_from_currently_usable() {
    let mut unconfigured =
        operation_row("gate", "private_ws_order_stream", VenueOperationStatus::Ok);
    unconfigured.configured = Some(false);
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_row(
                "okx",
                "private_ws_order_stream",
                VenueOperationStatus::Blocked,
            ),
            operation_row(
                "binance",
                "private_ws_order_stream",
                VenueOperationStatus::Ok,
            ),
            unconfigured,
        ],
        1_000,
    );

    assert_eq!(ws_label(Some(&snapshot)), "1可用/2配置");
    assert_eq!(ws_configured_count(&snapshot), 2);
    assert_eq!(ws_usable_count(&snapshot), 1);
}

#[test]
fn private_ws_unconfigured_optional_venue_does_not_degrade_healthy_configured_rows() {
    let configured = operation_row("okx", "private_ws_order_stream", VenueOperationStatus::Ok);
    let mut unconfigured = operation_row(
        "gate",
        "private_ws_order_stream",
        VenueOperationStatus::Blocked,
    );
    unconfigured.configured = Some(false);
    let snapshot = VenueOperationHealthSnapshot::new(vec![configured, unconfigured], 1_000);

    assert_eq!(ws_label(Some(&snapshot)), "1可用/1配置");
    assert!(!ws_degraded(Some(&snapshot), None));
    assert_eq!(
        most_severe_ws_operation(&snapshot).map(|row| row.venue.as_str()),
        Some("okx")
    );
}

#[test]
fn private_ws_live_without_any_configured_venue_stays_degraded() {
    let mut unconfigured = operation_row(
        "gate",
        "private_ws_order_stream",
        VenueOperationStatus::Blocked,
    );
    unconfigured.configured = Some(false);
    let snapshot = VenueOperationHealthSnapshot::new(vec![unconfigured], 1_000);

    assert_eq!(ws_label(Some(&snapshot)), "需配置凭证");
    assert!(ws_degraded(Some(&snapshot), None));
}

#[test]
fn private_ws_paper_mode_is_neutral_but_keeps_configuration_requirements() {
    let mut ws = operation_row(
        "binance",
        "private_ws_account_stream",
        VenueOperationStatus::Blocked,
    );
    ws.configured = Some(false);
    let mut credential = operation_row("binance", "private_read", VenueOperationStatus::Blocked);
    credential.configured = Some(false);
    credential.message =
        "需配置 Binance：API Key（BINANCE_API_KEY）、API Secret（BINANCE_API_SECRET）".into();
    let snapshot = VenueOperationHealthSnapshot::new(vec![ws, credential], 1_000);

    assert_eq!(
        ws_label_for_environment(Some(&snapshot), Some(ExecutionEnvironment::Paper)),
        "模拟无需"
    );
    assert!(!ws_degraded_for_environment(
        Some(&snapshot),
        None,
        Some(ExecutionEnvironment::Paper)
    ));
    let title = ws_title_for_environment(Some(&snapshot), None, Some(ExecutionEnvironment::Paper));
    assert!(title.contains("BINANCE_API_KEY"));
    assert!(title.contains("BINANCE_API_SECRET"));
}

#[test]
fn app_ws_uses_channel_state_not_subscriber_count() {
    let mut channel = WsChannelState::new("system");
    assert_eq!(app_ws_label(&channel, None), "已断开");
    assert!(app_ws_degraded(&channel, None));

    channel.status = WsStatus::Connected;
    assert_eq!(app_ws_label(&channel, None), "待订阅");
    assert!(app_ws_degraded(&channel, None));

    channel.subscribed = true;
    channel.message_count = 3;
    assert_eq!(app_ws_label(&channel, None), "已订阅");
    assert!(!app_ws_degraded(&channel, None));
    assert!(app_ws_title(&channel, None).contains("不使用 subscriber count 代理健康"));
    assert!(app_ws_title(&channel, None).contains("帧 3 · 错误 0"));

    channel.last_error = Some(api_problem());
    channel.problem_count = 1;
    channel.last_problem_at_ms = Some(42);
    assert_eq!(app_ws_label(&channel, None), "异常");
    assert!(app_ws_degraded(&channel, None));
    assert!(app_ws_title(&channel, None).contains("request_id req-1"));
    assert!(app_ws_title(&channel, None).contains("末次错误时间 42"));
}

#[test]
fn app_ws_surfaces_backend_broadcast_lag_counts() {
    let mut channel = WsChannelState::new("system");
    channel.status = WsStatus::Connected;
    channel.subscribed = true;
    let mut orders = operation_row("app", "app_ws_broadcast:orders", VenueOperationStatus::Warn);
    orders.requested = Some(2);
    orders.rows = Some(7);
    orders.source = "app_ws_hub".to_owned();
    let mut system = operation_row("app", "app_ws_broadcast:system", VenueOperationStatus::Ok);
    system.requested = Some(1);
    system.rows = Some(3);
    let snapshot = VenueOperationHealthSnapshot::new(vec![orders, system], 1_000);

    assert_eq!(app_ws_label(&channel, Some(&snapshot)), "丢帧 7");
    assert!(app_ws_degraded(&channel, Some(&snapshot)));
    let summary = app_ws_lag_summary(Some(&snapshot));
    assert_eq!(summary.channels, 2);
    assert_eq!(summary.lag_events, 3);
    assert_eq!(summary.skipped_messages, 10);
    assert_eq!(summary.recent_channels, 1);
    assert_eq!(summary.recent_skipped_messages, 7);
    let title = app_ws_title(&channel, Some(&snapshot));
    assert!(title.contains("lag 事件 3"));
    assert!(title.contains("累计丢帧 10"));
}

#[test]
fn market_data_uses_only_market_operation_rows() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_row("binance", "ws_ticker", VenueOperationStatus::Ok),
            operation_row("okx", "rest_orderbooks", VenueOperationStatus::Blocked),
            operation_row("gate", "order_write", VenueOperationStatus::Ok),
        ],
        1_000,
    );

    assert_eq!(market_data_label(Some(&snapshot), None), "1/1");
    assert!(!market_data_degraded(Some(&snapshot), None));
    assert!(market_data_title(Some(&snapshot), None)
        .contains("REST 冷启动/恢复 0/1 可用（不计实时核心）"));
}

#[test]
fn market_data_excludes_builder_extensions_and_disabled_watchlist_from_core_ratio() {
    let mut disabled_watchlist =
        operation_row("system", "watchlist_prewarm", VenueOperationStatus::Unknown);
    disabled_watchlist.configured = Some(false);
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_row("binance", "ws_ticker_snapshot", VenueOperationStatus::Ok),
            operation_row("binance", "rest_perp_tickers", VenueOperationStatus::Ok),
            operation_row(
                "hyperliquid:cash",
                "rest_instrument_specs",
                VenueOperationStatus::Blocked,
            ),
            operation_row(
                "hyperliquid:xyz",
                "rest_instrument_specs",
                VenueOperationStatus::Ok,
            ),
            operation_row(
                "okx",
                "rest_index_compositions",
                VenueOperationStatus::Blocked,
            ),
            disabled_watchlist,
        ],
        1_000,
    );

    assert_eq!(market_data_label(Some(&snapshot), None), "1/1");
    assert!(!market_data_degraded(Some(&snapshot), None));
    let title = market_data_title(Some(&snapshot), None);
    assert!(title.contains("REST 冷启动/恢复 1/1 可用"));
    assert!(title.contains("扩展市场 1/2 可用"));
    assert!(title.contains("指数成分验证 0/1 可用"));
    assert!(title.contains("watchlist 预热未启用"));
}

#[test]
fn market_data_counts_spot_ws_snapshot_as_realtime_core() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_row("binance", "ws_spot_snapshot", VenueOperationStatus::Ok),
            operation_row("bitget", "ws_spot_snapshot", VenueOperationStatus::Blocked),
            operation_row("binance", "rest_spot_ticks", VenueOperationStatus::Ok),
        ],
        1_000,
    );

    assert_eq!(market_data_label(Some(&snapshot), None), "1/2");
    assert!(market_data_degraded(Some(&snapshot), None));
    let title = market_data_title(Some(&snapshot), None);
    assert!(title.contains("ws_spot_snapshot"));
    assert!(title.contains("REST 冷启动/恢复 1/1 可用"));
}
