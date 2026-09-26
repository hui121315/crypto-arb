use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{OnchainCexPairCatalog, OnchainCexPairOption, ONCHAIN_CEX_VENUES};

use super::super::data::{OnchainData, TokenResolution};
use super::super::draft::{explicit_pair_assets, normalized_asset, OnchainConfigDraft};
use super::super::format::{cex_source_label, freshness_label};

pub(super) fn cex_pair_select(draft: OnchainConfigDraft, data: OnchainData) -> impl IntoView {
    view! {
        <section class="onchain-cex-pair" aria-label="交易所 对比市场">
            <div class="onchain-pair-heading">
                <div><strong>"交易所对比"</strong><span>"交易所和交易对都由你选定"</span></div>
                <span class="onchain-pair-count">{move || pair_count_label(&data.form.cex_pairs.get(), &draft.symbol.get())}</span>
            </div>
            <div class="onchain-cex-market-fields">
                {venue_selector(draft)}
                {pair_selector(draft, data.form.cex_pairs)}
            </div>
            {move || pair_status(draft, data, &data.form.cex_pairs.get())}
        </section>
    }
}

pub(super) fn cex_pair_apply_problem(
    draft: OnchainConfigDraft,
    _data: OnchainData,
) -> Option<String> {
    explicit_pair_problem(&draft.symbol.get())
}

fn pair_selector(draft: OnchainConfigDraft, state: RwSignal<LoadState<OnchainCexPairCatalog>>) -> AnyView {
    view! {
        <label class="workbench-field onchain-pair-field">
            <span>"交易对（可输入）"</span>
            <span class="onchain-pair-input-shell">
                <input
                    class="num"
                    aria-label="交易对（可输入）"
                    list="onchain-cex-pair-options"
                    placeholder="输入或选择，如 PUPS/USD"
                    prop:value=move || draft.symbol.get()
                    on:input=move |event| {
                        let symbol = event_target_value(&event);
                        draft.apply_cex_symbol(&symbol);
                    }
                />
                <span class="onchain-pair-input-cue" aria-hidden="true">"⌄"</span>
            </span>
            <datalist id="onchain-cex-pair-options">
                {move || state.get().value().map(|catalog| catalog.pairs.clone()).unwrap_or_default().into_iter().map(|pair| {
                    let value = pair.cex_symbol.clone();
                    let label = format!("{} · {}", pair.cex_symbol, pair.native_symbol);
                    view! {
                        <option value=value label=label></option>
                    }
                }).collect_view()}
            </datalist>
        </label>
    }
    .into_any()
}

fn venue_selector(draft: OnchainConfigDraft) -> impl IntoView {
    view! {
        <label class="workbench-field onchain-venue-field">
            <span>"交易所"</span>
            <select
                prop:value=move || draft.venue.get()
                on:change=move |event| draft.apply_venue(&event_target_value(&event))
            >
                {ONCHAIN_CEX_VENUES.iter().map(|venue| {
                    view! { <option value=venue.id>{venue.label}</option> }
                }).collect_view()}
            </select>
        </label>
    }
}

fn pair_status(
    draft: OnchainConfigDraft,
    data: OnchainData,
    state: &LoadState<OnchainCexPairCatalog>,
) -> AnyView {
    let model = pair_status_model(draft, data, state);
    let note_class = format!("onchain-pair-next {}", model.note_tone);
    let evidence_summary = format!(
        "{} · {} · {}",
        model.listing.value, model.market.value, model.conversion.value
    );
    view! {
        <details class="onchain-pair-evidence-details">
            <summary role="status">{evidence_summary}</summary>
            <div class="onchain-pair-evidence">
                {pair_fact("交易对", model.input)}
                {pair_fact("官方挂牌", model.listing)}
                {pair_fact("实时行情", model.market)}
                {pair_fact("Quote 换算", model.conversion)}
            </div>
        </details>
        <p class=note_class>{model.note}</p>
    }
    .into_any()
}

struct PairStatusModel {
    input: PairFact,
    listing: PairFact,
    market: PairFact,
    conversion: PairFact,
    note: String,
    note_tone: &'static str,
}

struct PairFact {
    value: String,
    detail: String,
    tone: &'static str,
}

fn pair_fact(label: &'static str, fact: PairFact) -> impl IntoView {
    let class = format!("onchain-pair-fact {}", fact.tone);
    let detail_title = fact.detail.clone();
    view! {
        <div class=class>
            <small>{label}</small>
            <strong>{fact.value}</strong>
            <span title=detail_title>{fact.detail}</span>
        </div>
    }
}

