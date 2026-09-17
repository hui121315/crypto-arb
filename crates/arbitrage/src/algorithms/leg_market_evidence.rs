use crate::algorithms::market;
use crate::models::{FundingMarketEvidence, RawOpportunity};
use shared_types::{
    normalized_venue_name, ArbitrageType, MarketDataRowEvidence, MarketDataSnapshotOperation,
    MarketDataSnapshotStatus, OpportunityLegMarketEvidence, OpportunityQuoteConversion, OrderSide,
    SpotLegMode,
};
use std::collections::HashMap;
use std::hash::Hash;

pub(crate) fn attach_leg_market_evidence(
    rows: &mut [RawOpportunity],
    status: Option<&MarketDataSnapshotStatus>,
    funding_row_evidence: &[MarketDataRowEvidence],
    perp_ticker_row_evidence: &[MarketDataRowEvidence],
    spot_tick_row_evidence: &[MarketDataRowEvidence],
) {
    if status.is_none()
        && funding_row_evidence.is_empty()
        && perp_ticker_row_evidence.is_empty()
        && spot_tick_row_evidence.is_empty()
    {
        return;
    };
    let _ = status;
    let row_index = row_evidence_index(
        funding_row_evidence,
        perp_ticker_row_evidence,
        spot_tick_row_evidence,
    );
    for row in rows {
        attach_row(row, &row_index);
    }
}

type RowEvidenceKey = (String, String, MarketDataSnapshotOperation);
type PerpIdentityKey = (String, String, Option<String>);

#[derive(Default)]
struct RowEvidenceIndex<'a> {
    exact: HashMap<RowEvidenceKey, &'a MarketDataRowEvidence>,
    perp_identity: HashMap<PerpIdentityKey, Option<&'a MarketDataRowEvidence>>,
    perp_base: HashMap<(String, String), Option<&'a MarketDataRowEvidence>>,
    funding_identity: HashMap<PerpIdentityKey, Option<&'a MarketDataRowEvidence>>,
    funding_base: HashMap<(String, String), Option<&'a MarketDataRowEvidence>>,
}

impl<'a> RowEvidenceIndex<'a> {
    fn with_capacity(funding_rows: usize, perp_rows: usize, spot_rows: usize) -> Self {
        Self {
            exact: HashMap::with_capacity(
                funding_rows
                    .saturating_add(perp_rows)
                    .saturating_add(spot_rows),
            ),
            perp_identity: HashMap::with_capacity(perp_rows),
            perp_base: HashMap::with_capacity(perp_rows),
            funding_identity: HashMap::with_capacity(funding_rows),
            funding_base: HashMap::with_capacity(funding_rows),
        }
    }

    fn get(
        &self,
        venue: &str,
        symbol: &str,
        market: LegMarket,
    ) -> Option<&'a MarketDataRowEvidence> {
        if let Some(evidence) = self.exact.get(&exact_evidence_key(venue, symbol, market)) {
            return Some(*evidence);
        }
        if market != LegMarket::Perp {
            return None;
        }
        let identity = perp_identity_key(venue, symbol);
        if let Some(evidence) = self.perp_identity.get(&identity) {
            return *evidence;
        }
        if identity.2.is_some() {
            return None;
        }
        self.perp_base
            .get(&perp_base_key(venue, symbol))
            .copied()
            .flatten()
    }

    fn get_funding(&self, venue: &str, symbol: &str) -> Option<&'a MarketDataRowEvidence> {
        if let Some(evidence) = self.exact.get(&operation_evidence_key(
            venue,
            symbol,
            MarketDataSnapshotOperation::FundingRates,
        )) {
            return Some(*evidence);
        }
        let identity = perp_identity_key(venue, symbol);
        if let Some(evidence) = self.funding_identity.get(&identity) {
            return *evidence;
        }
        if identity.2.is_some() {
            return None;
        }
        self.funding_base
            .get(&perp_base_key(venue, symbol))
            .copied()
            .flatten()
    }

    fn insert(&mut self, evidence: &'a MarketDataRowEvidence, market: LegMarket) {
        insert_latest(
            &mut self.exact,
            exact_evidence_key(&evidence.venue, &evidence.symbol, market),
            evidence,
        );
        if market == LegMarket::Perp {
            insert_unique(
                &mut self.perp_identity,
                perp_identity_key(&evidence.venue, &evidence.symbol),
                evidence,
            );
            insert_unique(
                &mut self.perp_base,
                perp_base_key(&evidence.venue, &evidence.symbol),
                evidence,
            );
        }
    }

    fn insert_funding(&mut self, evidence: &'a MarketDataRowEvidence) {
        insert_latest(
            &mut self.exact,
            operation_evidence_key(
                &evidence.venue,
                &evidence.symbol,
                MarketDataSnapshotOperation::FundingRates,
            ),
            evidence,
        );
        insert_unique(
            &mut self.funding_identity,
            perp_identity_key(&evidence.venue, &evidence.symbol),
            evidence,
        );
        insert_unique(
            &mut self.funding_base,
            perp_base_key(&evidence.venue, &evidence.symbol),
            evidence,
        );
    }
}

