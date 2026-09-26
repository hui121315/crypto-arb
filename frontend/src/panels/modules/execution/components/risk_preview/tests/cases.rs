use super::*;
use crate::state::load_state::LoadState;
use shared_types::{ApiProblem, ExchangeProblem};

#[test]
fn depth_health_badge_counts_fresh_legs() {
    let fresh = market_health(MarketDataQuality::Fresh);
    let limited = market_health(MarketDataQuality::RateLimited);

    assert_eq!(
        depth_health_badge_from_pair(Some(&fresh), Some(&limited)).as_deref(),
        Some("盘口 1/2")
    );
}

#[test]
fn execution_preview_problem_context_risk_error() {
    let state = LoadState::<ExecutionPreview>::Error(
        ApiProblem::new("RATE_LIMITED", "preview slow")
            .with_source("preview-rest")
            .with_status(429)
            .with_request_id(Some("preview-req-1".into()))
            .with_retry_after_ms(Some(2_000)),
    );

    let notice = preview_load_notice_text(&state).unwrap_or_default();

    assert!(notice.contains("预览失败：preview slow"));
    assert!(notice.contains("code RATE_LIMITED"));
    assert!(notice.contains("source preview-rest"));
    assert!(notice.contains("HTTP 429"));
    assert!(notice.contains("request_id preview-req-1"));
    assert!(notice.contains("retry 2000ms"));
}

#[test]
fn execution_preview_problem_context_risk_stale() {
    let state = LoadState::Stale {
        value: ready_preview(None),
        problem: ApiProblem::new("TIMEOUT", "preview timeout")
            .with_source("preview-cache")
            .with_status(504)
            .with_request_id(Some("preview-req-2".into()))
            .with_retry_after_ms(Some(3_000)),
    };

    let notice = preview_load_notice_text(&state).unwrap_or_default();

    assert!(notice.contains("预览已失效：preview timeout"));
    assert!(notice.contains("code TIMEOUT"));
    assert!(notice.contains("source preview-cache"));
    assert!(notice.contains("HTTP 504"));
    assert!(notice.contains("request_id preview-req-2"));
    assert!(notice.contains("retry 3000ms"));
}

#[test]
fn execution_preview_problem_context_risk_note_is_not_duplicated() {
    let mut preview = ready_preview(None);
    let text = "Preview 请求失败：slow · code RATE_LIMITED".to_owned();
    preview.risk.note.clone_from(&text);
    preview.risk.blockers = vec![text.clone(), "余额不足".into()];

    let rendered = risk_note_text(&preview);

    assert_eq!(rendered.matches(&text).count(), 1);
    assert!(rendered.contains("余额不足"));
}

#[test]
fn preview_load_notice_hides_ready_and_empty_pending() {
    assert!(preview_load_notice_text(&LoadState::Ready(ready_preview(None))).is_none());
    assert!(preview_load_notice_text(&LoadState::<ExecutionPreview>::Loading).is_none());
}

#[test]
fn preflight_summary_counts_blocked_outcomes() {
    let guards = vec![
        guard("余额", HedgePreflightStatus::Passed),
        guard("持仓", HedgePreflightStatus::Blocked),
    ];

    assert_eq!(
        preflight_health_summary(&guards).as_deref(),
        Some("交易检查 1/2 · 1 阻断")
    );
    let detail = guard_detail(&guards[1]);
    assert!(detail.contains("范围 binance"));
    assert!(detail.contains("账户 unified"));
    assert!(detail.contains("操作 保证金余额,持仓读取,挂单读取,私有WS"));
    assert!(detail.contains("检查 1"));
    assert!(detail.contains("余额 binance USDT 可用 $100 总额 $120 占用 $20"));
    assert!(detail.contains("余额健康 binance USDT account_cache"));
    assert!(detail.contains("新鲜度 500ms"));
    assert!(detail.contains("重试 2000ms"));
    assert!(detail.contains("BALANCE_READ_DEGRADED rate limited"));
    assert!(detail.contains("字段 binance USDT available MISSING · account_balance_runtime"));
    assert!(detail.contains("问题 MARGIN_BALANCE_MISSING missing margin HTTP 200"));
    assert!(detail.contains("来源 balance_cache"));
    assert!(detail.contains("错误 insufficient"));
}

#[test]
fn preflight_problem_line_surfaces_structured_http_context() {
    let problem = ExchangeProblem::new("binance", "rest_orderbooks", "rate limited")
        .with_path("/fapi/v1/depth")
        .with_symbol("BTCUSDT")
        .with_latency_ms(Some(37))
        .with_request_id(Some("req-pr-an-http".into()))
        .with_retry_after_ms(Some(2_000))
        .to_api_problem("UPSTREAM_HTTP");

    let line = preflight_problem_line(&problem);

    assert!(line.contains("rest_orderbooks"));
    assert!(line.contains("BTCUSDT"));
    assert!(line.contains("/fapi/v1/depth"));
    assert!(line.contains("HTTP耗时 37ms"));
    assert!(line.contains("req req-pr-an-http"));
    assert!(line.contains("retry 2000ms"));
}

