use super::super::*;
use super::fixtures::{api_problem, arbitrage_snapshot_problem};

#[test]
fn execution_mode_title_uses_backend_environment() {
    let status = TradingStatusResponse {
        adapter: "live_router".into(),
        environment: ExecutionEnvironment::Live,
        open_order_count: 2,
        risk: shared_types::TradingRiskStatus {
            live_trading_enabled: true,
            kill_switch_active: false,
            max_order_notional: 1.0,
            max_open_orders: 10,
            max_hedge_imbalance_pct: 0.05,
            liquidation_warn_pct: 0.2,
            liquidation_danger_pct: 0.1,
            allowed_exchanges: Vec::new(),
            allowed_symbols: Vec::new(),
            protected_positions: Vec::new(),
            auto_profit_close: Default::default(),
        },
        ws_channels: shared_types::TradingWsChannels {
            orders: "orders".into(),
            execution: "execution".into(),
            risk_alerts: "risk-alerts".into(),
        },
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        mutation: None,
    };

    let title = execution_mode_title(Some(&status), None);

    assert!(title.contains("后端执行环境：实盘"));
    assert!(title.contains("adapter：live_router"));
    assert_eq!(execution_mode_class(Some(&status), None), "slot live");

    let mut paper = status;
    paper.environment = ExecutionEnvironment::Paper;
    assert_eq!(execution_mode_class(Some(&paper), None), "slot paper");
    assert_eq!(execution_mode_class(None, None), "slot unknown");
}

#[test]
fn scan_status_label_distinguishes_loading_and_degraded_market() {
    assert_eq!(
        scan_status_label(&LoadState::<()>::Loading, None, None, &[]),
        "加载"
    );

    let mut meta = OpportunityCountMeta {
        status: OpportunityEnvelopeStatus::Fresh,
        ..Default::default()
    };
    meta.scan.market_data_problem_count = 2;

    assert_eq!(
        scan_status_label(&LoadState::Ready(()), Some(&meta), None, &[]),
        "降级"
    );
    assert!(scan_degraded(&LoadState::Ready(()), Some(&meta), None, &[]));
}

#[test]
fn scan_status_surfaces_snapshot_freshness_instead_of_network_latency() {
    let meta = OpportunityCountMeta {
        status: OpportunityEnvelopeStatus::Fresh,
        freshness_ms: Some(2_450),
        ..Default::default()
    };

    assert_eq!(
        scan_status_label(&LoadState::Ready(()), Some(&meta), None, &[]),
        "2.5s"
    );
}

#[test]
fn scan_status_title_includes_stream_problem() {
    let problem = ApiProblem::new("RATE_LIMITED", "rate limited")
        .with_source("market-data-cache")
        .with_retry_after_ms(Some(2_000));
    let title = scan_status_title(&LoadState::Ready(()), None, Some(&problem), &[]);

    assert!(title.contains("实时流问题"));
    assert!(title.contains("market-data-cache"));
    assert!(title.contains("retry 2000ms"));
}

#[test]
fn scan_status_background_task_problem_degrades() {
    let problem = arbitrage_snapshot_problem();
    let problems = vec![problem];

    assert_eq!(
        scan_status_label(&LoadState::Ready(()), None, None, &problems),
        "异常"
    );
    assert_eq!(
        scan_slot_class(&LoadState::Ready(()), None, None, &problems),
        "slot degraded"
    );
    let title = scan_status_title(&LoadState::Ready(()), None, None, &problems);
    assert!(title.contains("后台任务异常"));
    assert!(title.contains("arbitrage_snapshot"));
    assert!(title.contains("连续失败"));
}

#[test]
fn scan_status_fresh_old_snapshot_is_stale() {
    let meta = OpportunityCountMeta {
        status: OpportunityEnvelopeStatus::Fresh,
        freshness_ms: Some(65_000),
        ..Default::default()
    };

    assert_eq!(
        scan_status_label(&LoadState::Ready(()), Some(&meta), None, &[]),
        "过期"
    );
    assert!(scan_degraded(&LoadState::Ready(()), Some(&meta), None, &[]));
    assert_eq!(
        scan_slot_class(&LoadState::Ready(()), Some(&meta), None, &[]),
        "slot degraded"
    );
}

#[test]
fn risk_slot_treats_missing_status_as_unknown_degraded() {
    assert_eq!(risk_status_label(None), "未知");
    assert_eq!(risk_dot_class(None), "slot-dot red");
    assert_eq!(risk_slot_class(None), "slot clickable degraded");
}

#[test]
fn risk_slot_keeps_ok_green_only_for_explicit_ok() {
    assert_eq!(risk_status_label(Some(RiskStatusSlot::Ok)), "OK");
    assert_eq!(risk_dot_class(Some(RiskStatusSlot::Ok)), "slot-dot ok");
    assert_eq!(risk_slot_class(Some(RiskStatusSlot::Ok)), "slot clickable");
}

#[test]
fn scalar_slots_treat_missing_system_health_as_unknown_degraded() {
    assert_eq!(order_elapsed_label(None), "未知");
    assert_eq!(order_elapsed_slot_class(None), "slot degraded");
    assert_eq!(order_elapsed_dot_class(None), "slot-dot red");
    assert_eq!(net_delta_label(None), "未知");
    assert_eq!(net_delta_slot_class(None), "slot degraded");
    assert_eq!(net_delta_dot_class(None), "slot-dot red");
    assert_eq!(funding_label(None), "未知");
    assert_eq!(funding_slot_class(None), "slot degraded");
    assert_eq!(funding_dot_class(None), "slot-dot red");
}

