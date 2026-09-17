use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    ApiProblem, OnchainBatchItemSnapshot, OnchainBatchSnapshot, OnchainComparisonQuality,
    OnchainComparisonSnapshot, OnchainCrossChainQuality, OnchainDexComparisonQuality,
    OnchainSpreadAlertMode,
};

use super::super::data::OnchainData;
use super::super::draft::OnchainConfigDraft;
use super::super::format::{
    cex_source_label, chain_label, direction_label, freshness_label, percent_label, provider_label,
    quality_label, quality_reason_label, quality_tone, retry_after_label, usd,
};

pub(in crate::panels::modules::onchain) fn batch_watchlist(
    draft: OnchainConfigDraft,
    data: OnchainData,
) -> impl IntoView {
    view! {
        <section class="onchain-batch-panel" aria-label="市场监控队列">
            {move || batch_state(draft, data, data.state.get())}
        </section>
    }
}

fn batch_state(
    draft: OnchainConfigDraft,
    data: OnchainData,
    state: LoadState<OnchainComparisonSnapshot>,
) -> AnyView {
    match state {
        LoadState::Loading => loading_state().into_any(),
        LoadState::Error(problem) => error_state(&problem).into_any(),
        LoadState::Ready(snapshot) => {
            batch_view(draft, data, snapshot.config.enabled, &snapshot.batch, None).into_any()
        }
        LoadState::Stale {
            value: snapshot,
            problem,
        } => batch_view(
            draft,
            data,
            snapshot.config.enabled,
            &snapshot.batch,
            Some(&problem),
        )
        .into_any(),
    }
}

fn batch_view(
    draft: OnchainConfigDraft,
    data: OnchainData,
    monitoring_enabled: bool,
    batch: &OnchainBatchSnapshot,
    stale_problem: Option<&ApiProblem>,
) -> impl IntoView {
    let count = batch.items.len();
    let capacity = batch.max_items;
    let summary = batch_summary(batch);
    let has_items = !batch.items.is_empty();
    let header_class = if has_items {
        "onchain-panel-header onchain-batch-header"
    } else {
        "onchain-panel-header onchain-batch-header is-empty"
    };
    let panel_copy = if has_items {
        "共享 Provider 配额公平轮询；CEX 最优价由 WS 高频投影，完整深度仅在构建时读取"
    } else if monitoring_enabled {
        "从配置栏将当前链上/CEX 组合加入队列。"
    } else {
        "先启用套利监控，再从配置栏加入需要持续观察的组合。"
    };
    let body = if has_items {
        batch_table(draft, data, &batch.items)
    } else {
        empty_batch_state(monitoring_enabled)
    };
    view! {
        <header class=header_class>
            <div>
                <h2>"市场监控"</h2>
                <p>{panel_copy}</p>
            </div>
            <div class="onchain-batch-summary">
                <span><small>"监控"</small><strong class="num">{format!("{count}/{capacity}")}</strong></span>
                <span class=summary.opportunity_class><small>"费后机会"</small><strong>{summary.opportunities}</strong></span>
                <span class=summary.issue_class><small>"异常"</small><strong>{summary.issues}</strong></span>
                <span class=summary.sweep_class><small>"预计全轮"</small><strong>{summary.sweep}</strong></span>
            </div>
        </header>
        <div class="onchain-batch-body">
            {stale_problem.map(stale_notice)}
            {body}
        </div>
    }
}

fn empty_batch_state(monitoring_enabled: bool) -> AnyView {
    let (title, detail) = if monitoring_enabled {
        (
            "监控队列为空",
            "当前比较继续更新；需要并行观察时，可将已核验组合加入队列。",
        )
    } else {
        (
            "监控尚未启动",
            "启用套利监控后，当前比较与批量队列才会读取双源快照。",
        )
    };
    view! {
        <div class="onchain-batch-empty">
            <strong>{title}</strong>
            <span>{detail}</span>
        </div>
    }
    .into_any()
}

struct BatchSummary {
    opportunities: String,
    opportunity_class: &'static str,
    issues: String,
    issue_class: &'static str,
    sweep: String,
    sweep_class: &'static str,
}

