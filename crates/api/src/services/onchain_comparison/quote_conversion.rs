use crate::services::market_data::{MarketQuality, MarketRead, MarketSource};
use crate::state::AppState;
use shared_types::{OnchainComparisonConfig, OnchainQuoteConversionEvidence, OrderBookInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversionOrientation {
    Direct,
    Inverse,
}

#[derive(Debug, Clone)]
pub(super) struct QuoteConversionTarget {
    pub symbol: String,
    orientation: ConversionOrientation,
    cex_quote: String,
    onchain_quote: String,
}

pub(super) fn target(
    state: &AppState,
    config: &OnchainComparisonConfig,
    now_ms: i64,
) -> Option<QuoteConversionTarget> {
    let cex_quote = shared_types::onchain_cex_quote_token(&config.cex_symbol)?
        .trim()
        .to_ascii_uppercase();
    let onchain_quote = config.quote_token.trim().to_ascii_uppercase();
    if cex_quote.is_empty()
        || onchain_quote.is_empty()
        || cex_quote == onchain_quote
        || !config.quote_identity_resolved
    {
        return None;
    }
    let registry = state.instrument_registry();
    let direct =
        registry.exact_spot_listing_evidence(&config.cex_venue, &cex_quote, &onchain_quote, now_ms);
    if direct == Some(true) {
        return Some(QuoteConversionTarget {
            symbol: format!("{cex_quote}/{onchain_quote}"),
            orientation: ConversionOrientation::Direct,
            cex_quote,
            onchain_quote,
        });
    }
    let inverse =
        registry.exact_spot_listing_evidence(&config.cex_venue, &onchain_quote, &cex_quote, now_ms);
    (inverse == Some(true)).then(|| QuoteConversionTarget {
        symbol: format!("{onchain_quote}/{cex_quote}"),
        orientation: ConversionOrientation::Inverse,
        cex_quote,
        onchain_quote,
    })
}

pub(super) fn evidence(
    state: &AppState,
    config: &OnchainComparisonConfig,
    now_ms: i64,
) -> Result<Option<OnchainQuoteConversionEvidence>, String> {
    let Some(cex_quote) = shared_types::onchain_cex_quote_token(&config.cex_symbol) else {
        return Err("CEX 交易对缺少明确的 Quote 资产".to_owned());
    };
    if cex_quote.eq_ignore_ascii_case(config.quote_token.trim()) {
        return Ok(None);
    }
    if !config.quote_identity_resolved {
        return Err(format!(
            "链上 Quote {} 的合约身份尚未核验，不能选择换算市场",
            config.quote_token
        ));
    }
    let Some(target) = target(state, config, now_ms) else {
        return Err(format!(
            "{} 官方 Spot registry 没有 {}/{} 或 {}/{} 换算交易对",
            config.cex_venue.to_uppercase(),
            cex_quote,
            config.quote_token,
            config.quote_token,
            cex_quote,
        ));
    };
    let read = state.market_data().spot_bbo_read(
        &config.cex_venue,
        &target.symbol,
        now_ms,
        config.max_age_ms,
    );
    let read = super::ws_only_spot_bbo(read, &config.cex_venue, &target.symbol);
    project_evidence(config, target, read, now_ms).map(Some)
}

fn project_evidence(
    config: &OnchainComparisonConfig,
    target: QuoteConversionTarget,
    read: MarketRead<OrderBookInfo>,
    now_ms: i64,
) -> Result<OnchainQuoteConversionEvidence, String> {
    let book = read.value.ok_or_else(|| {
        read.last_error.unwrap_or_else(|| {
            format!(
                "{} {} 换算 WS 正在等待首个最优买卖价",
                config.cex_venue.to_uppercase(),
                target.symbol
            )
        })
    })?;
    if read.source != MarketSource::WsPush || read.quality != MarketQuality::Fresh {
        return Err(format!(
            "{} {} 换算盘口不是新鲜官方 WS 数据",
            config.cex_venue.to_uppercase(),
            target.symbol
        ));
    }
    let (Some(source_bid), Some(source_ask)) = (book.best_bid(), book.best_ask()) else {
        return Err(format!(
            "{} {} 换算盘口缺少有效买一或卖一",
            config.cex_venue.to_uppercase(),
            target.symbol
        ));
    };
    let source_bid_size = book.bids.first().map_or(0.0, |level| level[1]);
    let source_ask_size = book.asks.first().map_or(0.0, |level| level[1]);
    let (cex_to_onchain_bid, cex_to_onchain_ask) = match target.orientation {
        ConversionOrientation::Direct => (source_bid, source_ask),
        ConversionOrientation::Inverse => (1.0 / source_ask, 1.0 / source_bid),
    };
    let (cex_to_onchain_capacity, onchain_to_cex_capacity) = match target.orientation {
        ConversionOrientation::Direct => {
            (source_bid_size * source_bid, source_ask_size * source_ask)
        }
        ConversionOrientation::Inverse => (source_ask_size, source_bid_size),
    };
    if !cex_to_onchain_bid.is_finite()
        || !cex_to_onchain_ask.is_finite()
        || cex_to_onchain_bid <= 0.0
        || cex_to_onchain_ask <= 0.0
        || cex_to_onchain_bid > cex_to_onchain_ask
        || !cex_to_onchain_capacity.is_finite()
        || !onchain_to_cex_capacity.is_finite()
        || cex_to_onchain_capacity <= 0.0
        || onchain_to_cex_capacity <= 0.0
    {
        return Err(format!(
            "{} {} 换算后的双向汇率无效",
            config.cex_venue.to_uppercase(),
            target.symbol
        ));
    }
    let freshness_ms = read.freshness_ms.unwrap_or(i64::MAX);
    Ok(OnchainQuoteConversionEvidence {
        venue: config.cex_venue.clone(),
        symbol: target.symbol,
        source: read.source.as_str().to_owned(),
        cex_quote: target.cex_quote,
        onchain_quote: target.onchain_quote,
        source_bid,
        source_ask,
        cex_to_onchain_bid,
        cex_to_onchain_ask,
        cex_to_onchain_capacity,
        onchain_to_cex_capacity,
        freshness_ms,
        observed_at_ms: now_ms.saturating_sub(freshness_ms.max(0)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(symbol: &str, bid: f64, ask: f64) -> MarketRead<OrderBookInfo> {
        MarketRead {
            value: Some(OrderBookInfo {
                symbol: symbol.to_owned(),
                exchange: "kraken".to_owned(),
                bids: vec![[bid, 10_000.0]],
                asks: vec![[ask, 10_000.0]],
                timestamp: 900,
            }),
            quality: MarketQuality::Fresh,
            freshness_ms: Some(100),
            source: MarketSource::WsPush,
            retry_after_ms: None,
            last_error: None,
        }
    }

    #[test]
    fn inverse_market_uses_ask_for_proceeds_and_bid_for_cost() {
        let config = OnchainComparisonConfig {
            cex_venue: "kraken".to_owned(),
            ..OnchainComparisonConfig::default()
        };
        let target = QuoteConversionTarget {
            symbol: "USDC/USD".to_owned(),
            orientation: ConversionOrientation::Inverse,
            cex_quote: "USD".to_owned(),
            onchain_quote: "USDC".to_owned(),
        };
        let evidence = project_evidence(&config, target, read("USDC/USD", 0.999, 1.001), 1_000)
            .expect("fresh WS conversion should project");
        assert_eq!(evidence.cex_to_onchain_bid, 1.0 / 1.001);
        assert_eq!(evidence.cex_to_onchain_ask, 1.0 / 0.999);
        assert!(evidence.cex_to_onchain_bid < evidence.cex_to_onchain_ask);
        assert_eq!(evidence.cex_to_onchain_capacity, 10_000.0);
        assert_eq!(evidence.onchain_to_cex_capacity, 10_000.0);
    }
}