fn pair_status_model(
    draft: OnchainConfigDraft,
    data: OnchainData,
    state: &LoadState<OnchainCexPairCatalog>,
) -> PairStatusModel {
    let selected_symbol = draft.symbol.get();
    let base_token = draft.base_token.get();
    let chain_quote = draft.quote_token.get();
    let input_problem = explicit_pair_problem(&selected_symbol);
    let identity_problem = identity_pair_problem(data);
    let input = input_problem.as_ref().map_or_else(
        || PairFact {
            value: selected_symbol.trim().to_ascii_uppercase(),
            detail: "Base/Quote 格式有效".to_owned(),
            tone: "is-positive",
        },
        |problem| PairFact {
            value: "等待输入".to_owned(),
            detail: problem.clone(),
            tone: "is-danger",
        },
    );
    let listing = if input_problem.is_some() {
        PairFact {
            value: "尚未核对".to_owned(),
            detail: "先输入有效交易对".to_owned(),
            tone: "is-neutral",
        }
    } else {
        listing_fact(
            &draft.venue.get(),
            &selected_symbol,
            state,
            &data.state.get(),
        )
    };
    let market = if input_problem.is_some() || identity_problem.is_some() {
        PairFact {
            value: "尚未订阅".to_owned(),
            detail: "等待身份与交易对通过".to_owned(),
            tone: "is-neutral",
        }
    } else {
        market_fact(&draft.venue.get(), &selected_symbol, &data.state.get())
    };
    let conversion = quote_conversion_fact(draft, &data.state.get());
    let pair_context = PairContext {
        selected_symbol: &selected_symbol,
        base_token: &base_token,
        chain_quote: &chain_quote,
        base_identity_resolved: draft.base_identity_resolved.get(),
        quote_identity_resolved: draft.quote_identity_resolved.get(),
    };
    let (note, note_tone) = pair_next_step(
        input_problem,
        identity_problem,
        pair_context,
        &listing,
        &market,
        &conversion,
    );
    PairStatusModel {
        input,
        listing,
        market,
        conversion,
        note,
        note_tone,
    }
}

fn quote_conversion_fact(
    draft: OnchainConfigDraft,
    state: &LoadState<shared_types::OnchainComparisonSnapshot>,
) -> PairFact {
    let selected_symbol = draft.symbol.get();
    let Some((_, cex_quote)) = explicit_pair_assets(&selected_symbol) else {
        return quote_conversion_waiting("等待有效 交易所 交易对");
    };
    let onchain_quote = normalized_asset(&draft.quote_token.get());
    if onchain_quote == normalized_asset(&cex_quote) {
        return PairFact {
            value: "同一 Quote".to_owned(),
            detail: format!("链上与 交易所 均使用 {onchain_quote}，无需汇率换算"),
            tone: "is-positive",
        };
    }
    let Some(snapshot) = state.value() else {
        return quote_conversion_waiting("等待链上套利运行快照");
    };
    if !draft_matches_snapshot(draft, snapshot) {
        return quote_conversion_waiting("当前交易对草稿尚未应用，不借用旧汇率数据依据");
    }
    let Some(evidence) = snapshot.quote_conversion.as_ref() else {
        return quote_conversion_waiting("等待所选 交易所 的官方 WS 汇率交易对");
    };
    if !quote_conversion_matches(evidence, &draft.venue.get(), &cex_quote, &onchain_quote) {
        return quote_conversion_waiting("现有汇率数据依据与当前交易所或 Quote 不匹配");
    }
    if evidence.freshness_ms > snapshot.config.max_age_ms {
        return PairFact {
            value: "汇率已过期".to_owned(),
            detail: format!(
                "{} 新鲜度 {}，超过 {}ms 门槛；等待 WS 更新",
                evidence.symbol,
                freshness_label(Some(evidence.freshness_ms)),
                snapshot.config.max_age_ms,
            ),
            tone: "is-warning",
        };
    }
    PairFact {
        value: "WS 汇率已接入".to_owned(),
        detail: format!(
            "{} · {} → {} · 新鲜度 {}",
            evidence.symbol,
            evidence.cex_quote,
            evidence.onchain_quote,
            freshness_label(Some(evidence.freshness_ms)),
        ),
        tone: "is-positive",
    }
}

pub(super) fn selected_quotes_comparable(
    draft: OnchainConfigDraft,
    state: &LoadState<shared_types::OnchainComparisonSnapshot>,
) -> bool {
    let Some((_, cex_quote)) = explicit_pair_assets(&draft.symbol.get()) else {
        return false;
    };
    let onchain_quote = normalized_asset(&draft.quote_token.get());
    if onchain_quote == normalized_asset(&cex_quote) {
        return true;
    }
    let Some(snapshot) = state
        .value()
        .filter(|snapshot| draft_matches_snapshot(draft, snapshot))
    else {
        return false;
    };
    snapshot.quote_conversion.as_ref().is_some_and(|evidence| {
        quote_conversion_matches(evidence, &draft.venue.get(), &cex_quote, &onchain_quote)
            && evidence.freshness_ms <= snapshot.config.max_age_ms
    })
}

