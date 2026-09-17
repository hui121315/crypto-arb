use crate::state::AppState;
use onchain_monitor::{
    OnchainDexCrossQuoteSet, OnchainDexCrossRouteQuote, OnchainQuotePair, ProviderQuote,
};
use shared_types::{
    OnchainComparisonConfig, OnchainComparisonSnapshot, OnchainDexComparisonDirection,
    OnchainDexComparisonQuality, OnchainDexComparisonSnapshot, OnchainDexRouteComparison,
    OnchainDexRouteIdentity,
};

use super::quote::{fetch_exact_in, raw_units};

const OKX_QUOTE_GAP_MS: u64 = 1_000;

pub(super) async fn fetch(
    config: &OnchainComparisonConfig,
    primary: &OnchainQuotePair,
    started_at_ms: i64,
) -> Result<OnchainDexCrossQuoteSet, String> {
    if !config.dex_comparison.enabled {
        return Err("DEX cross comparison is disabled".to_owned());
    }
    if !primary.matches_config(config) {
        return Err("primary DEX quote does not match the active token scope".to_owned());
    }
    let peer_provider = config.dex_comparison.peer_provider.as_str();
    let peer_reverse = fetch_exact_in(
        config,
        peer_provider,
        &config.quote_mint,
        &config.base_mint,
        &config.quote_amount_raw,
    )
    .await?;
    ensure_positive_quote(&peer_reverse, &config.quote_amount_raw, "peer buy quote")?;
    ensure_positive_quote(
        &primary.reverse,
        &config.quote_amount_raw,
        "primary buy quote",
    )?;

    let peer_sell =
        exact_sell_after_provider_gap(config, peer_provider, &primary.reverse.output_amount_raw);
    let primary_sell =
        exact_sell_after_provider_gap(config, &config.provider, &peer_reverse.output_amount_raw);
    let (peer_sell, primary_sell) = tokio::join!(peer_sell, primary_sell);
    let peer_sell = peer_sell?;
    let primary_sell = primary_sell?;
    ensure_positive_quote(
        &peer_sell,
        &primary.reverse.output_amount_raw,
        "peer sell quote",
    )?;
    ensure_positive_quote(
        &primary_sell,
        &peer_reverse.output_amount_raw,
        "primary sell quote",
    )?;

    let observed_at_ms = common::time::now_ms();
    Ok(OnchainDexCrossQuoteSet {
        chain: config.chain.clone(),
        base_address: config.base_mint.clone(),
        quote_address: config.quote_mint.clone(),
        primary_provider: config.provider.clone(),
        peer_provider: peer_provider.to_owned(),
        routes: vec![
            OnchainDexCrossRouteQuote {
                direction: OnchainDexComparisonDirection::BuyPrimarySellPeer,
                buy: primary.reverse.clone(),
                sell: peer_sell,
            },
            OnchainDexCrossRouteQuote {
                direction: OnchainDexComparisonDirection::BuyPeerSellPrimary,
                buy: peer_reverse,
                sell: primary_sell,
            },
        ],
        observed_at_ms,
        request_latency_ms: observed_at_ms.saturating_sub(started_at_ms),
    })
}

async fn exact_sell_after_provider_gap(
    config: &OnchainComparisonConfig,
    provider: &str,
    base_amount_raw: &str,
) -> Result<ProviderQuote, String> {
    if provider == "okx_dex_v6" {
        tokio::time::sleep(std::time::Duration::from_millis(OKX_QUOTE_GAP_MS)).await;
    }
    fetch_exact_in(
        config,
        provider,
        &config.base_mint,
        &config.quote_mint,
        base_amount_raw,
    )
    .await
}

fn ensure_positive_quote(
    quote: &ProviderQuote,
    expected_input: &str,
    label: &str,
) -> Result<(), String> {
    if quote.input_amount_raw != expected_input {
        return Err(format!("{label} changed the exact input amount"));
    }
    if !quote
        .output_amount_raw
        .parse::<u128>()
        .is_ok_and(|amount| amount > 0)
    {
        return Err(format!("{label} returned an invalid output amount"));
    }
    Ok(())
}

pub(super) fn attach(state: &AppState, snapshot: &mut OnchainComparisonSnapshot, now_ms: i64) {
    let config = &snapshot.config;
    if !config.dex_comparison.enabled {
        snapshot.dex_comparison = disabled_snapshot(config, now_ms);
        return;
    }
    let Some(quotes) = state.onchain_monitor().dex_cross_quotes() else {
        snapshot.dex_comparison = unavailable_snapshot(
            config,
            state
                .onchain_monitor()
                .dex_cross_problem()
                .as_deref()
                .map(String::as_str),
            now_ms,
        );
        return;
    };
    if !quotes.matches_config(config) {
        snapshot.dex_comparison = unavailable_snapshot(
            config,
            Some("缓存中的 DEX 对比报价不属于当前资产配置，正在重新取证"),
            now_ms,
        );
        return;
    }
    let usd_rate = super::usd_valuation::quote_evidence(state, config, now_ms)
        .ok()
        .map(|row| row.usd_bid);
    snapshot.dex_comparison = project(config, &quotes, usd_rate, now_ms);
}

