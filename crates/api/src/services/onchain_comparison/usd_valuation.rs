use crate::services::market_data::{MarketQuality, MarketRead, MarketSource};
use crate::services::market_subscriptions::MarketSubscriptionFeed;
use crate::state::AppState;
use exchange::PublicWsSnapshot;
use shared_types::{OnchainComparisonConfig, OnchainUsdValuation, OrderBookInfo};
use std::collections::{BTreeMap, BTreeSet};

struct Target {
    venue: String,
    symbol: String,
    inverse: bool,
}

fn targets(
    state: &AppState,
    config: &OnchainComparisonConfig,
    asset: &str,
    now_ms: i64,
) -> Vec<Target> {
    let mut targets = Vec::new();
    let mut seen = BTreeSet::new();
    for venue in [config.cex_venue.as_str(), "kraken"] {
        if !seen.insert(venue.to_owned())
            || state.aggregator().get(venue).is_none()
            || !state
                .market_subscriptions()
                .enabled(venue, MarketSubscriptionFeed::Spot)
        {
            continue;
        }
        for (base, quote, inverse) in [(asset, "USD", false), ("USD", asset, true)] {
            if state
                .instrument_registry()
                .exact_spot_listing_evidence(venue, base, quote, now_ms)
                == Some(true)
            {
                targets.push(Target {
                    venue: venue.to_owned(),
                    symbol: format!("{base}/{quote}"),
                    inverse,
                });
                break;
            }
        }
    }
    targets
}

pub(super) async fn refresh(state: &AppState, config: &OnchainComparisonConfig) {
    let assets = [
        Some(config.quote_token.as_str()),
        shared_types::onchain_cex_quote_token(&config.cex_symbol),
    ]
    .into_iter()
    .flatten()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    refresh_assets(state, config, &assets).await;
}

pub(super) async fn refresh_assets(
    state: &AppState,
    config: &OnchainComparisonConfig,
    assets: &[String],
) {
    let now_ms = common::time::now_ms();
    let mut subscriptions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for asset in assets {
        let asset = asset.trim().to_ascii_uppercase();
        if asset == "USD" {
            continue;
        }
        // Reading cached prices does not renew the adapter's demand lease.
        // Keep the bounded valuation subscriptions alive while monitoring.
        for target in targets(state, config, &asset, now_ms) {
            subscriptions
                .entry(target.venue)
                .or_default()
                .insert(target.symbol);
        }
    }
    for (venue, symbols) in subscriptions {
        if let Some(adapter) = state.aggregator().get(&venue) {
            if let Ok(PublicWsSnapshot::Ready(rows)) = adapter
                .public_ws_spot_snapshot(&symbols.into_iter().collect::<Vec<_>>())
                .await
            {
                state
                    .market_data()
                    .store_spot_ticks(&rows, MarketSource::WsPush);
            }
        }
    }
}

pub(super) fn quote_evidence(
    state: &AppState,
    config: &OnchainComparisonConfig,
    now_ms: i64,
) -> Result<OnchainUsdValuation, String> {
    evidence(state, config, &config.quote_token, now_ms)
}

pub(super) fn evidence(
    state: &AppState,
    config: &OnchainComparisonConfig,
    asset: &str,
    now_ms: i64,
) -> Result<OnchainUsdValuation, String> {
    let asset = asset.trim().to_ascii_uppercase();
    if asset == "USD" {
        return Ok(OnchainUsdValuation {
            asset,
            venue: String::new(),
            symbol: "USD/USD".to_owned(),
            source: "same_currency".to_owned(),
            usd_bid: 1.0,
            usd_ask: 1.0,
            observed_at_ms: now_ms,
        });
    }
    for target in targets(state, config, &asset, now_ms) {
        let read = state.market_data().spot_bbo_read(
            &target.venue,
            &target.symbol,
            now_ms,
            config.max_age_ms,
        );
        if let Ok(evidence) = project(&asset, &target, read, config.max_age_ms, now_ms) {
            return Ok(evidence);
        }
    }
    Err(format!("{asset}/USD 估值汇率未就绪：需要当前交易所或 Kraken 已启用的官方现货 WS；不会将稳定币默认按 1 美元计价"))
}

