use super::*;
use crate::panels::modules::futures::data::FuturesSummary;

#[test]
fn build_accepts_verified_partial_rows_but_not_failed_or_stale_snapshots() {
    let mut meta = OpportunityCountMeta::default();
    meta.status = shared_types::OpportunityEnvelopeStatus::Fresh;
    assert!(futures_snapshot_usable(&LoadState::Ready(()), &meta));
    assert!(!futures_snapshot_usable(&LoadState::Loading, &meta));
    let issue = ApiProblem::new("VENUE_MISSING", "one venue is unavailable");
    let partial = LoadState::Stale {
        value: (),
        problem: issue.clone(),
    };
    meta.status = shared_types::OpportunityEnvelopeStatus::Degraded;
    meta.partial_failures.push(issue);
    assert!(futures_snapshot_usable(&partial, &meta));
    assert!(!futures_snapshot_usable(
        &LoadState::Stale {
            value: (),
            problem: ApiProblem::new("TIMEOUT", "read failed"),
        },
        &meta
    ));
    meta.status = shared_types::OpportunityEnvelopeStatus::Stale;
    assert!(!futures_snapshot_usable(&partial, &meta));
}

#[test]
fn visible_problem_prefers_stream_problem() {
    let stream = ApiProblem::new("WS_PAYLOAD_DECODE", "ws failed").with_source("frontend-ws");
    let list = ApiProblem::new("UPSTREAM_HTTP", "list failed").with_source("rest");

    let visible = visible_futures_problem(Some(stream), Some(list));

    assert_eq!(
        visible.as_ref().map(|problem| problem.code.as_str()),
        Some("WS_PAYLOAD_DECODE")
    );
}

#[test]
fn visible_problem_uses_list_problem_without_stream_problem() {
    let list = ApiProblem::new("UPSTREAM_HTTP", "list failed").with_source("rest");

    let visible = visible_futures_problem(None, Some(list));

    assert_eq!(
        visible.as_ref().map(|problem| problem.code.as_str()),
        Some("UPSTREAM_HTTP")
    );
}

#[test]
fn best_profit_kpi_detail_includes_verified_net_floor() {
    let summary = FuturesSummary {
        candidates: 2,
        filtered_candidates: 2,
        executable_candidates: 1,
        best_monitor_net_bps: Some(5.0),
        best_monitor_pair: "BTC-USDT".into(),
        best_monitor_profit_detail: "费率证据 2/2 · 费后边际 +0.050%".into(),
        best_monitor_preview_ready: true,
    };

    assert_eq!(
        kpis::best_profit_kpi_detail(&summary),
        "BTC-USDT · 费率证据 2/2 · 费后边际 +0.050% · 可进入构建预检"
    );
}

#[test]
fn best_profit_kpi_detail_never_promotes_observation_only_profit() {
    let summary = FuturesSummary {
        candidates: 1,
        filtered_candidates: 1,
        executable_candidates: 0,
        best_monitor_net_bps: None,
        best_monitor_pair: "-".into(),
        best_monitor_profit_detail: "-".into(),
        best_monitor_preview_ready: false,
    };

    assert_eq!(
        kpis::best_profit_kpi_detail(&summary),
        "没有通过策略与收益校验的候选"
    );
}

#[test]
fn best_profit_kpi_detail_names_positive_but_blocked_monitoring() {
    let summary = FuturesSummary {
        candidates: 1,
        filtered_candidates: 1,
        executable_candidates: 0,
        best_monitor_net_bps: Some(1379.3),
        best_monitor_pair: "COTI".into(),
        best_monitor_profit_detail: "费率证据 2/2 · 费后边际 +13.793%".into(),
        best_monitor_preview_ready: false,
    };

    assert_eq!(
        kpis::best_profit_kpi_detail(&summary),
        "COTI · 费率证据 2/2 · 费后边际 +13.793% · 仅监控，当前不可构建"
    );
}

#[test]
fn feed_status_prioritizes_degraded_snapshot_over_live_channel() {
    let mut channel = WsChannelState::new("arbitrage");
    channel.status = WsStatus::Connected;
    channel.subscribed = true;
    let stale = LoadState::Stale {
        value: (),
        problem: ApiProblem::new("OPPORTUNITY_ENVELOPE_STALE", "partial market data"),
    };

    assert_eq!(
        futures_feed_status(&stale, &channel, false, false),
        FuturesFeedStatus {
            label: "候选快照陈旧",
            tone: "is-degraded",
        }
    );
}

#[test]
fn feed_status_reports_healthy_live_stream() {
    let mut channel = WsChannelState::new("arbitrage");
    channel.status = WsStatus::Connected;
    channel.subscribed = true;

    assert_eq!(
        futures_feed_status(&LoadState::Ready(()), &channel, false, false),
        FuturesFeedStatus {
            label: "候选实时更新",
            tone: "is-live",
        }
    );
}

#[test]
fn feed_status_names_disconnected_source_without_generic_empty_copy() {
    let channel = WsChannelState::new("arbitrage");

    assert_eq!(
        futures_feed_status(&LoadState::Ready(()), &channel, false, false),
        FuturesFeedStatus {
            label: "候选源未就绪",
            tone: "is-warming",
        }
    );
}