#[test]
fn scalar_slots_surface_system_health_problem_on_cold_error() {
    let problem = api_problem();

    assert_eq!(
        order_elapsed_label_with_problem(None, Some(&problem)),
        "错误"
    );
    assert_eq!(risk_status_label_with_problem(None, Some(&problem)), "错误");
    assert_eq!(net_delta_label_with_problem(None, Some(&problem)), "错误");
    assert_eq!(funding_label_with_problem(None, Some(&problem)), "错误");
    assert_eq!(
        order_elapsed_slot_class_with_problem(None, Some(&problem)),
        "slot degraded"
    );
    assert_eq!(
        net_delta_dot_class_with_problem(None, Some(&problem)),
        "slot-dot red"
    );
}

#[test]
fn scalar_slot_titles_append_system_health_problem_to_stale_values() {
    let problem = api_problem();
    let funding = FundingSlot {
        symbol: "ETH".into(),
        venue: "okx".into(),
        minutes_to_settle: 8,
        estimated_outflow_usd: 0.0,
    };

    let order_title = order_elapsed_title_with_problem(Some(120), Some(&problem));
    let risk_title = risk_title_with_problem(Some(RiskStatusSlot::Ok), Some(&problem));
    let delta_title = net_delta_title_with_problem(Some((10.0, 1.0)), Some(&problem));
    let funding_title = funding_title_with_problem(Some(&funding), Some(&problem));

    assert!(order_title.contains("订单最终结果耗时"));
    assert!(risk_title.contains("风险状态：OK"));
    assert!(delta_title.contains("净 Delta"));
    assert!(funding_title.contains("ETH @ okx"));
    assert!(order_title.contains("request_id req-1"));
    assert!(risk_title.contains("retry 2000ms"));
    assert!(delta_title.contains("RATE_LIMITED"));
    assert!(funding_title.contains("slow down"));
}

#[test]
fn scalar_slots_keep_existing_thresholds_for_real_values() {
    let funding = FundingSlot {
        symbol: "BTC".into(),
        venue: "binance".into(),
        minutes_to_settle: 6,
        estimated_outflow_usd: 1.0,
    };

    assert_eq!(order_elapsed_slot_class(Some(199)), "slot");
    assert_eq!(order_elapsed_slot_class(Some(201)), "slot degraded");
    assert_eq!(net_delta_slot_class(Some((10.0, 4.9))), "slot");
    assert_eq!(net_delta_slot_class(Some((10.0, 5.1))), "slot degraded");
    assert_eq!(funding_slot_class(Some(&funding)), "slot");
}

#[test]
fn paper_mode_treats_absent_live_order_and_funding_samples_as_neutral() {
    assert_eq!(
        order_elapsed_label_for_environment(None, None, Some(ExecutionEnvironment::Paper)),
        "无实盘订单"
    );
    assert_eq!(
        order_elapsed_slot_class_for_environment(None, None, Some(ExecutionEnvironment::Paper)),
        "slot"
    );
    assert_eq!(
        funding_label_for_environment(None, None, Some(ExecutionEnvironment::Paper)),
        "无模拟持仓"
    );
    assert_eq!(
        funding_slot_class_for_environment(None, None, Some(ExecutionEnvironment::Paper)),
        "slot"
    );
}

#[test]
fn paper_funding_slot_shows_market_window_without_claiming_live_debit() {
    let funding = FundingSlot {
        symbol: "BTC".into(),
        venue: "binance".into(),
        minutes_to_settle: 12,
        estimated_outflow_usd: 1.0,
    };

    assert_eq!(
        funding_label_for_environment(Some(&funding), None, Some(ExecutionEnvironment::Paper)),
        "12m"
    );
    let title =
        funding_title_for_environment(Some(&funding), None, Some(ExecutionEnvironment::Paper));
    assert!(title.contains("市场结算窗口"));
    assert!(title.contains("不产生真实账户扣款"));
}

#[test]
fn net_delta_does_not_render_negative_zero() {
    assert_eq!(net_delta_label(Some((-0.001, -0.001))), "$0 (+0.0%)");
    assert!(net_delta_title(Some((-0.001, -0.001))).contains("净 Delta：$0"));
}

#[test]
fn scalar_slot_titles_explain_source_and_thresholds() {
    let funding = FundingSlot {
        symbol: "ETH".into(),
        venue: "okx".into(),
        minutes_to_settle: 4,
        estimated_outflow_usd: 12.4,
    };

    assert!(risk_title(None).contains("SystemHealth"));
    assert!(risk_title(Some(RiskStatusSlot::Block)).contains("高风险动作应被阻断"));
    assert!(net_delta_title(None).contains("等待 SystemHealth"));
    assert!(net_delta_title(Some((120.0, 5.2))).contains("阈值 ±5%"));
    assert!(net_delta_title(Some((120.0, 5.2))).contains("netDeltaUsd"));
    assert!(funding_title(Some(&funding)).contains("ETH @ okx"));
    assert!(funding_title(Some(&funding)).contains("<5m 标红"));
    assert!(funding_title(None).contains("SystemHealth.nextFunding"));
}

#[test]
fn visible_runtime_slot_labels_do_not_use_ambiguous_api_ws_rtt_copy() {
    assert_eq!(API_SLOT_LABEL, "交易接口");
    assert_eq!(WS_SLOT_LABEL, "账户连接");
    assert_eq!(MARKET_DATA_SLOT_LABEL, "行情数据");
    assert_eq!(APP_WS_SLOT_LABEL, "后台连接");
    assert_eq!(ORDER_ELAPSED_SLOT_LABEL, "订单最终结果");
    assert_ne!(API_SLOT_LABEL, "API");
    assert_ne!(WS_SLOT_LABEL, "WS");
    assert_ne!(ORDER_ELAPSED_SLOT_LABEL, "RTT");
}