fn project(
    asset: &str,
    target: &Target,
    read: MarketRead<OrderBookInfo>,
    max_age_ms: i64,
    now_ms: i64,
) -> Result<OnchainUsdValuation, String> {
    let book = read.value.ok_or("美元估值盘口缺失")?;
    let age = read.freshness_ms.ok_or("美元估值时效未知")?;
    if read.source != MarketSource::WsPush
        || read.quality != MarketQuality::Fresh
        || age < 0
        || age > max_age_ms
        || book.timestamp > now_ms
        || now_ms.saturating_sub(book.timestamp) > max_age_ms
        || !book.exchange.eq_ignore_ascii_case(&target.venue)
        || onchain_monitor::normalized_pair_symbol(&book.symbol)
            != onchain_monitor::normalized_pair_symbol(&target.symbol)
    {
        return Err("美元估值不是同一市场的新鲜官方 WS".to_owned());
    }
    let bid = book.best_bid().ok_or("美元估值买价缺失")?;
    let ask = book.best_ask().ok_or("美元估值卖价缺失")?;
    let (usd_bid, usd_ask) = if target.inverse {
        (1.0 / ask, 1.0 / bid)
    } else {
        (bid, ask)
    };
    if !usd_bid.is_finite() || !usd_ask.is_finite() || usd_bid <= 0.0 || usd_bid > usd_ask {
        return Err("美元估值买卖价无效".to_owned());
    }
    Ok(OnchainUsdValuation {
        asset: asset.to_owned(),
        venue: target.venue.clone(),
        symbol: target.symbol.clone(),
        source: "ws_push".to_owned(),
        usd_bid,
        usd_ask,
        observed_at_ms: book.timestamp.min(now_ms.saturating_sub(age)),
    })
}

pub(super) fn rate(
    evidence: Option<&OnchainUsdValuation>,
    asset: &str,
    max_age_ms: i64,
    now_ms: i64,
) -> Option<f64> {
    let evidence = evidence?;
    let identity = evidence.source == "same_currency"
        && asset.eq_ignore_ascii_case("USD")
        && evidence.usd_bid == 1.0
        && evidence.usd_ask == 1.0;
    (evidence.asset.eq_ignore_ascii_case(asset.trim())
        && (identity || evidence.source == "ws_push")
        && now_ms >= evidence.observed_at_ms
        && now_ms.saturating_sub(evidence.observed_at_ms) <= max_age_ms
        && evidence.usd_bid.is_finite()
        && evidence.usd_bid > 0.0
        && evidence.usd_ask.is_finite()
        && evidence.usd_ask >= evidence.usd_bid)
        .then_some(evidence.usd_bid)
}

#[cfg(test)]
pub(super) fn fixture(asset: &str, rate: f64, now_ms: i64) -> OnchainUsdValuation {
    OnchainUsdValuation {
        asset: asset.to_owned(),
        venue: "kraken".to_owned(),
        symbol: format!("{asset}/USD"),
        source: "ws_push".to_owned(),
        usd_bid: rate,
        usd_ask: rate,
        observed_at_ms: now_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read() -> MarketRead<OrderBookInfo> {
        MarketRead {
            value: Some(OrderBookInfo {
                exchange: "kraken".to_owned(),
                symbol: "USDC/USD".to_owned(),
                bids: vec![[0.9, 1000.0]],
                asks: vec![[1.1, 1000.0]],
                timestamp: 950,
            }),
            quality: MarketQuality::Fresh,
            freshness_ms: Some(50),
            source: MarketSource::WsPush,
            retry_after_ms: None,
            last_error: None,
        }
    }

    #[test]
    fn usd_valuation_uses_bid_and_ask_instead_of_a_stablecoin_peg() {
        let target = Target {
            venue: "kraken".to_owned(),
            symbol: "USDC/USD".to_owned(),
            inverse: false,
        };
        let result = project("USDC", &target, read(), 100, 1000).unwrap();
        assert_eq!(result.usd_bid, 0.9);
        assert_eq!(result.usd_ask, 1.1);
        assert_eq!(rate(Some(&result), "USDC", 100, 1000), Some(0.9));
        assert_eq!(rate(Some(&result), "USDT", 100, 1000), None);
        assert_eq!(rate(Some(&result), "USDC", 100, 1051), None);
        let mut inverse = read();
        inverse.value.as_mut().unwrap().symbol = "USD/USDC".to_owned();
        let target = Target {
            symbol: "USD/USDC".to_owned(),
            inverse: true,
            ..target
        };
        let result = project("USDC", &target, inverse, 100, 1000).unwrap();
        assert_eq!(result.usd_bid, 1.0 / 1.1);
        assert_eq!(result.usd_ask, 1.0 / 0.9);
    }

    #[test]
    fn usd_valuation_rejects_rest_stale_future_and_wrong_market_evidence() {
        let target = Target {
            venue: "kraken".to_owned(),
            symbol: "USDC/USD".to_owned(),
            inverse: false,
        };
        for case in 0..6 {
            let mut input = read();
            match case {
                0 => input.source = MarketSource::LocalCache,
                1 => input.freshness_ms = Some(101),
                2 => input.value.as_mut().unwrap().timestamp = 1001,
                3 => input.value.as_mut().unwrap().symbol = "USDT/USD".to_owned(),
                4 => input.value.as_mut().unwrap().exchange = "other".to_owned(),
                _ => input.value.as_mut().unwrap().bids[0][0] = 1.2,
            }
            assert!(project("USDC", &target, input, 100, 1000).is_err());
        }
        let mut invented = fixture("USDT", 1.0, 1000);
        invented.source = "same_currency".to_owned();
        assert_eq!(rate(Some(&invented), "USDT", 100, 1000), None);
    }
}