fn quote_conversion_waiting(detail: &str) -> PairFact {
    PairFact {
        value: "汇率待建立".to_owned(),
        detail: detail.to_owned(),
        tone: "is-warning",
    }
}

fn draft_matches_snapshot(
    draft: OnchainConfigDraft,
    snapshot: &shared_types::OnchainComparisonSnapshot,
) -> bool {
    draft
        .venue
        .get()
        .eq_ignore_ascii_case(&snapshot.config.cex_venue)
        && normalized_asset(&draft.symbol.get()) == normalized_asset(&snapshot.config.cex_symbol)
        && normalized_asset(&draft.base_token.get())
            == normalized_asset(&snapshot.config.base_token)
        && normalized_asset(&draft.quote_token.get())
            == normalized_asset(&snapshot.config.quote_token)
}

fn quote_conversion_matches(
    evidence: &shared_types::OnchainQuoteConversionEvidence,
    venue: &str,
    cex_quote: &str,
    onchain_quote: &str,
) -> bool {
    evidence.venue.eq_ignore_ascii_case(venue)
        && evidence.cex_quote.eq_ignore_ascii_case(cex_quote)
        && evidence.onchain_quote.eq_ignore_ascii_case(onchain_quote)
        && evidence.source == "ws_push"
        && evidence.cex_to_onchain_bid.is_finite()
        && evidence.cex_to_onchain_bid > 0.0
        && evidence.cex_to_onchain_ask.is_finite()
        && evidence.cex_to_onchain_ask > 0.0
}

fn listing_fact(
    venue: &str,
    selected_symbol: &str,
    catalog_state: &LoadState<OnchainCexPairCatalog>,
    runtime_state: &LoadState<shared_types::OnchainComparisonSnapshot>,
) -> PairFact {
    if let Some(fact) = runtime_listing_fact(venue, selected_symbol, runtime_state) {
        return fact;
    }
    catalog_listing_fact(selected_symbol, catalog_state)
}

fn runtime_listing_fact(
    venue: &str,
    selected_symbol: &str,
    state: &LoadState<shared_types::OnchainComparisonSnapshot>,
) -> Option<PairFact> {
    let LoadState::Ready(snapshot) = state else {
        return None;
    };
    if !snapshot.config.cex_venue.eq_ignore_ascii_case(venue)
        || normalized_asset(&snapshot.config.cex_symbol) != normalized_asset(selected_symbol)
    {
        return None;
    }
    let evidence = snapshot
        .execution_readiness
        .directions
        .iter()
        .map(|direction| &direction.cex_instrument)
        .find(|evidence| {
            evidence.ready
                && evidence.venue.eq_ignore_ascii_case(venue)
                && normalized_asset(&evidence.requested_symbol) == normalized_asset(selected_symbol)
        })?;
    Some(PairFact {
        value: "官方已挂牌".to_owned(),
        detail: format!(
            "官方 Spot registry · 原生代码 {} · 当前执行规格已核对",
            evidence.native_symbol.as_deref().unwrap_or(selected_symbol),
        ),
        tone: "is-positive",
    })
}

fn catalog_listing_fact(
    selected_symbol: &str,
    state: &LoadState<OnchainCexPairCatalog>,
) -> PairFact {
    match state {
        LoadState::Loading => PairFact {
            value: "目录读取中".to_owned(),
            detail: "正在读取官方 Spot instrument registry".to_owned(),
            tone: "is-neutral",
        },
        LoadState::Error(problem) => PairFact {
            value: "目录读取失败".to_owned(),
            detail: problem.message.clone(),
            tone: "is-danger",
        },
        LoadState::Stale {
            value: catalog,
            problem,
        } => selected_pair(selected_symbol, catalog).map_or_else(
            || PairFact {
                value: "挂牌未核对".to_owned(),
                detail: format!("官方目录更新失败：{}", problem.message),
                tone: "is-warning",
            },
            |pair| PairFact {
                value: "历史已挂牌".to_owned(),
                detail: format!("{} · 目录更新失败", pair_evidence(pair)),
                tone: "is-warning",
            },
        ),
        LoadState::Ready(catalog) => ready_listing_fact(selected_symbol, catalog),
    }
}

