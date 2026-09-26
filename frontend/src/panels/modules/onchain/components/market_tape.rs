use leptos::prelude::*;
use shared_types::OnchainComparisonQuality;

use super::super::data::OnchainData;
use super::super::draft::OnchainConfigDraft;
use super::super::format::{
    cex_source_label, chain_label, freshness_label, provider_label, raw_amount_label,
};
use super::opportunity_status::opportunity_status;

pub(in crate::panels::modules::onchain) fn market_tape(
    draft: OnchainConfigDraft,
    data: OnchainData,
    market_rail_open: RwSignal<bool>,
    open_markets: Callback<()>,
) -> impl IntoView {
    view! {
        <header class=move || market_tape_class(data) aria-label="链上套利市场状态">
            <div class="onchain-market-identity">
                <button
                    type="button"
                    class="onchain-market-picker"
                    aria-label="打开套利市场列表"
                    aria-expanded=move || market_rail_open.get().to_string()
                    on:click=move |_| open_markets.run(())
                >
                    <span class="onchain-market-glyph" aria-hidden="true"></span>
                    <span class="onchain-market-copy">
                        <strong>{move || pair_label(draft, data)}</strong>
                        <small title=move || market_route_label(draft, data)>
                            {move || market_route_label(draft, data)}
                        </small>
                    </span>
                    <span class="onchain-market-picker-caret" aria-hidden="true">"⌄"</span>
                </button>
                <span
                    class=move || market_quality_class(data)
                    title=move || market_quality_title(data)
                >
                    {move || market_quality_label(data)}
                </span>
            </div>
            <div class="onchain-market-freshness" aria-label="双源行情时效">
                {source_freshness(data, MarketSource::Onchain)}
                {source_freshness(data, MarketSource::Cex)}
            </div>
        </header>
    }
}

fn market_tape_class(data: OnchainData) -> &'static str {
    data.state.with(|state| {
        if state.problem().is_some() { return "onchain-market-tape is-stale"; }
        state
            .value()
            .map_or("onchain-market-tape", |snapshot| match snapshot.quality {
                OnchainComparisonQuality::Stale => "onchain-market-tape is-stale",
                OnchainComparisonQuality::UpstreamUnavailable => {
                    "onchain-market-tape is-unavailable"
                }
                _ => "onchain-market-tape",
            })
    })
}

fn pair_label(draft: OnchainConfigDraft, data: OnchainData) -> String {
    data.state.with(|state| {
        state.value().map_or_else(
            || format!("{}/{}", draft.base_token.get(), draft.quote_token.get()),
            |snapshot| {
                format!(
                    "{}/{}",
                    snapshot.config.base_token, snapshot.config.quote_token
                )
            },
        )
    })
}

fn market_route_label(draft: OnchainConfigDraft, data: OnchainData) -> String {
    data.state.with(|state| {
        state.value().map_or_else(
            || {
                format!(
                    "{} · {} · {}",
                    chain_label(&draft.chain.get()),
                    provider_label(&draft.provider.get()),
                    draft.venue.get().to_uppercase(),
                )
            },
            |snapshot| {
                format!(
                    "{} · {} · {} {} · {}",
                    chain_label(&snapshot.config.chain),
                    provider_label(&snapshot.config.provider),
                    snapshot.config.cex_venue.to_uppercase(),
                    snapshot.config.cex_symbol,
                    raw_amount_label(
                        &snapshot.config.quote_amount_raw,
                        snapshot.config.quote_decimals,
                        &snapshot.config.quote_token,
                    ),
                )
            },
        )
    })
}

fn market_quality_label(data: OnchainData) -> &'static str {
    if data.configuration.awaiting_confirmation() { return "配置待确认"; }
    data.state.with(|state| {
        if state.problem().is_some() { return "状态待确认"; }
        state.value().map_or("读取中", |snapshot| {
            opportunity_status(snapshot).short_label
        })
    })
}