#[test]
fn check_item_state_reflects_readiness_and_guard_outcomes() {
    assert_eq!(
        readiness_state(PreviewReadiness::Pending),
        Some(CheckItemState::Missing)
    );
    assert_eq!(
        readiness_state(PreviewReadiness::Stale),
        Some(CheckItemState::Stale)
    );
    assert_eq!(
        readiness_state(PreviewReadiness::Error),
        Some(CheckItemState::Error)
    );
    assert_eq!(readiness_state(PreviewReadiness::Ready), None);
    assert_eq!(liq_distance_state(None), CheckItemState::Missing);
    assert_eq!(liq_distance_state(Some(12.0)), CheckItemState::Block);
    assert_eq!(
        guard_state(&guard("余额", HedgePreflightStatus::Passed)),
        CheckItemState::Ok
    );
    assert_eq!(
        guard_state(&guard("余额", HedgePreflightStatus::Blocked)),
        CheckItemState::Block
    );
    assert_eq!(
        guard_state(&guard("余额", HedgePreflightStatus::Failed)),
        CheckItemState::Error
    );
    assert_eq!(
        guard_state(&guard("余额", HedgePreflightStatus::Skipped)),
        CheckItemState::Unknown
    );
}

#[test]
fn decision_summary_follows_final_submit_readiness() {
    let mut preview = ready_preview(None);
    assert_eq!(decision_text(&preview), "通过");
    assert_eq!(decision_state(&preview), CheckItemState::Ok);

    preview.risk.blockers = vec!["等待按需 WS 深度首帧".into()];

    assert_eq!(decision_text(&preview), "阻断");
    assert_eq!(decision_state(&preview), CheckItemState::Block);
}

#[test]
fn one_cycle_cost_summary_surfaces_blocking_shortfall() {
    let cost = PreviewOneCycleCost {
        gross_edge_bps: 4.0,
        total_cost_bps: 15.0,
        open_fee_bps: 3.5,
        close_fee_bps: 3.5,
        open_slippage_bps: 4.0,
        close_slippage_bps: 4.0,
        funding_window_mismatch_evidence: None,
        profitability_status: shared_types::ProfitabilityEvidenceStatus::Missing,
        funding_history_health: None,
        funding_history_sample_count: 0,
        net_bps: -11.0,
        covers_round_trip_cost: false,
    };

    assert_eq!(one_cycle_cost_line(&cost), "单次 -0.110% · 阻断执行");
    assert!(pct_from_bps(4.0).contains("+0.040%"));
}

#[test]
fn missing_one_cycle_cost_hides_zero_cost_values() {
    let preview = ready_preview(None);

    assert_eq!(net_edge_text(&preview), "缺成本数据依据");
    assert_eq!(
        cost_money(&preview, preview.total_cost_usd(), "待成本"),
        "缺成本数据依据"
    );
    assert_eq!(cost_breakdown_text(&preview), "缺成本数据依据");
    assert_eq!(positive_net_edge_state(&preview), CheckItemState::Missing);
    assert_eq!(cost_edge_state(&preview), CheckItemState::Missing);
    assert_eq!(
        ready_nonnegative_state(&preview, preview.open_cost_usd),
        CheckItemState::Missing
    );
}

#[test]
fn small_execution_costs_do_not_round_to_zero() {
    assert_eq!(money(0.125), "$0.125");
    assert_eq!(money(0.0042), "$0.0042");
    assert_eq!(money(-0.0042), "-$0.0042");
    assert_eq!(money(0.0), "$0");
}

#[test]
fn profit_evidence_surfaces_net_floor_and_fee_ids() {
    let mut preview = ready_preview(Some(PreviewOneCycleCost {
        gross_edge_bps: 4.0,
        total_cost_bps: 15.0,
        open_fee_bps: 3.5,
        close_fee_bps: 3.5,
        open_slippage_bps: 4.0,
        close_slippage_bps: 4.0,
        funding_window_mismatch_evidence: Some(PreviewFundingWindowEvidence {
            yield_basis: shared_types::YieldBasis::NativeSettlement,
            buffer_bps: 1.25,
            long_next_settlement_ms: 1_000,
            short_next_settlement_ms: 2_000,
        }),
        profitability_status: shared_types::ProfitabilityEvidenceStatus::Verified,
        funding_history_health: Some(shared_types::FundingDiffSampleHealth::Ok),
        funding_history_sample_count: 9,
        net_bps: -11.0,
        covers_round_trip_cost: false,
    }));
    preview.profit_evidence = PreviewProfitEvidence {
        one_cycle_net_bps: -11.0,
        fee_evidence_ids: vec![
            "fee:hyperliquid:perp:vip0".into(),
            "fee:gate:perp:vip0".into(),
        ],
        fee_evidence_complete: true,
    };

    assert_eq!(
        profit_evidence_summary(&preview),
        "费率数据依据 2/2 · 单次费后 -0.110%"
    );
    assert!(one_cycle_cost_detail(&preview).contains("盈利数据依据 完整 / 历史 健康 / 9 样本"));
    let detail = profit_evidence_detail(&preview);
    assert!(detail.contains("列表单次费后净利 -0.110%"));
    assert!(detail.contains("fee:hyperliquid:perp:vip0"));
    assert!(detail.contains("单次费后净利下限非正，阻断执行"));
    let cost_detail = one_cycle_cost_detail(&preview);
    assert!(cost_detail.contains("native_settlement"));
    assert!(cost_detail.contains("缓冲 +0.013%"));
    assert!(cost_detail.contains("多腿 1000ms / 空腿 2000ms"));
    assert!(cost_detail.contains("净利不足，阻断执行"));
}