fn ready_listing_fact(selected_symbol: &str, catalog: &OnchainCexPairCatalog) -> PairFact {
    let pair = selected_pair(selected_symbol, catalog);
    let Some(problem) = catalog.problem.as_ref() else {
        return pair.map_or_else(
            || catalog_problem_fact(catalog),
            |pair| PairFact {
                value: "官方已挂牌".to_owned(),
                detail: pair_evidence(pair),
                tone: "is-positive",
            },
        );
    };
    let problem_fact = catalog_problem_fact(catalog);
    let Some(pair) = pair else {
        return problem_fact;
    };
    PairFact {
        value: if problem.code == "ONCHAIN_CEX_PAIR_REGISTRY_SYNCING" {
            "挂牌待复核".to_owned()
        } else {
            "历史已挂牌".to_owned()
        },
        detail: format!("{} · {}", pair_evidence(pair), problem_fact.detail),
        tone: "is-warning",
    }
}

fn catalog_problem_fact(catalog: &OnchainCexPairCatalog) -> PairFact {
    let Some(problem) = catalog.problem.as_ref() else {
        return PairFact {
            value: "所选未收录".to_owned(),
            detail: "官方目录没有当前交易对数据依据".to_owned(),
            tone: "is-warning",
        };
    };
    let value = match problem.code.as_str() {
        "ONCHAIN_CEX_PAIR_REGISTRY_SYNCING" => "目录同步中",
        "ONCHAIN_CEX_PAIR_REGISTRY_STALE" => "目录已过期",
        "ONCHAIN_CEX_PAIR_REGISTRY_UNAVAILABLE" => "目录刷新失败",
        "ONCHAIN_CEX_PAIR_REGISTRY_UNSUPPORTED" => "目录未接入",
        "ONCHAIN_CEX_PAIR_MISSING" => "官方未收录",
        _ => "挂牌未核对",
    };
    let detail = match problem.code.as_str() {
        "ONCHAIN_CEX_PAIR_REGISTRY_SYNCING" => format!(
            "{} 官方现货规格目录正在首次同步，完成后自动刷新",
            catalog.venue.to_ascii_uppercase(),
        ),
        "ONCHAIN_CEX_PAIR_REGISTRY_STALE" => format!(
            "{} 官方现货规格目录已过期，系统正在刷新",
            catalog.venue.to_ascii_uppercase(),
        ),
        "ONCHAIN_CEX_PAIR_REGISTRY_UNAVAILABLE" => format!(
            "{} 官方现货规格目录刷新失败，系统会按退避规则自动重试",
            catalog.venue.to_ascii_uppercase(),
        ),
        "ONCHAIN_CEX_PAIR_REGISTRY_UNSUPPORTED" => format!(
            "{} 尚未接入可核对的官方现货规格目录",
            catalog.venue.to_ascii_uppercase(),
        ),
        "ONCHAIN_CEX_PAIR_MISSING" => format!(
            "{} 官方现货目录当前没有 {} 交易对",
            catalog.venue.to_ascii_uppercase(),
            catalog.base_token.to_ascii_uppercase(),
        ),
        _ => problem.message.clone(),
    };
    PairFact {
        value: value.to_owned(),
        detail,
        tone: "is-warning",
    }
}

fn market_fact(
    draft_venue: &str,
    draft_symbol: &str,
    state: &LoadState<shared_types::OnchainComparisonSnapshot>,
) -> PairFact {
    let Some(snapshot) = state.value() else {
        return PairFact {
            value: "等待运行状态".to_owned(),
            detail: "尚未读取链上套利运行快照".to_owned(),
            tone: "is-neutral",
        };
    };
    let config = &snapshot.config;
    if !draft_venue.eq_ignore_ascii_case(&config.cex_venue)
        || normalized_asset(draft_symbol) != normalized_asset(&config.cex_symbol)
    {
        return PairFact {
            value: "等待应用".to_owned(),
            detail: "当前输入尚未应用，不会借用旧交易对行情".to_owned(),
            tone: "is-neutral",
        };
    }
    if !config.enabled {
        return PairFact {
            value: "监控未启用".to_owned(),
            detail: "应用配置后开始订阅所选精确交易对".to_owned(),
            tone: "is-neutral",
        };
    }
    match snapshot.cex_source.as_str() {
        "ws_push"
            if snapshot.cex_problem.is_none()
                && snapshot
                    .cex_freshness_ms
                    .is_some_and(|age| age <= snapshot.config.max_age_ms) =>
        {
            PairFact {
                value: "WS 实时".to_owned(),
                detail: format!(
                    "最优价新鲜度 {}",
                    freshness_label(snapshot.cex_freshness_ms)
                ),
                tone: "is-positive",
            }
        }
        "ws_push" => PairFact {
            value: "WS 已过期".to_owned(),
            detail: snapshot.cex_problem.clone().unwrap_or_else(|| {
                format!(
                    "最后最优价距今 {}，已超过 {}ms 时效门槛；正在自动重连",
                    freshness_label(snapshot.cex_freshness_ms),
                    snapshot.config.max_age_ms,
                )
            }),
            tone: "is-warning",
        },
        "ws_pending" => PairFact {
            value: if snapshot.cex_problem.is_some() {
                "WS 重连中".to_owned()
            } else {
                "等待 WS 首帧".to_owned()
            },
            detail: snapshot
                .cex_problem
                .clone()
                .unwrap_or_else(|| "订阅已请求，等待精确交易对首个最优价".to_owned()),
            tone: "is-warning",
        },
        "not_started" => PairFact {
            value: "尚未启动".to_owned(),
            detail: "等待行情订阅任务启动".to_owned(),
            tone: "is-neutral",
        },
        _ => PairFact {
            value: "非 WS 快照".to_owned(),
            detail: format!(
                "{} · 新鲜度 {}",
                cex_source_label(&snapshot.cex_source),
                freshness_label(snapshot.cex_freshness_ms),
            ),
            tone: "is-warning",
        },
    }
}