fn batch_summary(batch: &OnchainBatchSnapshot) -> BatchSummary {
    if batch.items.is_empty() {
        return BatchSummary {
            opportunities: "0".to_owned(),
            opportunity_class: "is-unresolved",
            issues: "0".to_owned(),
            issue_class: "is-unresolved",
            sweep: "未运行".to_owned(),
            sweep_class: "is-unresolved",
        };
    }

    let opportunity_count = batch
        .items
        .iter()
        .filter(|item| verified_net_opportunity(item))
        .count();
    let issue_count = batch
        .items
        .iter()
        .filter(|item| item_has_issue(item))
        .count();
    let sweep_ready = batch.estimated_sweep_ms > 0;
    BatchSummary {
        opportunities: opportunity_count.to_string(),
        opportunity_class: if opportunity_count > 0 {
            "has-opportunity"
        } else {
            ""
        },
        issues: issue_count.to_string(),
        issue_class: if issue_count > 0 { "has-attention" } else { "" },
        sweep: if sweep_ready {
            freshness_label(Some(batch.estimated_sweep_ms))
        } else {
            "待调度".to_owned()
        },
        sweep_class: if sweep_ready { "" } else { "is-unresolved" },
    }
}

fn verified_net_opportunity(item: &OnchainBatchItemSnapshot) -> bool {
    item.config.spread_alert.mode == OnchainSpreadAlertMode::VerifiedNet
        && (cex_verified_net_opportunity(item)
            || dex_verified_net_opportunity(item)
            || cross_chain_verified_net_opportunity(item))
}

fn cex_verified_net_opportunity(item: &OnchainBatchItemSnapshot) -> bool {
    item.quality == OnchainComparisonQuality::Fresh
        && item.best_net_spread_bps.is_some_and(|spread| {
            spread > 0.0 && spread >= item.config.spread_alert.min_net_spread_bps.max(0.0)
        })
}

fn dex_verified_net_opportunity(item: &OnchainBatchItemSnapshot) -> bool {
    matches!(
        item.dex_quality,
        OnchainDexComparisonQuality::Fresh | OnchainDexComparisonQuality::EvidencePending
    ) && item.best_dex_net_return_bps.is_some_and(|spread| {
        spread > 0.0 && spread >= item.config.spread_alert.min_net_spread_bps.max(0.0)
    })
}

fn cross_chain_verified_net_opportunity(item: &OnchainBatchItemSnapshot) -> bool {
    item.cross_chain_quality == OnchainCrossChainQuality::Fresh
        && item.best_cross_chain_net_return_bps.is_some_and(|spread| {
            spread > 0.0 && spread >= item.config.spread_alert.min_net_spread_bps.max(0.0)
        })
}

fn item_has_issue(item: &OnchainBatchItemSnapshot) -> bool {
    matches!(
        item.quality,
        OnchainComparisonQuality::Stale
            | OnchainComparisonQuality::MappingInvalid
            | OnchainComparisonQuality::UpstreamUnavailable
    ) || (item.config.spread_alert.mode == OnchainSpreadAlertMode::VerifiedNet
        && matches!(
            item.quality,
            OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
        ))
        || matches!(
            item.dex_quality,
            OnchainDexComparisonQuality::Stale
                | OnchainDexComparisonQuality::EvidencePending
                | OnchainDexComparisonQuality::UpstreamUnavailable
        )
        || matches!(
            item.cross_chain_quality,
            OnchainCrossChainQuality::Stale
                | OnchainCrossChainQuality::PeerMissing
                | OnchainCrossChainQuality::EvidencePending
                | OnchainCrossChainQuality::UpstreamUnavailable
        )
}

fn batch_table(
    draft: OnchainConfigDraft,
    data: OnchainData,
    items: &[OnchainBatchItemSnapshot],
) -> AnyView {
    view! {
        <div class="workbench-table-wrap onchain-batch-table-wrap">
            <table class="onchain-batch-table" data-table-budget="bounded-small">
                <thead><tr>
                    <th>"市场"</th><th>"价差 / 提醒"</th><th>"规模 / 时效"</th>
                    <th>"状态"</th><th aria-label="操作"></th>
                </tr></thead>
                <tbody>{items.iter().map(|item| batch_row(draft, data, item)).collect_view()}</tbody>
            </table>
        </div>
    }
    .into_any()
}