fn attach_row(row: &mut RawOpportunity, rows: &RowEvidenceIndex<'_>) {
    if row.extra.long_leg_market_evidence.is_none() {
        row.extra.long_leg_market_evidence = leg_evidence(row, OrderSide::Buy, rows);
    }
    if row.extra.short_leg_market_evidence.is_none() {
        row.extra.short_leg_market_evidence = leg_evidence(row, OrderSide::Sell, rows);
    }
    if row.extra.long_funding_evidence.is_none() {
        row.extra.long_funding_evidence =
            funding_evidence(row, OrderSide::Buy, rows).map(compact_funding_evidence);
    }
    if row.extra.short_funding_evidence.is_none() {
        row.extra.short_funding_evidence =
            funding_evidence(row, OrderSide::Sell, rows).map(compact_funding_evidence);
    }
    for conversion in &mut row.extra.quote_conversions {
        if conversion.market_evidence.is_none() {
            conversion.market_evidence = quote_conversion_evidence(conversion, rows);
        }
    }
}

fn compact_funding_evidence(evidence: &MarketDataRowEvidence) -> FundingMarketEvidence {
    FundingMarketEvidence {
        quality: evidence.health.quality,
        source: evidence.health.source,
        observed_at_ms: evidence.health.observed_at_ms,
    }
}

fn funding_evidence<'a>(
    row: &RawOpportunity,
    side: OrderSide,
    rows: &'a RowEvidenceIndex<'a>,
) -> Option<&'a MarketDataRowEvidence> {
    if leg_market(row, side) != Some(LegMarket::Perp) {
        return None;
    }
    let rate = match side {
        OrderSide::Buy => &row.long_rate,
        OrderSide::Sell => &row.short_rate,
    };
    rows.get_funding(leg_venue(row, side), &rate.symbol)
}

fn quote_conversion_evidence(
    conversion: &OpportunityQuoteConversion,
    rows: &RowEvidenceIndex<'_>,
) -> Option<OpportunityLegMarketEvidence> {
    let evidence = rows.get(&conversion.venue, &conversion.symbol, LegMarket::Spot)?;
    Some(OpportunityLegMarketEvidence {
        venue: evidence.venue.clone(),
        symbol: evidence.symbol.clone(),
        price: Some(conversion.rate),
        health: evidence.health.clone(),
    })
}

fn leg_evidence(
    row: &RawOpportunity,
    side: OrderSide,
    rows: &RowEvidenceIndex<'_>,
) -> Option<OpportunityLegMarketEvidence> {
    let market = leg_market(row, side)?;
    let venue = leg_venue(row, side);
    let symbol = leg_symbol(row, side, market);
    if let Some(evidence) = rows.get(venue, symbol, market) {
        return Some(OpportunityLegMarketEvidence {
            venue: evidence.venue.clone(),
            symbol: evidence.symbol.clone(),
            price: leg_price(row, side),
            health: evidence.health.clone(),
        });
    }
    None
}

fn row_evidence_index<'a>(
    funding_rows: &'a [MarketDataRowEvidence],
    perp_ticker_rows: &'a [MarketDataRowEvidence],
    spot_tick_rows: &'a [MarketDataRowEvidence],
) -> RowEvidenceIndex<'a> {
    let mut out = RowEvidenceIndex::with_capacity(
        funding_rows.len(),
        perp_ticker_rows.len(),
        spot_tick_rows.len(),
    );
    for evidence in funding_rows {
        if evidence.operation == MarketDataSnapshotOperation::FundingRates {
            out.insert_funding(evidence);
        }
    }
    for evidence in perp_ticker_rows {
        if evidence.operation == MarketDataSnapshotOperation::PerpTickers {
            out.insert(evidence, LegMarket::Perp);
        }
    }
    for evidence in spot_tick_rows {
        if evidence.operation == MarketDataSnapshotOperation::SpotTicks {
            out.insert(evidence, LegMarket::Spot);
        }
    }
    out
}