pub(super) fn attach_batch(
    state: &AppState,
    item_id: &str,
    snapshot: &mut OnchainComparisonSnapshot,
    now_ms: i64,
) {
    let config = &snapshot.config;
    if !config.dex_comparison.enabled {
        snapshot.dex_comparison = disabled_snapshot(config, now_ms);
        return;
    }
    let Some(quotes) = state.onchain_monitor().batch().dex_cross_quotes(item_id) else {
        let problem = state.onchain_monitor().batch().dex_cross_problem(item_id);
        snapshot.dex_comparison = unavailable_snapshot(config, problem.as_deref(), now_ms);
        return;
    };
    if quotes.matches_config(config) {
        let usd_rate = super::usd_valuation::quote_evidence(state, config, now_ms)
            .ok()
            .map(|row| row.usd_bid);
        snapshot.dex_comparison = project(config, &quotes, usd_rate, now_ms);
    } else {
        snapshot.dex_comparison = unavailable_snapshot(
            config,
            Some("批量监控中的 DEX 对比报价不属于当前资产配置，正在重新取证"),
            now_ms,
        );
    }
}

fn disabled_snapshot(
    config: &OnchainComparisonConfig,
    now_ms: i64,
) -> OnchainDexComparisonSnapshot {
    OnchainDexComparisonSnapshot {
        primary_provider: config.provider.clone(),
        peer_provider: config.dex_comparison.peer_provider.clone(),
        quality: OnchainDexComparisonQuality::Disabled,
        observed_at_ms: now_ms,
        ..Default::default()
    }
}

fn unavailable_snapshot(
    config: &OnchainComparisonConfig,
    problem: Option<&str>,
    now_ms: i64,
) -> OnchainDexComparisonSnapshot {
    OnchainDexComparisonSnapshot {
        primary_provider: config.provider.clone(),
        peer_provider: config.dex_comparison.peer_provider.clone(),
        quality: if problem.is_some() {
            OnchainDexComparisonQuality::UpstreamUnavailable
        } else {
            OnchainDexComparisonQuality::Pending
        },
        problem: problem.map(str::to_owned),
        observed_at_ms: now_ms,
        ..Default::default()
    }
}

pub(super) fn project(
    config: &OnchainComparisonConfig,
    quotes: &OnchainDexCrossQuoteSet,
    usd_rate: Option<f64>,
    now_ms: i64,
) -> OnchainDexComparisonSnapshot {
    let age_ms = now_ms.saturating_sub(quotes.observed_at_ms).max(0);
    let routes = quotes
        .routes
        .iter()
        .filter_map(|route| project_route(config, quotes, route, usd_rate))
        .collect::<Vec<_>>();
    let quality = classify(config, &routes, age_ms);
    let problem = match quality {
        OnchainDexComparisonQuality::DuplicateRoute => {
            Some("两个聚合器命中了相同底层路由，已去重，不视为套利".to_owned())
        }
        OnchainDexComparisonQuality::EvidencePending => {
            Some("报价存在名义净差，但底层池路由或 Gas 换算证据不足，只发送监控提醒".to_owned())
        }
        OnchainDexComparisonQuality::Stale => {
            Some("DEX 对比报价已过期，等待下一轮同数量报价".to_owned())
        }
        _ => None,
    };
    OnchainDexComparisonSnapshot {
        primary_provider: quotes.primary_provider.clone(),
        peer_provider: quotes.peer_provider.clone(),
        quality,
        routes,
        quote_observed_at_ms: Some(quotes.observed_at_ms),
        quote_latency_ms: Some(quotes.request_latency_ms),
        problem,
        observed_at_ms: now_ms,
    }
}