fn batch_row(
    draft: OnchainConfigDraft,
    data: OnchainData,
    item: &OnchainBatchItemSnapshot,
) -> impl IntoView {
    let focus_config = item.config.clone();
    let remove_id = item.item_id.clone();
    let pair = format!("{}/{}", item.config.base_token, item.config.quote_token,);
    let market = format!("{} · {}", item.config.cex_venue, item.config.cex_symbol);
    let source = format!(
        "{} · {}",
        chain_label(&item.config.chain),
        provider_label(&item.config.provider)
    );
    let cex_source = cex_source_label(&item.cex_source);
    let direction = item
        .best_direction
        .map(direction_label)
        .unwrap_or("等待双向报价");
    let raw_observation = batch_raw_observation(item);
    let edge_bps = if raw_observation {
        item.best_gross_spread_bps
    } else {
        item.best_net_spread_bps
    };
    let edge = edge_bps.map_or_else(|| "未知".to_owned(), percent_label);
    let conversion_label = item
        .quote_conversion
        .as_ref()
        .map(|conversion| format!(" · WS 换算 {}", conversion.symbol));
    let edge_basis = if raw_observation {
        format!("原始观察 · {direction}")
    } else {
        format!(
            "{direction}{}",
            conversion_label.as_deref().unwrap_or_default()
        )
    };
    let alert_rule = batch_alert_rule_label(item);
    let alert_rule_class = batch_alert_rule_class(item);
    let dex_edge = dex_edge_label(item);
    let cross_chain_edge = cross_chain_edge_label(item);
    let depth = if raw_observation {
        "未换算".to_owned()
    } else {
        item.observable_notional_usd
            .map_or_else(|| "未知".to_owned(), usd)
    };
    let freshness = format!(
        "{} / {}",
        freshness_label(item.onchain_freshness_ms),
        freshness_label(item.cex_freshness_ms)
    );
    let latency = item
        .onchain_latency_ms
        .map_or_else(|| "请求耗时未知".to_owned(), |ms| format!("请求 {ms}ms"));
    let problem_details = batch_problem_details(item);
    let problem_count = problem_details.len();
    let quality_reason = quality_reason_label(item.quality);
    let provider_retry = item
        .provider_retry_after_ms
        .map(|delay| format!("Provider · {}", retry_after_label(delay)));
    let tone = quality_tone(item.quality);
    view! {
        <tr>
            <td data-label="市场">
                <div class="onchain-cell-stack onchain-batch-market">
                    <strong>{pair}</strong>
                    <span>{market}</span>
                    <small>{format!("{source} · {cex_source}")}</small>
                </div>
            </td>
            <td data-label="价差 / 提醒" class="num">
                <div class="onchain-cell-stack">
                    <strong>{edge}</strong>
                    <small>{edge_basis}</small>
                    {dex_edge.map(|label| view! { <small class="onchain-batch-dex-edge">{label}</small> })}
                    {cross_chain_edge.map(|label| view! { <small class="onchain-batch-cross-chain-edge">{label}</small> })}
                    <small class=alert_rule_class>{alert_rule}</small>
                </div>
            </td>
            <td data-label="规模 / 时效">
                <div class="onchain-cell-stack onchain-batch-market-data">
                    <strong class="num">{depth}</strong>
                    <span class="num">{freshness}</span>
                    <small>{latency}</small>
                </div>
            </td>
            <td data-label="状态" class="onchain-batch-status-cell">
                <div class="onchain-cell-stack onchain-batch-status">
                    <span class=format!("onchain-state-badge {tone}")>{quality_label(item.quality)}</span>
                    <small>{quality_reason}</small>
                    {provider_retry.map(|retry| view! { <small>{retry}</small> })}
                    {(!problem_details.is_empty()).then(|| view! {
                        <details class="onchain-batch-problem">
                            <summary>{format!("技术证据 · {problem_count} 条")}</summary>
                            <div>{problem_details.into_iter().map(|problem| view! { <p>{problem}</p> }).collect_view()}</div>
                        </details>
                    })}
                </div>
            </td>
            <td data-label="操作">
                <div class="onchain-row-actions">
                    <button type="button" title="载入为当前比较" on:click=move |_| focus_item(draft, data, &focus_config)>"载入"</button>
                    <button class="is-danger" type="button" title="移出批量监控" disabled=move || data.saving.get() on:click=move |_| data.remove_batch.run(remove_id.clone())>"移除"</button>
                </div>
            </td>
        </tr>
    }
}

