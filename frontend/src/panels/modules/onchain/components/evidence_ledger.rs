use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::OnchainComparisonSnapshot;

use super::super::format::{
    cex_source_label, compact_identity, freshness_label, provider_label, time_label,
};

pub(in crate::panels::modules::onchain) fn evidence_ledger(
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
) -> impl IntoView {
    view! {
        {move || evidence_state(&state.get())}
    }
}

fn evidence_state(state: &LoadState<OnchainComparisonSnapshot>) -> AnyView {
    let Some(snapshot) = state.value().cloned() else {
        let message = match state {
            LoadState::Loading => "正在读取链上报价与 CEX 来源状态…".to_owned(),
            LoadState::Error(problem) => problem.message.clone(),
            LoadState::Ready(_) | LoadState::Stale { .. } => "尚无来源证据。".to_owned(),
        };
        return evidence_disclosure("0 条", "来源状态", message);
    };
    if snapshot.quote_evidence.is_empty() {
        let summary = format!(
            "{} · {}",
            cex_source_label(&snapshot.cex_source),
            freshness_label(snapshot.cex_freshness_ms)
        );
        return evidence_disclosure(
            "0 条",
            &summary,
            "启用监控并取得链上官方响应后，这里会保留最新报价证据。".to_owned(),
        );
    }
    let count = snapshot.quote_evidence.len() + usize::from(snapshot.quote_conversion.is_some());
    let latest = snapshot.quote_evidence.last();
    let conversion = snapshot.quote_conversion.clone();
    let summary = latest.map_or_else(
        || "链上官方响应与 CEX 盘口来源".to_owned(),
        |evidence| {
            format!(
                "{} · 最新 {}",
                provider_label(&evidence.provider),
                time_label(evidence.observed_at_ms)
            )
        },
    );
    view! {
        <section class="onchain-evidence-ledger onchain-evidence-surface has-evidence">
            <header>
                <div>
                    <strong>"报价证据"</strong>
                    <span>{summary}</span>
                </div>
                <small>{format!("{count} 条")}</small>
            </header>
            {conversion.map(|evidence| view! {
                <div class="onchain-evidence-conversion" aria-label="报价币种换算证据">
                    <div class="onchain-evidence-conversion-market">
                        <span>"报价换算"</span>
                        <strong>{format!("{} · {}", evidence.venue.to_uppercase(), evidence.symbol)}</strong>
                        <small>{format!("{} → {}", evidence.cex_quote, evidence.onchain_quote)}</small>
                    </div>
                    <dl>
                        <div>
                            <dt>"卖出换算"</dt>
                            <dd>{format!("{:.6}", evidence.cex_to_onchain_bid)}</dd>
                        </div>
                        <div>
                            <dt>"买入换算"</dt>
                            <dd>{format!("{:.6}", evidence.cex_to_onchain_ask)}</dd>
                        </div>
                    </dl>
                    <div class="onchain-evidence-conversion-status">
                        <span>{cex_source_label(&evidence.source)}</span>
                        <strong>{freshness_label(Some(evidence.freshness_ms))}</strong>
                    </div>
                </div>
            })}
            <div class="workbench-table-wrap">
                <table class="workbench-table onchain-evidence-table" data-table-budget="bounded-small">
                    <thead>
                        <tr>
                            <th>"时间"</th>
                            <th>"来源"</th>
                            <th>"输入 → 输出"</th>
                            <th>"原始数量"</th>
                            <th>"路由"</th>
                            <th>"端点"</th>
                            <th>"边界"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {snapshot.quote_evidence.into_iter().rev().map(|evidence| {
                            let docs = evidence.official_docs_url.clone();
                            let endpoint_title = evidence.endpoint.clone();
                            view! {
                                <tr>
                                    <td data-label="时间" class="num">{time_label(evidence.observed_at_ms)}</td>
                                    <td data-label="来源"><a href=docs target="_blank" rel="noreferrer">{provider_label(&evidence.provider)}</a></td>
                                    <td data-label="输入 → 输出" title=format!("{} → {}", evidence.input_mint, evidence.output_mint)>
                                        {format!("{} → {}", compact_identity(&evidence.input_mint), compact_identity(&evidence.output_mint))}
                                    </td>
                                    <td data-label="原始数量" class="num">{format!("{} → {}", evidence.input_amount_raw, evidence.output_amount_raw)}</td>
                                    <td data-label="路由">{evidence.router.unwrap_or_else(|| "未返回".to_owned())}</td>
                                    <td data-label="端点" title=endpoint_title>{evidence.endpoint}</td>
                                    <td data-label="边界">{if evidence.transaction_requested { "边界异常" } else { "只读报价" }}</td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
        </section>
    }
    .into_any()
}

fn evidence_disclosure(count: &str, summary: &str, message: String) -> AnyView {
    view! {
        <section class="onchain-evidence-ledger onchain-evidence-surface">
            <header>
                <div>
                    <strong>"报价证据"</strong>
                    <span>{summary.to_owned()}</span>
                </div>
                <small>{count.to_owned()}</small>
            </header>
            <div class="onchain-evidence-empty">
                <strong>"尚无链上报价证据"</strong>
                <span>{message}</span>
            </div>
        </section>
    }
    .into_any()
}