#[derive(Clone, Copy)]
struct PairContext<'a> {
    selected_symbol: &'a str,
    base_token: &'a str,
    chain_quote: &'a str,
    base_identity_resolved: bool,
    quote_identity_resolved: bool,
}

fn pair_next_step(
    input_problem: Option<String>,
    identity_problem: Option<String>,
    context: PairContext<'_>,
    listing: &PairFact,
    market: &PairFact,
    conversion: &PairFact,
) -> (String, &'static str) {
    if let Some(problem) = input_problem.or(identity_problem) {
        return (problem, "is-danger");
    }
    let (cex_base, cex_quote) = explicit_pair_assets(context.selected_symbol).unwrap_or_default();
    if !context.base_identity_resolved || !context.quote_identity_resolved {
        return (
            format!(
                "链上 {}/{} 的精度已读取，但资产符号仍待核对；{} 继续读取官方挂牌与 WS 原始价格，不判断利润或允许执行。",
                context.base_token,
                context.chain_quote,
                context.selected_symbol.trim().to_ascii_uppercase(),
            ),
            "is-warning",
        );
    }
    if normalized_asset(context.base_token) != cex_base {
        return (
            format!(
                "链上 {}/{} 与 交易所 {} 的 Base 不同；只展示两个独立市场的原始价格，不证明同一资产或利润。",
                normalized_asset(context.base_token),
                normalized_asset(context.chain_quote),
                context.selected_symbol.trim().to_ascii_uppercase(),
            ),
            "is-warning",
        );
    }
    if normalized_asset(context.chain_quote) != normalized_asset(&cex_quote)
        && conversion.tone != "is-positive"
    {
        return (
            format!(
                "链上 Quote 为 {}，交易所 Quote 为 {}；{}，因此只展示原始价格，不判断利润。",
                normalized_asset(context.chain_quote),
                normalized_asset(&cex_quote),
                conversion.detail,
            ),
            conversion.tone,
        );
    }
    if listing.tone != "is-positive" {
        if matches!(
            listing.value.as_str(),
            "目录同步中" | "目录已过期" | "目录刷新失败" | "挂牌待复核" | "历史已挂牌"
        ) && market.tone == "is-positive"
        {
            return (
                "WS 实时价格已到达，但这只证明交易对可订阅；下单精度与最小数量目录仍在恢复，系统会自动重试。"
                    .to_owned(),
                "is-warning",
            );
        }
        return (listing.detail.clone(), listing.tone);
    }
    if market.tone != "is-positive" {
        if market.value.contains("WS") {
            return (
                "保持监控开启；收到官方 WS 首帧后自动开始比较，无需重新应用配置。".to_owned(),
                market.tone,
            );
        }
        return (market.detail.clone(), market.tone);
    }
    if normalized_asset(context.chain_quote) != normalized_asset(&cex_quote) {
        return (
            "官方挂牌、精确 WS 和 Quote 换算均已通过；系统已按买卖方向换算 Quote，并据此计算费后净收益。"
                .to_owned(),
            "is-positive",
        );
    }
    (
        "官方挂牌与精确 WS 最优价均已通过；完整盘口只在构建时读取。".to_owned(),
        "is-positive",
    )
}

fn identity_pair_problem(data: OnchainData) -> Option<String> {
    identity_leg_problem("Base", &data.form.base_identity.get())
}

fn identity_leg_problem(label: &str, state: &TokenResolution) -> Option<String> {
    match state {
        TokenResolution::Dirty | TokenResolution::Loading => {
            Some(format!("等待 {label} 身份自动识别后读取 交易所 市场"))
        }
        TokenResolution::PrecisionOnly(_) => None,
        TokenResolution::Error(_) => Some(format!("等待 {label} 身份通过后读取 交易所 市场")),
        TokenResolution::Idle | TokenResolution::Ready(_) => None,
    }
}