fn batch_raw_observation(item: &OnchainBatchItemSnapshot) -> bool {
    item.config.spread_alert.mode == OnchainSpreadAlertMode::RawObservation
        || matches!(
            item.quality,
            OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
        )
}

fn dex_edge_label(item: &OnchainBatchItemSnapshot) -> Option<String> {
    item.config.dex_comparison.enabled.then(|| {
        let edge = item
            .best_dex_net_return_bps
            .map_or_else(|| "待报价".to_owned(), percent_label);
        let state = match item.dex_quality {
            OnchainDexComparisonQuality::Disabled => "未启用",
            OnchainDexComparisonQuality::Pending => "读取中",
            OnchainDexComparisonQuality::Fresh => "已核算",
            OnchainDexComparisonQuality::NoNetProfit => "无净收益",
            OnchainDexComparisonQuality::Stale => "已过期",
            OnchainDexComparisonQuality::DuplicateRoute => "同路由去重",
            OnchainDexComparisonQuality::EvidencePending => "仅监控",
            OnchainDexComparisonQuality::UpstreamUnavailable => "来源异常",
        };
        format!("DEX↔DEX {edge} · {state}")
    })
}

fn cross_chain_edge_label(item: &OnchainBatchItemSnapshot) -> Option<String> {
    item.config.cross_chain.enabled.then(|| {
        let edge = item
            .best_cross_chain_net_return_bps
            .map_or_else(|| "待报价".to_owned(), percent_label);
        let state = match item.cross_chain_quality {
            OnchainCrossChainQuality::Disabled => "未启用",
            OnchainCrossChainQuality::Pending => "读取中",
            OnchainCrossChainQuality::Fresh => "闭环已核算",
            OnchainCrossChainQuality::NoNetProfit => "无净收益",
            OnchainCrossChainQuality::Stale => "已过期",
            OnchainCrossChainQuality::PeerMissing => "目标缺失",
            OnchainCrossChainQuality::EvidencePending => "仅监控",
            OnchainCrossChainQuality::UpstreamUnavailable => "来源异常",
        };
        format!("跨链闭环 {edge} · {state}")
    })
}

fn batch_alert_rule_label(item: &OnchainBatchItemSnapshot) -> String {
    if !item.config.spread_alert.enabled {
        return "提醒关闭".to_owned();
    }
    let (mode, threshold) = match item.config.spread_alert.mode {
        OnchainSpreadAlertMode::VerifiedNet => (
            "费后净差",
            item.config.spread_alert.min_net_spread_bps.max(0.0),
        ),
        OnchainSpreadAlertMode::RawObservation => {
            ("原始价差", item.config.spread_alert.min_raw_spread_bps)
        }
    };
    format!(
        "提醒 · {mode} ≥ {} · {} 去重",
        threshold_percent_label(threshold),
        cooldown_label(item.config.spread_alert.cooldown_ms),
    )
}

fn threshold_percent_label(threshold_bps: f64) -> String {
    let value = format!("{:.3}", threshold_bps.max(0.0) / 100.0);
    let value = value.trim_end_matches('0').trim_end_matches('.');
    format!("{value}%")
}

fn batch_alert_rule_class(item: &OnchainBatchItemSnapshot) -> &'static str {
    if !item.config.spread_alert.enabled {
        "onchain-batch-alert-rule is-disabled"
    } else if batch_alert_threshold_met(item) {
        "onchain-batch-alert-rule is-triggered"
    } else if item.config.spread_alert.mode == OnchainSpreadAlertMode::RawObservation {
        "onchain-batch-alert-rule is-observation"
    } else {
        "onchain-batch-alert-rule"
    }
}

fn batch_alert_threshold_met(item: &OnchainBatchItemSnapshot) -> bool {
    if !item.config.spread_alert.enabled {
        return false;
    }
    match item.config.spread_alert.mode {
        OnchainSpreadAlertMode::VerifiedNet => verified_net_opportunity(item),
        OnchainSpreadAlertMode::RawObservation
            if raw_observation_sources_are_fresh(item.quality) =>
        {
            item.best_gross_spread_bps
                .is_some_and(|spread| spread >= item.config.spread_alert.min_raw_spread_bps)
        }
        OnchainSpreadAlertMode::RawObservation => false,
    }
}