fn market_quality_class(data: OnchainData) -> String {
    if data.configuration.awaiting_confirmation() { return "onchain-market-quality is-warning".to_owned(); }
    data.state.with(|state| {
        if state.problem().is_some() { return "onchain-market-quality is-warning".to_owned(); }
        let tone = state
            .value()
            .map_or("is-neutral", |snapshot| opportunity_status(snapshot).tone);
        format!("onchain-market-quality {tone}")
    })
}

fn market_quality_title(data: OnchainData) -> String {
    if data.configuration.awaiting_confirmation() { return "配置操作尚未核对完整，暂停构建与修改".to_owned(); }
    data.state.with(|state| {
        if let Some(problem) = state.problem() { return problem.message.clone(); }
        state.value().map_or_else(
            || "正在读取链上与 交易所 运行状态".to_owned(),
            |snapshot| opportunity_status(snapshot).detail,
        )
    })
}

#[derive(Clone, Copy)]
enum MarketSource {
    Onchain,
    Cex,
}

fn source_freshness(data: OnchainData, source: MarketSource) -> impl IntoView {
    let label = match source {
        MarketSource::Onchain => "链上 时效",
        MarketSource::Cex => "交易所 时效",
    };
    view! {
        <span
            class=move || source_freshness_class(data, source)
            title=move || source_freshness_title(data, source)
        >
            <small>{label}</small>
            <strong class="num">{move || source_freshness_label(data, source)}</strong>
        </span>
    }
}

fn source_freshness_label(data: OnchainData, source: MarketSource) -> String {
    if data.configuration.awaiting_confirmation() { return "待确认".to_owned(); }
    data.state.with(|state| {
        if state.problem().is_some() { return "待确认".to_owned(); }
        state.value().map_or_else(
            || "读取中".to_owned(),
            |snapshot| {
                let freshness_ms = match source {
                    MarketSource::Onchain => snapshot.onchain_freshness_ms,
                    MarketSource::Cex => snapshot.cex_freshness_ms,
                };
                freshness_label(freshness_ms)
            },
        )
    })
}

fn source_freshness_class(data: OnchainData, source: MarketSource) -> String {
    if data.configuration.awaiting_confirmation() { return "onchain-market-source-age is-warning".to_owned(); }
    data.state.with(|state| {
        if state.problem().is_some() { return "onchain-market-source-age is-warning".to_owned(); }
        let tone = state.value().map_or("is-neutral", |snapshot| {
            if matches!(snapshot.quality, OnchainComparisonQuality::Pending) {
                return "is-neutral";
            }
            if !snapshot.config.enabled { return "is-neutral"; }
            let problem = match source {
                MarketSource::Onchain => snapshot.provider_problem.as_ref(),
                MarketSource::Cex => snapshot.cex_problem.as_ref(),
            };
            let age = match source {
                MarketSource::Onchain => snapshot.onchain_freshness_ms,
                MarketSource::Cex => snapshot.cex_freshness_ms,
            };
            if problem.is_some() || age.is_none_or(|age| age > snapshot.config.max_age_ms) {
                "is-warning"
            } else {
                "is-positive"
            }
        });
        format!("onchain-market-source-age {tone}")
    })
}

fn source_freshness_title(data: OnchainData, source: MarketSource) -> String {
    if data.configuration.awaiting_confirmation() { return "当前配置与报价尚待核对".to_owned(); }
    data.state.with(|state| {
        if let Some(problem) = state.problem() { return problem.message.clone(); }
        state.value().map_or_else(
            || "正在读取行情来源".to_owned(),
            |snapshot| {
                let (source_label, problem) = match source {
                    MarketSource::Onchain => (
                        provider_label(&snapshot.config.provider),
                        snapshot.provider_problem.as_deref(),
                    ),
                    MarketSource::Cex => (
                        format!(
                            "{} · {}",
                            snapshot.config.cex_venue.to_uppercase(),
                            cex_source_label(&snapshot.cex_source),
                        ),
                        snapshot.cex_problem.as_deref(),
                    ),
                };
                problem.map_or(source_label.clone(), |problem| {
                    format!("{source_label} · {problem}")
                })
            },
        )
    })
}