fn project_route(
    config: &OnchainComparisonConfig,
    quotes: &OnchainDexCrossQuoteSet,
    route: &OnchainDexCrossRouteQuote,
    usd_rate: Option<f64>,
) -> Option<OnchainDexRouteComparison> {
    let input = route.buy.input_amount_raw.parse::<u128>().ok()?;
    let output = route.sell.output_amount_raw.parse::<u128>().ok()?;
    if input == 0 || output == 0 || route.sell.input_amount_raw != route.buy.output_amount_raw {
        return None;
    }
    let (buy_provider, sell_provider) = match route.direction {
        OnchainDexComparisonDirection::BuyPrimarySellPeer => {
            (&quotes.primary_provider, &quotes.peer_provider)
        }
        OnchainDexComparisonDirection::BuyPeerSellPrimary => {
            (&quotes.peer_provider, &quotes.primary_provider)
        }
    };
    let gross_return_bps = (output as f64 / input as f64 - 1.0) * 10_000.0;
    let execution_buffer_bps = config.slippage_bps.max(0.0) * 2.0;
    let gas_usd = config.gas_usd.max(0.0) * 2.0;
    let gas_bps = usd_rate
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .and_then(|rate| {
            raw_units(&route.buy.input_amount_raw, config.quote_decimals)
                .map(|amount| amount * rate)
        })
        .filter(|notional| notional.is_finite() && *notional > 0.0)
        .map(|notional| gas_usd / notional * 10_000.0);
    let total_cost_bps = gas_bps.map(|gas_bps| gas_bps + execution_buffer_bps);
    let net_return_bps = total_cost_bps.map(|cost| gross_return_bps - cost);
    let route_identity = route_identity(&route.buy.router, &route.sell.router);
    let profitable =
        net_return_bps.is_some_and(|net| net >= config.spread_alert.min_net_spread_bps.max(0.0));
    let executable = profitable && route_identity == OnchainDexRouteIdentity::ProvenDistinct;
    let problem = if gas_bps.is_none() {
        Some("缺少新鲜 Quote/USD 估值汇率，不能将美元 Gas 直接扣减报价币收益".to_owned())
    } else if route_identity == OnchainDexRouteIdentity::Duplicate {
        Some("两个报价命中相同底层池或路由，已去重".to_owned())
    } else if route_identity == OnchainDexRouteIdentity::Unknown {
        Some("聚合器未返回可核验的底层池标识，只能监控".to_owned())
    } else if executable {
        Some("两笔链上交易尚未生成原子执行计划，当前保持监控态".to_owned())
    } else {
        None
    };
    Some(OnchainDexRouteComparison {
        direction: route.direction,
        buy_provider: buy_provider.clone(),
        sell_provider: sell_provider.clone(),
        input_quote_amount_raw: route.buy.input_amount_raw.clone(),
        acquired_base_amount_raw: route.buy.output_amount_raw.clone(),
        output_quote_amount_raw: route.sell.output_amount_raw.clone(),
        gross_return_bps,
        execution_buffer_bps,
        gas_usd,
        gas_bps,
        total_cost_bps,
        net_return_bps,
        buy_router: route.buy.router.clone(),
        sell_router: route.sell.router.clone(),
        route_identity,
        executable: false,
        problem,
        observed_at_ms: quotes.observed_at_ms,
    })
}

fn classify(
    config: &OnchainComparisonConfig,
    routes: &[OnchainDexRouteComparison],
    age_ms: i64,
) -> OnchainDexComparisonQuality {
    if age_ms > config.max_age_ms {
        return OnchainDexComparisonQuality::Stale;
    }
    let threshold = config.spread_alert.min_net_spread_bps.max(0.0);
    let profitable = routes
        .iter()
        .filter(|route| route.net_return_bps.is_some_and(|net| net >= threshold))
        .collect::<Vec<_>>();
    if profitable.is_empty() {
        return OnchainDexComparisonQuality::NoNetProfit;
    }
    if profitable
        .iter()
        .all(|route| route.route_identity == OnchainDexRouteIdentity::Duplicate)
    {
        return OnchainDexComparisonQuality::DuplicateRoute;
    }
    if profitable.iter().any(|route| {
        route.gas_bps.is_none() || route.route_identity == OnchainDexRouteIdentity::Unknown
    }) {
        return OnchainDexComparisonQuality::EvidencePending;
    }
    OnchainDexComparisonQuality::Fresh
}

fn route_identity(
    buy_router: &Option<String>,
    sell_router: &Option<String>,
) -> OnchainDexRouteIdentity {
    let buy = pool_identifiers(buy_router.as_deref().unwrap_or_default());
    let sell = pool_identifiers(sell_router.as_deref().unwrap_or_default());
    if buy.is_empty() || sell.is_empty() {
        return OnchainDexRouteIdentity::Unknown;
    }
    if buy.iter().any(|identifier| sell.contains(identifier)) {
        OnchainDexRouteIdentity::Duplicate
    } else {
        OnchainDexRouteIdentity::ProvenDistinct
    }
}