const fn raw_observation_sources_are_fresh(quality: OnchainComparisonQuality) -> bool {
    matches!(
        quality,
        OnchainComparisonQuality::Fresh
            | OnchainComparisonQuality::LowLiquidity
            | OnchainComparisonQuality::NoNetProfit
            | OnchainComparisonQuality::RawCrossQuote
            | OnchainComparisonQuality::RawCustomPair
    )
}

fn cooldown_label(cooldown_ms: i64) -> String {
    let seconds = cooldown_ms.max(0) / 1_000;
    if seconds > 0 && seconds % 3_600 == 0 {
        format!("{}h", seconds / 3_600)
    } else if seconds >= 60 && seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn focus_item(
    draft: OnchainConfigDraft,
    data: OnchainData,
    config: &shared_types::OnchainComparisonConfig,
) {
    draft.load_config(config);
    data.reset_token_states();
    data.update.run(draft.patch());
}

fn batch_problem_details(item: &OnchainBatchItemSnapshot) -> Vec<String> {
    let mut details = Vec::new();
    push_unique_problem(&mut details, item.cex_problem.as_deref());
    push_unique_problem(&mut details, item.provider_problem.as_deref());
    push_unique_problem(&mut details, item.dex_problem.as_deref());
    push_unique_problem(&mut details, item.cross_chain_problem.as_deref());
    for problem in &item.degradation_reasons {
        push_unique_problem(&mut details, Some(problem));
    }
    details
}

fn push_unique_problem(details: &mut Vec<String>, problem: Option<&str>) {
    if let Some(problem) = problem.map(str::trim).filter(|problem| !problem.is_empty()) {
        if !details.iter().any(|detail| detail == problem) {
            details.push(problem.to_owned());
        }
    }
}

fn stale_notice(problem: &ApiProblem) -> AnyView {
    view! {
        <div class="onchain-batch-runtime-note is-warning" role="status">
            <strong>"显示上次队列快照"</strong>
            <span>{format!("{} · {}", problem.code, problem.message)}</span>
        </div>
    }
    .into_any()
}

fn loading_state() -> AnyView {
    view! {
        <div class="onchain-batch-empty"><strong>"正在读取队列"</strong><span>"等待后端返回批量监控快照。"</span></div>
    }
    .into_any()
}

fn error_state(problem: &ApiProblem) -> AnyView {
    view! {
        <div class="onchain-batch-runtime-note is-danger" role="alert">
            <strong>"队列读取失败"</strong>
            <span>{format!("{} · {}", problem.code, problem.message)}</span>
            <small>"当前状态未知，不表示队列为空。"</small>
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_evidence_includes_cex_and_provider_failures_once() {
        let mut item = OnchainBatchItemSnapshot::pending(
            "row-1".to_owned(),
            shared_types::OnchainComparisonConfig::default(),
            2_000,
            1,
        );
        item.cex_problem = Some("Kraken WS disconnected".to_owned());
        item.provider_problem = Some("Jupiter quote timed out".to_owned());
        item.degradation_reasons = vec![
            "Kraken WS disconnected".to_owned(),
            "waiting for retry".to_owned(),
        ];

        assert_eq!(
            batch_problem_details(&item),
            vec![
                "Kraken WS disconnected".to_owned(),
                "Jupiter quote timed out".to_owned(),
                "waiting for retry".to_owned(),
            ]
        );
    }

    #[test]
    fn batch_summary_separates_verified_opportunities_from_normal_no_profit_rows() {
        let mut opportunity = OnchainBatchItemSnapshot::pending(
            "opportunity".to_owned(),
            shared_types::OnchainComparisonConfig::default(),
            2_000,
            1,
        );
        opportunity.quality = OnchainComparisonQuality::Fresh;
        opportunity.best_net_spread_bps = Some(25.0);
        opportunity.config.spread_alert.min_net_spread_bps = 20.0;

        let mut no_profit = opportunity.clone();
        no_profit.item_id = "no-profit".to_owned();
        no_profit.quality = OnchainComparisonQuality::NoNetProfit;
        no_profit.best_net_spread_bps = Some(-5.0);

        let summary = batch_summary(&OnchainBatchSnapshot {
            items: vec![opportunity, no_profit],
            estimated_sweep_ms: 4_500,
            ..OnchainBatchSnapshot::default()
        });

        assert_eq!(summary.opportunities, "1");
        assert_eq!(summary.opportunity_class, "has-opportunity");
        assert_eq!(summary.issues, "0");
    }

    #[test]
    fn verified_cross_chain_closed_cycle_counts_as_an_opportunity() {
        let mut item = OnchainBatchItemSnapshot::pending(
            "cross-chain".to_owned(),
            shared_types::OnchainComparisonConfig::default(),
            30_000,
            1,
        );
        item.config.cross_chain.enabled = true;
        item.cross_chain_quality = OnchainCrossChainQuality::Fresh;
        item.best_cross_chain_net_return_bps = Some(35.0);
        item.config.spread_alert.min_net_spread_bps = 20.0;

        assert!(verified_net_opportunity(&item));
        assert!(!item_has_issue(&item));
        assert_eq!(
            cross_chain_edge_label(&item).as_deref(),
            Some("跨链闭环 +0.350% · 闭环已核算")
        );
    }

    #[test]
    fn expected_raw_observation_is_not_counted_as_a_runtime_issue() {
        let mut item = OnchainBatchItemSnapshot::pending(
            "raw".to_owned(),
            shared_types::OnchainComparisonConfig::default(),
            2_000,
            1,
        );
        item.quality = OnchainComparisonQuality::RawCrossQuote;
        item.config.spread_alert.mode = OnchainSpreadAlertMode::RawObservation;
        assert!(!item_has_issue(&item));

        item.config.spread_alert.mode = OnchainSpreadAlertMode::VerifiedNet;
        assert!(item_has_issue(&item));
        item.quality = OnchainComparisonQuality::Stale;
        assert!(item_has_issue(&item));
    }

    #[test]
    fn verified_cross_quote_conversion_keeps_the_batch_row_on_net_spread() {
        let mut item = OnchainBatchItemSnapshot::pending(
            "converted".to_owned(),
            shared_types::OnchainComparisonConfig::default(),
            2_000,
            1,
        );
        item.config.cex_symbol = "SOL/USD".to_owned();
        item.quality = OnchainComparisonQuality::Fresh;
        item.best_gross_spread_bps = Some(250.0);
        item.best_net_spread_bps = Some(120.0);
        item.quote_conversion = Some(shared_types::OnchainQuoteConversionEvidence {
            venue: "kraken".to_owned(),
            symbol: "USDC/USD".to_owned(),
            source: "ws_push".to_owned(),
            cex_quote: "USD".to_owned(),
            onchain_quote: "USDC".to_owned(),
            source_bid: 0.999,
            source_ask: 1.001,
            cex_to_onchain_bid: 1.0 / 1.001,
            cex_to_onchain_ask: 1.0 / 0.999,
            cex_to_onchain_capacity: 10_000.0,
            onchain_to_cex_capacity: 10_000.0,
            freshness_ms: 10,
            observed_at_ms: 1,
        });

        assert!(!batch_raw_observation(&item));
        assert!(verified_net_opportunity(&item));
        assert!(!item_has_issue(&item));
        assert_eq!(item.best_net_spread_bps, Some(120.0));
    }

    #[test]
    fn batch_rule_copy_keeps_profit_and_raw_observation_semantics_separate() {
        let mut item = OnchainBatchItemSnapshot::pending(
            "alert".to_owned(),
            shared_types::OnchainComparisonConfig::default(),
            2_000,
            1,
        );
        item.config.spread_alert.enabled = true;
        item.config.spread_alert.min_net_spread_bps = 20.0;
        item.config.spread_alert.cooldown_ms = 30_000;
        assert_eq!(
            batch_alert_rule_label(&item),
            "提醒 · 费后净差 ≥ 0.2% · 30s 去重"
        );

        item.config.spread_alert.mode = OnchainSpreadAlertMode::RawObservation;
        assert_eq!(
            batch_alert_rule_label(&item),
            "提醒 · 原始价差 ≥ 0.2% · 30s 去重"
        );
    }
}