fn explicit_pair_problem(symbol: &str) -> Option<String> {
    let Some((base, quote)) = explicit_pair_assets(symbol) else {
        return Some("请输入明确的 Base/Quote 交易对，例如 PUPS/USD".to_owned());
    };
    (base == quote).then(|| "Base 与 Quote 不能相同".to_owned())
}

fn selected_pair<'a>(
    selected_symbol: &str,
    catalog: &'a OnchainCexPairCatalog,
) -> Option<&'a OnchainCexPairOption> {
    let selected = normalized_asset(selected_symbol);
    catalog
        .pairs
        .iter()
        .find(|pair| normalized_asset(&pair.cex_symbol) == selected)
}

fn pair_evidence(pair: &OnchainCexPairOption) -> String {
    format!("官方 Spot registry · 原生代码 {}", pair.native_symbol,)
}

fn pair_count_label(state: &LoadState<OnchainCexPairCatalog>, selected_symbol: &str) -> String {
    let custom_pair = explicit_pair_assets(selected_symbol).is_some();
    match state {
        LoadState::Loading if custom_pair => "自定义".to_owned(),
        LoadState::Loading => "读取中".to_owned(),
        LoadState::Error(_) if custom_pair => "自定义".to_owned(),
        LoadState::Error(_) => "目录暂不可用".to_owned(),
        LoadState::Stale { value, .. } if !value.pairs.is_empty() => {
            format!("{} 个历史", value.pairs.len())
        }
        LoadState::Stale { .. } if custom_pair => "自定义".to_owned(),
        LoadState::Stale { .. } => "目录暂不可用".to_owned(),
        LoadState::Ready(catalog) if catalog.pairs.is_empty() && custom_pair => "自定义".to_owned(),
        LoadState::Ready(catalog) if catalog.pairs.is_empty() => "目录未收录".to_owned(),
        LoadState::Ready(catalog) => format!("{} 个", catalog.pairs.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{MarketDataQuality, MarketDataSourceKind};

    #[test]
    fn cross_quote_pair_is_selectable_for_raw_comparison() {
        let ready = PairFact {
            value: "ready".to_owned(),
            detail: "ready".to_owned(),
            tone: "is-positive",
        };
        assert!(explicit_pair_problem("SOL/USDT").is_none());
        let context = PairContext {
            selected_symbol: "SOL/USDT",
            base_token: "SOL",
            chain_quote: "USDC",
            base_identity_resolved: true,
            quote_identity_resolved: true,
        };
        let conversion = quote_conversion_waiting("等待所选 交易所 的官方 WS 汇率交易对");
        let (note, tone) = pair_next_step(None, None, context, &ready, &ready, &conversion);
        assert!(note.contains("链上 Quote 为 USDC"));
        assert!(note.contains("不判断利润"));
        assert_eq!(tone, "is-warning");
    }

    #[test]
    fn fresh_cross_quote_conversion_enables_fee_adjusted_profit_copy() {
        let ready = PairFact {
            value: "ready".to_owned(),
            detail: "ready".to_owned(),
            tone: "is-positive",
        };
        let conversion = PairFact {
            value: "WS 汇率已接入".to_owned(),
            detail: "USDC/USD · USD → USDC · 新鲜度 10ms".to_owned(),
            tone: "is-positive",
        };
        let context = PairContext {
            selected_symbol: "PUPS/USD",
            base_token: "PUPS",
            chain_quote: "USDC",
            base_identity_resolved: true,
            quote_identity_resolved: true,
        };

        let (note, tone) = pair_next_step(None, None, context, &ready, &ready, &conversion);

        assert!(note.contains("已按买卖方向换算 Quote"));
        assert!(note.contains("费后净收益"));
        assert_eq!(tone, "is-positive");
    }

    #[test]
    fn fresh_matching_ws_conversion_makes_cross_quote_comparable() {
        Owner::new().with(|| {
            let mut snapshot = shared_types::OnchainComparisonSnapshot::default();
            snapshot.config.cex_venue = "kraken".to_owned();
            snapshot.config.cex_symbol = "PUPS/USD".to_owned();
            snapshot.config.base_token = "PUPS".to_owned();
            snapshot.config.quote_token = "USDC".to_owned();
            snapshot.config.max_age_ms = 5_000;
            snapshot.quote_conversion = Some(shared_types::OnchainQuoteConversionEvidence {
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
            let draft = OnchainConfigDraft::from_config(&snapshot.config);
            let mut state = LoadState::Ready(snapshot.clone());

            assert!(selected_quotes_comparable(draft, &state));

            snapshot
                .quote_conversion
                .as_mut()
                .expect("conversion")
                .freshness_ms = 5_001;
            state = LoadState::Ready(snapshot);
            assert!(!selected_quotes_comparable(draft, &state));
        });
    }

    #[test]
    fn explicit_pair_remains_usable_without_catalog_rows() {
        assert!(explicit_pair_problem("PUPS/USD").is_none());
        assert!(explicit_pair_problem("SOL/USD").is_none());
        assert_eq!(
            pair_count_label(
                &LoadState::Ready(OnchainCexPairCatalog::default()),
                "PUPS/USD"
            ),
            "自定义"
        );

        let catalog = OnchainCexPairCatalog {
            venue: "kraken".to_owned(),
            problem: Some(shared_types::ApiProblem::new(
                "ONCHAIN_CEX_PAIR_REGISTRY_SYNCING",
                "Kraken official registry not ready",
            )),
            ..OnchainCexPairCatalog::default()
        };
        let fact = catalog_problem_fact(&catalog);
        assert_eq!(fact.value, "目录同步中");
        assert_eq!(
            fact.detail,
            "KRAKEN 官方现货规格目录正在首次同步，完成后自动刷新"
        );
    }

    #[test]
    fn official_pair_count_wins_over_the_manual_input_label() {
        let pair = |symbol: &str| OnchainCexPairOption {
            venue: "kraken".to_owned(),
            base_token: "PUPS".to_owned(),
            quote_token: symbol
                .split_once('/')
                .map_or("", |(_, quote)| quote)
                .to_owned(),
            cex_symbol: symbol.to_owned(),
            native_symbol: symbol.to_owned(),
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(0),
            observed_at_ms: 1,
        };
        let catalog = OnchainCexPairCatalog {
            pairs: vec![pair("PUPS/USD"), pair("PUPS/EUR")],
            ..OnchainCexPairCatalog::default()
        };

        assert_eq!(
            pair_count_label(&LoadState::Ready(catalog), "PUPS/USD"),
            "2 个"
        );
    }

    #[test]
    fn fresh_runtime_instrument_evidence_replaces_a_recovered_catalog_warning() {
        let catalog = LoadState::Ready(OnchainCexPairCatalog {
            venue: "kraken".to_owned(),
            base_token: "PUPS".to_owned(),
            problem: Some(shared_types::ApiProblem::new(
                "ONCHAIN_CEX_PAIR_MISSING",
                "startup checkpoint did not contain PUPS",
            )),
            ..OnchainCexPairCatalog::default()
        });
        let mut snapshot = shared_types::OnchainComparisonSnapshot::default();
        snapshot.config.cex_venue = "kraken".to_owned();
        snapshot.config.cex_symbol = "PUPS/USD".to_owned();
        snapshot.execution_readiness.directions = vec![shared_types::OnchainDirectionReadiness {
            direction: shared_types::OnchainComparisonDirection::BuyOnchainSellCex,
            path: Default::default(),
            inventory: Vec::new(),
            cex_instrument: shared_types::OnchainCexInstrumentEvidence {
                venue: "kraken".to_owned(),
                requested_symbol: "PUPS/USD".to_owned(),
                native_symbol: Some("PUPS/USD".to_owned()),
                status: shared_types::OnchainCexInstrumentStatus::Ready,
                ready: true,
                source: "instrument_registry".to_owned(),
                observed_at_ms: Some(100),
                problem: None,
            },
            build_ready: false,
            submit_ready: false,
            blockers: Vec::new(),
        }];

        let fact = listing_fact("kraken", "PUPS/USD", &catalog, &LoadState::Ready(snapshot));

        assert_eq!(fact.value, "官方已挂牌");
        assert!(fact.detail.contains("PUPS/USD"));
        assert!(fact.detail.contains("当前执行规格已核对"));
        assert_eq!(fact.tone, "is-positive");
    }

    #[test]
    fn live_ws_does_not_masquerade_as_execution_spec_evidence() {
        let listing = PairFact {
            value: "目录同步中".to_owned(),
            detail: "目录正在恢复".to_owned(),
            tone: "is-warning",
        };
        let market = PairFact {
            value: "WS 实时".to_owned(),
            detail: "最优价新鲜度 0ms".to_owned(),
            tone: "is-positive",
        };
        let context = PairContext {
            selected_symbol: "SOL/USDC",
            base_token: "SOL",
            chain_quote: "USDC",
            base_identity_resolved: true,
            quote_identity_resolved: true,
        };

        let (note, tone) = pair_next_step(None, None, context, &listing, &market, &market);

        assert!(note.contains("WS 实时价格已到达"));
        assert!(note.contains("下单精度与最小数量目录仍在恢复"));
        assert_eq!(tone, "is-warning");
    }

    #[test]
    fn restored_pair_is_labeled_as_historical_until_the_registry_refreshes() {
        let pair = OnchainCexPairOption {
            venue: "binance".to_owned(),
            base_token: "SOL".to_owned(),
            quote_token: "USDC".to_owned(),
            cex_symbol: "SOL/USDC".to_owned(),
            native_symbol: "SOLUSDC".to_owned(),
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(2),
            observed_at_ms: 100,
        };
        let catalog = OnchainCexPairCatalog {
            venue: "binance".to_owned(),
            base_token: "SOL".to_owned(),
            pairs: vec![pair],
            problem: Some(shared_types::ApiProblem::new(
                "ONCHAIN_CEX_PAIR_REGISTRY_STALE",
                "restored checkpoint requires a fresh probe",
            )),
        };

        let fact = ready_listing_fact("SOL/USDC", &catalog);

        assert_eq!(fact.value, "历史已挂牌");
        assert!(fact.detail.contains("SOLUSDC"));
        assert!(fact.detail.contains("系统正在刷新"));
        assert_eq!(fact.tone, "is-warning");
    }

    #[test]
    fn different_base_pair_is_allowed_but_never_presented_as_profit() {
        let ready = PairFact {
            value: "ready".to_owned(),
            detail: "ready".to_owned(),
            tone: "is-positive",
        };
        let context = PairContext {
            selected_symbol: "SOL/USD",
            base_token: "PUPS",
            chain_quote: "USDC",
            base_identity_resolved: true,
            quote_identity_resolved: true,
        };
        let (note, tone) = pair_next_step(None, None, context, &ready, &ready, &ready);

        assert!(note.contains("Base 不同"));
        assert!(note.contains("不证明同一资产或利润"));
        assert_eq!(tone, "is-warning");
    }

    #[test]
    fn provisional_identity_keeps_market_monitoring_in_observation_mode() {
        let ready = PairFact {
            value: "ready".to_owned(),
            detail: "ready".to_owned(),
            tone: "is-positive",
        };
        let context = PairContext {
            selected_symbol: "PUPS/USD",
            base_token: "PUPS",
            chain_quote: "USDC",
            base_identity_resolved: false,
            quote_identity_resolved: true,
        };
        let (note, tone) = pair_next_step(None, None, context, &ready, &ready, &ready);

        assert!(note.contains("精度已读取"));
        assert!(note.contains("不判断利润或允许执行"));
        assert_eq!(tone, "is-warning");
    }

    #[test]
    fn applied_pair_marks_an_expired_ws_snapshot_as_reconnecting() {
        let mut snapshot = shared_types::OnchainComparisonSnapshot::default();
        snapshot.config.cex_venue = "kraken".to_owned();
        snapshot.config.cex_symbol = "PUPS/USD".to_owned();
        snapshot.config.enabled = true;
        snapshot.cex_source = "ws_push".to_owned();
        snapshot.cex_freshness_ms = Some(11_400);
        let state = LoadState::Ready(snapshot);

        let fact = market_fact("kraken", "PUPS/USD", &state);
        assert_eq!(fact.value, "WS 已过期");
        assert!(fact.detail.contains("11.4s"));
        assert_eq!(fact.tone, "is-warning");
    }

    #[test]
    fn listing_evidence_never_reuses_stale_market_runtime() {
        let pair = OnchainCexPairOption {
            venue: "binance".to_owned(),
            base_token: "SOL".to_owned(),
            quote_token: "USDC".to_owned(),
            cex_symbol: "SOL/USDC".to_owned(),
            native_symbol: "SOLUSDC".to_owned(),
            quality: MarketDataQuality::StaleAllowed,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(222_000),
            observed_at_ms: 1,
        };

        let evidence = pair_evidence(&pair);

        assert_eq!(evidence, "官方 Spot registry · 原生代码 SOLUSDC");
        assert!(!evidence.contains("222"));
        assert!(!evidence.contains("允许陈旧"));
    }

    #[test]
    fn ws_failure_detail_is_not_repeated_as_the_next_step() {
        let listing = PairFact {
            value: "官方已挂牌".to_owned(),
            detail: "官方目录数据依据".to_owned(),
            tone: "is-positive",
        };
        let market = PairFact {
            value: "WS 重连中".to_owned(),
            detail: "Kraken websocket disconnected".to_owned(),
            tone: "is-warning",
        };
        let context = PairContext {
            selected_symbol: "SOL/USDC",
            base_token: "SOL",
            chain_quote: "USDC",
            base_identity_resolved: true,
            quote_identity_resolved: true,
        };

        let (note, tone) = pair_next_step(None, None, context, &listing, &market, &listing);

        assert!(!note.contains("disconnected"));
        assert!(note.contains("无需重新应用配置"));
        assert_eq!(tone, "is-warning");
    }
}