fn pool_identifiers(router: &str) -> Vec<String> {
    let mut identifiers = router
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter_map(|token| {
            let evm = token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
                .is_some_and(|address| {
                    address.len() == 40 && address.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                || (token.len() == 40 && token.bytes().all(|byte| byte.is_ascii_hexdigit()));
            let solana = (32..=44).contains(&token.len())
                && token.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() && !matches!(byte, b'0' | b'O' | b'I' | b'l')
                });
            (evm || solana).then(|| token.to_ascii_lowercase())
        })
        .collect::<Vec<_>>();
    identifiers.sort();
    identifiers.dedup();
    identifiers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(input: &str, output: &str, router: Option<&str>) -> ProviderQuote {
        ProviderQuote {
            input_address: "input".to_owned(),
            output_address: "output".to_owned(),
            input_amount_raw: input.to_owned(),
            output_amount_raw: output.to_owned(),
            router: router.map(str::to_owned),
        }
    }

    fn quote_set(
        output: &str,
        buy_router: Option<&str>,
        sell_router: Option<&str>,
    ) -> OnchainDexCrossQuoteSet {
        let config = OnchainComparisonConfig {
            chain: "base".to_owned(),
            provider: "zeroex_swap_v2".to_owned(),
            base_mint: "base".to_owned(),
            quote_mint: "quote".to_owned(),
            ..OnchainComparisonConfig::default()
        };
        OnchainDexCrossQuoteSet {
            chain: config.chain,
            base_address: config.base_mint,
            quote_address: config.quote_mint,
            primary_provider: config.provider,
            peer_provider: "okx_dex_v6".to_owned(),
            routes: vec![OnchainDexCrossRouteQuote {
                direction: OnchainDexComparisonDirection::BuyPrimarySellPeer,
                buy: quote("100000000", "1000000000000000000", buy_router),
                sell: quote("1000000000000000000", output, sell_router),
            }],
            observed_at_ms: 1_000,
            request_latency_ms: 25,
        }
    }

    fn config() -> OnchainComparisonConfig {
        OnchainComparisonConfig {
            enabled: true,
            chain: "base".to_owned(),
            provider: "zeroex_swap_v2".to_owned(),
            quote_token: "USDC".to_owned(),
            quote_decimals: 6,
            quote_amount_raw: "100000000".to_owned(),
            gas_usd: 0.1,
            slippage_bps: 5.0,
            dex_comparison: shared_types::OnchainDexComparisonConfig {
                enabled: true,
                peer_provider: "okx_dex_v6".to_owned(),
            },
            ..OnchainComparisonConfig::default()
        }
    }

    #[test]
    fn exact_round_trip_subtracts_two_gas_and_execution_buffers() {
        let quotes = quote_set(
            "101000000",
            Some("pool 0x1111111111111111111111111111111111111111"),
            Some("pool 0x2222222222222222222222222222222222222222"),
        );
        let snapshot = project(&config(), &quotes, Some(1.0), 1_010);
        let route = &snapshot.routes[0];

        assert_eq!(snapshot.quality, OnchainDexComparisonQuality::Fresh);
        assert!((route.gross_return_bps - 100.0).abs() < 0.001);
        assert!((route.gas_bps.unwrap_or_default() - 20.0).abs() < 0.001);
        assert!((route.net_return_bps.unwrap_or_default() - 70.0).abs() < 0.001);
        assert!(!route.executable, "two-swap submission remains gated");
    }

    #[test]
    fn shared_pool_is_deduplicated_and_unknown_route_stays_monitor_only() {
        let same_pool = "pool 0x1111111111111111111111111111111111111111";
        let duplicate = project(
            &config(),
            &quote_set("101000000", Some(same_pool), Some(same_pool)),
            Some(1.0),
            1_010,
        );
        assert_eq!(
            duplicate.quality,
            OnchainDexComparisonQuality::DuplicateRoute
        );

        let unknown = project(
            &config(),
            &quote_set("101000000", Some("0x sources"), Some("OKX router")),
            Some(1.0),
            1_010,
        );
        assert_eq!(
            unknown.quality,
            OnchainDexComparisonQuality::EvidencePending
        );
        assert_eq!(
            unknown.routes[0].route_identity,
            OnchainDexRouteIdentity::Unknown
        );
    }

    #[test]
    fn non_stable_quote_does_not_invent_a_usd_gas_conversion() {
        let mut config = config();
        config.quote_token = "WETH".to_owned();
        let snapshot = project(&config, &quote_set("101000000", None, None), None, 1_010);

        assert!(snapshot.routes[0].gas_bps.is_none());
        assert!(snapshot.routes[0].net_return_bps.is_none());
        let valued = project(
            &config,
            &quote_set("101000000", None, None),
            Some(0.5),
            1_010,
        );
        assert!((valued.routes[0].gas_bps.unwrap() - 40.0).abs() < 1e-9);
    }
}