fn exact_evidence_key(venue: &str, symbol: &str, market: LegMarket) -> RowEvidenceKey {
    operation_evidence_key(venue, symbol, market.operation())
}

fn operation_evidence_key(
    venue: &str,
    symbol: &str,
    operation: MarketDataSnapshotOperation,
) -> RowEvidenceKey {
    let (venue, symbol) = market::venue_symbol_key(venue, symbol);
    (venue, symbol, operation)
}

fn perp_identity_key(venue: &str, symbol: &str) -> PerpIdentityKey {
    (
        normalized_venue_name(venue),
        market::canonical_base_symbol(symbol),
        market::canonical_perp_quote_symbol(venue, symbol),
    )
}

fn perp_base_key(venue: &str, symbol: &str) -> (String, String) {
    market::venue_base_symbol_key(venue, symbol)
}

fn insert_latest<'a, K: Eq + Hash>(
    rows: &mut HashMap<K, &'a MarketDataRowEvidence>,
    key: K,
    evidence: &'a MarketDataRowEvidence,
) {
    rows.entry(key)
        .and_modify(|current| {
            if evidence.health.observed_at_ms > current.health.observed_at_ms {
                *current = evidence;
            }
        })
        .or_insert(evidence);
}

fn insert_unique<'a, K: Eq + Hash>(
    rows: &mut HashMap<K, Option<&'a MarketDataRowEvidence>>,
    key: K,
    evidence: &'a MarketDataRowEvidence,
) {
    rows.entry(key)
        .and_modify(|current| match current {
            Some(existing)
                if !existing
                    .symbol
                    .trim()
                    .eq_ignore_ascii_case(evidence.symbol.trim()) =>
            {
                *current = None;
            }
            Some(existing) if evidence.health.observed_at_ms > existing.health.observed_at_ms => {
                *current = Some(evidence);
            }
            Some(_) | None => {}
        })
        .or_insert(Some(evidence));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegMarket {
    Perp,
    Spot,
}

impl LegMarket {
    const fn operation(self) -> MarketDataSnapshotOperation {
        match self {
            Self::Perp => MarketDataSnapshotOperation::PerpTickers,
            Self::Spot => MarketDataSnapshotOperation::SpotTicks,
        }
    }
}

fn leg_market(row: &RawOpportunity, side: OrderSide) -> Option<LegMarket> {
    match row.arb_type {
        ArbitrageType::CrossExchange => Some(LegMarket::Perp),
        ArbitrageType::SpotCross => Some(LegMarket::Spot),
        ArbitrageType::SpotFutures | ArbitrageType::CrossSpotFutures => spot_perp_leg(row, side),
        _ => None,
    }
}

fn spot_perp_leg(row: &RawOpportunity, side: OrderSide) -> Option<LegMarket> {
    match (row.extra.spot_leg_mode?, side) {
        (SpotLegMode::BuySpot, OrderSide::Buy)
        | (SpotLegMode::SellInventory | SpotLegMode::BorrowAndSell, OrderSide::Sell) => {
            Some(LegMarket::Spot)
        }
        (SpotLegMode::BuySpot, OrderSide::Sell)
        | (SpotLegMode::SellInventory | SpotLegMode::BorrowAndSell, OrderSide::Buy) => {
            Some(LegMarket::Perp)
        }
    }
}

fn leg_venue(row: &RawOpportunity, side: OrderSide) -> &str {
    match side {
        OrderSide::Buy => &row.long_exchange,
        OrderSide::Sell => &row.short_exchange,
    }
}

fn leg_symbol(row: &RawOpportunity, side: OrderSide, market: LegMarket) -> &str {
    match (side, market) {
        (OrderSide::Buy, LegMarket::Spot) => row
            .extra
            .long_depth_symbol
            .as_deref()
            .unwrap_or(&row.symbol),
        (OrderSide::Sell, LegMarket::Spot) => row
            .extra
            .short_depth_symbol
            .as_deref()
            .unwrap_or(&row.symbol),
        (OrderSide::Buy, LegMarket::Perp) => &row.long_rate.symbol,
        (OrderSide::Sell, LegMarket::Perp) => &row.short_rate.symbol,
    }
}

fn leg_price(row: &RawOpportunity, side: OrderSide) -> Option<f64> {
    match side {
        OrderSide::Buy => row.extra.long_price,
        OrderSide::Sell => row.extra.short_price,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        FundingRateData, MarketDataCoverage, MarketDataHealth, MarketDataQuality,
        MarketDataSnapshotStatusRow, MarketDataSourceKind,
    };

    #[test]
    fn aggregate_feed_health_cannot_invent_exact_leg_evidence() {
        let mut rows = vec![row(ArbitrageType::CrossExchange, None, None)];
        let status = status(vec![
            status_row(
                "binance",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
            ),
            status_row(
                "kucoin",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::RateLimited,
            ),
        ]);

        attach_leg_market_evidence(&mut rows, Some(&status), &[], &[], &[]);

        assert!(rows[0].extra.long_leg_market_evidence.is_none());
        assert!(rows[0].extra.short_leg_market_evidence.is_none());
    }

    #[test]
    fn attaches_spot_perp_leg_evidence_by_operation() {
        let mut rows = vec![row(
            ArbitrageType::SpotFutures,
            Some("BTC/USDT".into()),
            Some("BTC".into()),
        )];
        rows[0].extra.spot_leg_mode = Some(SpotLegMode::BuySpot);
        let status = status(vec![
            status_row(
                "binance",
                MarketDataSnapshotOperation::SpotTicks,
                MarketDataQuality::Fresh,
            ),
            status_row(
                "binance",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::StaleAllowed,
            ),
        ]);

        let perp_rows = vec![row_evidence(
            "binance",
            "BTC",
            MarketDataSnapshotOperation::PerpTickers,
            MarketDataQuality::StaleAllowed,
            MarketDataSourceKind::WsPush,
        )];
        let spot_rows = vec![row_evidence(
            "binance",
            "BTC/USDT",
            MarketDataSnapshotOperation::SpotTicks,
            MarketDataQuality::Fresh,
            MarketDataSourceKind::WsPush,
        )];

        attach_leg_market_evidence(&mut rows, Some(&status), &[], &perp_rows, &spot_rows);

        let long = rows[0].extra.long_leg_market_evidence.as_ref();
        let short = rows[0].extra.short_leg_market_evidence.as_ref();
        assert_eq!(long.map(|item| item.symbol.as_str()), Some("BTC/USDT"));
        assert_eq!(
            long.map(|item| item.health.quality),
            Some(MarketDataQuality::Fresh)
        );
        assert_eq!(short.map(|item| item.symbol.as_str()), Some("BTC"));
        assert_eq!(
            short.map(|item| item.health.quality),
            Some(MarketDataQuality::StaleAllowed)
        );
    }

    #[test]
    fn attaches_cross_spot_perp_reverse_leg_evidence_by_operation() {
        let mut rows = vec![row(
            ArbitrageType::CrossSpotFutures,
            Some("BTC".into()),
            Some("BTC/USDT".into()),
        )];
        rows[0].extra.spot_leg_mode = Some(SpotLegMode::SellInventory);
        let status = status(vec![
            status_row(
                "binance",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
            ),
            status_row(
                "kucoin",
                MarketDataSnapshotOperation::SpotTicks,
                MarketDataQuality::Missing,
            ),
        ]);

        let perp_rows = vec![row_evidence(
            "binance",
            "BTC",
            MarketDataSnapshotOperation::PerpTickers,
            MarketDataQuality::Fresh,
            MarketDataSourceKind::WsPush,
        )];
        let spot_rows = vec![row_evidence(
            "kucoin",
            "BTC/USDT",
            MarketDataSnapshotOperation::SpotTicks,
            MarketDataQuality::Missing,
            MarketDataSourceKind::WsPush,
        )];

        attach_leg_market_evidence(&mut rows, Some(&status), &[], &perp_rows, &spot_rows);

        let long = rows[0].extra.long_leg_market_evidence.as_ref();
        let short = rows[0].extra.short_leg_market_evidence.as_ref();
        assert_eq!(long.map(|item| item.symbol.as_str()), Some("BTC"));
        assert_eq!(
            long.map(|item| item.health.quality),
            Some(MarketDataQuality::Fresh)
        );
        assert_eq!(short.map(|item| item.symbol.as_str()), Some("BTC/USDT"));
        assert_eq!(
            short.map(|item| item.health.quality),
            Some(MarketDataQuality::Missing)
        );
    }

    #[test]
    fn prefers_perp_row_evidence_over_feed_status() {
        let mut rows = vec![row(ArbitrageType::CrossExchange, None, None)];
        rows[0].symbol = "btc-usdt".into();
        let status = status(vec![status_row(
            "kucoin",
            MarketDataSnapshotOperation::PerpTickers,
            MarketDataQuality::RateLimited,
        )]);
        let perp_rows = vec![
            row_evidence(
                "binance",
                "BTCUSDTM",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::WsPush,
            ),
            row_evidence(
                "kucoin",
                "BTC-USDT",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::WsPush,
            ),
        ];

        attach_leg_market_evidence(&mut rows, Some(&status), &[], &perp_rows, &[]);

        let long = rows[0].extra.long_leg_market_evidence.as_ref();
        let short = rows[0].extra.short_leg_market_evidence.as_ref();
        assert_eq!(long.map(|item| item.symbol.as_str()), Some("BTCUSDTM"));
        assert_eq!(short.map(|item| item.symbol.as_str()), Some("BTC-USDT"));
        assert_eq!(
            short.map(|item| item.health.quality),
            Some(MarketDataQuality::Fresh)
        );
        assert_eq!(
            short.map(|item| item.health.source),
            Some(MarketDataSourceKind::WsPush)
        );
    }

    #[test]
    fn base_only_perp_leg_uses_the_venue_default_quote_without_crossing_usdc() {
        let mut rows = vec![row(ArbitrageType::CrossExchange, None, None)];
        let perp_rows = vec![
            row_evidence(
                "binance",
                "BTCUSDT",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::WsPush,
            ),
            row_evidence(
                "binance",
                "BTCUSDC",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::WsPush,
            ),
            row_evidence(
                "kucoin",
                "BTC-USDT",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::WsPush,
            ),
        ];

        attach_leg_market_evidence(&mut rows, None, &[], &perp_rows, &[]);

        assert_eq!(
            rows[0]
                .extra
                .long_leg_market_evidence
                .as_ref()
                .map(|evidence| evidence.symbol.as_str()),
            Some("BTCUSDT")
        );
    }

    #[test]
    fn exact_perp_quote_keeps_its_own_row_evidence() {
        let mut rows = vec![row(ArbitrageType::CrossExchange, None, None)];
        rows[0].long_rate.symbol = "BTCUSDC".into();
        let perp_rows = vec![
            row_evidence(
                "binance",
                "BTCUSDC",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::WsPush,
            ),
            row_evidence(
                "binance",
                "BTCUSDT",
                MarketDataSnapshotOperation::PerpTickers,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::WsPush,
            ),
        ];

        attach_leg_market_evidence(&mut rows, None, &[], &perp_rows, &[]);

        assert_eq!(
            rows[0]
                .extra
                .long_leg_market_evidence
                .as_ref()
                .map(|evidence| evidence.symbol.as_str()),
            Some("BTCUSDC")
        );
    }

    #[test]
    fn missing_perp_contract_cannot_borrow_aggregate_feed_health() {
        let mut rows = vec![row(ArbitrageType::CrossExchange, None, None)];
        rows[0].long_rate.symbol = "ETHUSDT".into();
        let status = status(vec![status_row(
            "binance",
            MarketDataSnapshotOperation::PerpTickers,
            MarketDataQuality::Fresh,
        )]);
        let perp_rows = vec![row_evidence(
            "binance",
            "BTCUSDT",
            MarketDataSnapshotOperation::PerpTickers,
            MarketDataQuality::Fresh,
            MarketDataSourceKind::WsPush,
        )];

        attach_leg_market_evidence(&mut rows, Some(&status), &[], &perp_rows, &[]);

        assert!(rows[0].extra.long_leg_market_evidence.is_none());
    }

    #[test]
    fn attaches_exact_funding_evidence_without_upgrading_rest_to_ws() {
        let mut rows = vec![row(ArbitrageType::CrossExchange, None, None)];
        let funding_rows = vec![
            row_evidence(
                "binance",
                "BTCUSDT",
                MarketDataSnapshotOperation::FundingRates,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::WsPush,
            ),
            row_evidence(
                "kucoin",
                "BTC-USDT",
                MarketDataSnapshotOperation::FundingRates,
                MarketDataQuality::Fresh,
                MarketDataSourceKind::RestBaseline,
            ),
        ];

        attach_leg_market_evidence(&mut rows, None, &funding_rows, &[], &[]);

        assert_eq!(
            rows[0]
                .extra
                .long_funding_evidence
                .as_ref()
                .map(|evidence| evidence.source),
            Some(MarketDataSourceKind::WsPush)
        );
        assert_eq!(
            rows[0]
                .extra
                .short_funding_evidence
                .as_ref()
                .map(|evidence| evidence.source),
            Some(MarketDataSourceKind::RestBaseline)
        );
    }

    #[test]
    fn quote_conversion_requires_exact_row_evidence() {
        let mut rows = vec![row(ArbitrageType::CrossSpotFutures, None, None)];
        rows[0].extra.quote_conversions = vec![OpportunityQuoteConversion {
            from_quote: "USDC".into(),
            to_quote: "USDT".into(),
            rate: 0.999,
            venue: "kucoin".into(),
            symbol: "USDC-USDT".into(),
            market_evidence: None,
        }];
        let status = status(vec![status_row(
            "kucoin",
            MarketDataSnapshotOperation::SpotTicks,
            MarketDataQuality::Fresh,
        )]);

        attach_leg_market_evidence(&mut rows, Some(&status), &[], &[], &[]);
        assert!(rows[0].extra.quote_conversions[0].market_evidence.is_none());

        let spot_rows = vec![row_evidence(
            "kucoin",
            "USDC-USDT",
            MarketDataSnapshotOperation::SpotTicks,
            MarketDataQuality::Fresh,
            MarketDataSourceKind::WsPush,
        )];
        attach_leg_market_evidence(&mut rows, Some(&status), &[], &[], &spot_rows);

        let evidence = rows[0].extra.quote_conversions[0].market_evidence.as_ref();
        assert_eq!(
            evidence.map(|value| value.symbol.as_str()),
            Some("USDC-USDT")
        );
        assert_eq!(
            evidence.map(|value| value.health.source),
            Some(MarketDataSourceKind::WsPush)
        );
    }

    fn row(
        arb_type: ArbitrageType,
        long_depth_symbol: Option<String>,
        short_depth_symbol: Option<String>,
    ) -> RawOpportunity {
        RawOpportunity {
            symbol: "BTC".into(),
            arb_type,
            long_exchange: "binance".into(),
            short_exchange: short_exchange_for(arb_type).into(),
            long_rate: rate("binance"),
            short_rate: rate(short_exchange_for(arb_type)),
            spread_8h: 0.001,
            single_yield: 0.001,
            extra: crate::models::RawOpportunityExtra {
                long_price: Some(100.0),
                short_price: Some(101.0),
                long_depth_symbol,
                short_depth_symbol,
                ..Default::default()
            },
        }
    }

    fn short_exchange_for(arb_type: ArbitrageType) -> &'static str {
        match arb_type {
            ArbitrageType::SpotFutures => "binance",
            _ => "kucoin",
        }
    }

    fn rate(exchange: &str) -> FundingRateData {
        FundingRateData {
            symbol: "BTC".into(),
            exchange: exchange.into(),
            rate: 0.0,
            rate_8h: 0.0,
            predicted_rate: None,
            next_funding_time: 0,
            funding_interval: 8,
            volume_24h: 1_000_000.0,
            timestamp: 1,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        }
    }

    fn status(rows: Vec<MarketDataSnapshotStatusRow>) -> MarketDataSnapshotStatus {
        MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows,
        }
    }

    fn status_row(
        venue: &str,
        operation: MarketDataSnapshotOperation,
        quality: MarketDataQuality,
    ) -> MarketDataSnapshotStatusRow {
        MarketDataSnapshotStatusRow {
            venue: venue.into(),
            operation,
            health: MarketDataHealth {
                quality,
                source: MarketDataSourceKind::LocalCache,
                freshness_ms: Some(10),
                retry_after_ms: (quality == MarketDataQuality::RateLimited).then_some(2_000),
                last_error: None,
                observed_at_ms: 1,
                coverage: Some(MarketDataCoverage::new(
                    1,
                    (quality == MarketDataQuality::Fresh) as u64,
                )),
                problem: None,
            },
        }
    }

    fn row_evidence(
        venue: &str,
        symbol: &str,
        operation: MarketDataSnapshotOperation,
        quality: MarketDataQuality,
        source: MarketDataSourceKind,
    ) -> MarketDataRowEvidence {
        MarketDataRowEvidence {
            venue: venue.into(),
            symbol: symbol.into(),
            operation,
            health: MarketDataHealth {
                quality,
                source,
                freshness_ms: Some(10),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: 1,
                coverage: Some(MarketDataCoverage::new(1, 1)),
                problem: None,
            },
        }
    }
}
