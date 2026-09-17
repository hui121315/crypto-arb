use super::{
    AssetCtx, ExchangeError, ExchangeResult, Hyperliquid, L2Book, OrderBookInfo, SpotMetaWrapper,
    SpotTick,
};
use crate::adapter::PublicWsSnapshot;
use crate::adapters::hyperliquid_market_data::{
    parse_levels, spot_context_for_entry, spot_contexts_by_coin, spot_pair, spot_token_names,
};
use moka::future::Cache;
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub(super) type SpotContext = Arc<(SpotMetaWrapper, Vec<AssetCtx>)>;
type SpotMetadata = Arc<SpotMetaWrapper>;

static SPOT_CONTEXTS: OnceLock<Cache<String, SpotContext>> = OnceLock::new();
static SPOT_METADATA: OnceLock<Cache<String, SpotMetadata>> = OnceLock::new();

const SPOT_CONTEXT_TTL: Duration = Duration::from_secs(15);
const SPOT_METADATA_TTL: Duration = Duration::from_secs(5 * 60);

struct SpotRoute<'a> {
    coin: &'a str,
    base: String,
    quote: String,
}

impl Hyperliquid {
    pub(super) async fn spot_ticks(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<SpotTick>> {
        if self.config.market.dex().is_some() {
            return Ok(Vec::new());
        }
        let context = self.spot_context().await?;
        let (meta, ctxs) = context.as_ref();
        Ok(spot_ticks_from_context(meta, ctxs, symbols))
    }

    pub(super) async fn ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        self.ensure_core_spot()?;
        let metadata = self.spot_metadata().await?;
        let routes = spot_routes(&metadata, Some(symbols)).collect::<Vec<_>>();
        let coins = routes
            .iter()
            .map(|route| route.coin.to_owned())
            .collect::<Vec<_>>();
        let Some(mids) = self.ws_all_mids_snapshot(&coins) else {
            return Ok(PublicWsSnapshot::Pending);
        };
        let rows = spot_ticks_from_mids(self.adapter_name(), &routes, &mids);
        Ok((!rows.is_empty())
            .then_some(rows)
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    pub(super) async fn spot_orderbook(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<OrderBookInfo> {
        self.ensure_core_spot()?;
        let metadata = self.spot_metadata().await?;
        let requested = [symbol.to_owned()];
        let route = spot_routes(&metadata, Some(&requested))
            .next()
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(symbol.to_owned()))?;
        if let Some(book) = self.ws_orderbook(route.coin, depth) {
            return Ok(relabel_book(book, &route));
        }

        let raw: L2Book = self
            .post_info(json!({"type": "l2Book", "coin": route.coin}))
            .await?;
        let mut book = OrderBookInfo {
            symbol: pair_symbol(&route),
            exchange: self.adapter_name().to_owned(),
            bids: parse_levels(&raw.levels[0]),
            asks: parse_levels(&raw.levels[1]),
            timestamp: raw.time,
        };
        truncate_book(&mut book, depth);
        Ok(book)
    }

    pub(super) async fn ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        self.ensure_core_spot()?;
        let metadata = self.spot_metadata().await?;
        let requested = [symbol.to_owned()];
        let route = spot_routes(&metadata, Some(&requested))
            .next()
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(symbol.to_owned()))?;
        Ok(self
            .ws_orderbook(route.coin, depth)
            .map(|book| PublicWsSnapshot::Ready(vec![relabel_book(book, &route)]))
            .unwrap_or(PublicWsSnapshot::Pending))
    }

    pub(super) async fn spot_context(&self) -> ExchangeResult<SpotContext> {
        let key = self.base_url.clone();
        let fetch = self.post_info(json!({"type": "spotMetaAndAssetCtxs"}));
        spot_contexts()
            .try_get_with(key, async { fetch.await.map(Arc::new) })
            .await
            .map_err(|error| ExchangeError::Api {
                exchange: self.adapter_name().to_owned(),
                code: "spot_context_fetch_failed".to_owned(),
                message: error.to_string(),
            })
    }

    async fn spot_metadata(&self) -> ExchangeResult<SpotMetadata> {
        let key = self.base_url.clone();
        let fetch = self.post_info(json!({"type": "spotMeta"}));
        spot_metadata()
            .try_get_with(key, async { fetch.await.map(Arc::new) })
            .await
            .map_err(|error| ExchangeError::Api {
                exchange: self.adapter_name().to_owned(),
                code: "spot_metadata_fetch_failed".to_owned(),
                message: error.to_string(),
            })
    }

    pub(super) async fn spot_instruments(
        &self,
        checked_at_ms: i64,
    ) -> ExchangeResult<Vec<shared_types::instrument_registry::VenueInstrument>> {
        self.ensure_core_spot()?;
        let metadata = self.spot_metadata().await?;
        Ok(
            crate::adapters::hyperliquid_instruments::spot_instruments_from_metadata(
                metadata.as_ref(),
                checked_at_ms,
            ),
        )
    }

    fn ensure_core_spot(&self) -> ExchangeResult<()> {
        if self.config.market.dex().is_some() {
            return Err(ExchangeError::UnsupportedCapability(
                "spot market data is provided by the Hyperliquid core venue",
            ));
        }
        Ok(())
    }
}

fn spot_routes<'a>(
    meta: &'a SpotMetaWrapper,
    symbols: Option<&'a [String]>,
) -> impl Iterator<Item = SpotRoute<'a>> + 'a {
    let token_names = spot_token_names(&meta.tokens);
    meta.universe.iter().filter_map(move |entry| {
        let (base, quote) = spot_pair(entry, &token_names)?;
        crate::spot::symbol_matches(&entry.name, &base, &quote, symbols).then_some(SpotRoute {
            coin: &entry.name,
            base,
            quote,
        })
    })
}

fn spot_ticks_from_context(
    meta: &SpotMetaWrapper,
    contexts: &[AssetCtx],
    symbols: Option<&[String]>,
) -> Vec<SpotTick> {
    let token_names = spot_token_names(&meta.tokens);
    let contexts_by_coin = spot_contexts_by_coin(contexts);
    meta.universe
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let context =
                spot_context_for_entry(entry, index, contexts, contexts_by_coin.as_ref())?;
            let (base, quote) = spot_pair(entry, &token_names)?;
            crate::spot::symbol_matches(&entry.name, &base, &quote, symbols)
                .then(|| {
                    crate::adapters::hyperliquid_market_data::parse_spot_tick(
                        &base, &quote, context,
                    )
                })
                .flatten()
        })
        .collect()
}

fn spot_ticks_from_mids(
    venue: &str,
    routes: &[SpotRoute<'_>],
    mids: &[(String, String)],
) -> Vec<SpotTick> {
    let mids = mids
        .iter()
        .map(|(coin, mid)| (coin.as_str(), mid.as_str()))
        .collect::<HashMap<_, _>>();
    let received_at_ms = common::time::now_ms();
    routes
        .iter()
        .filter_map(|route| {
            let mid = mids.get(route.coin)?.parse::<Decimal>().ok()?;
            (mid > Decimal::ZERO).then(|| SpotTick {
                venue: venue.to_owned(),
                symbol: pair_symbol(route),
                bid: mid,
                ask: mid,
                last: mid,
                bid_size: None,
                ask_size: None,
                volume_24h: Decimal::ZERO,
                exchange_ts_ms: None,
                received_at_ms,
            })
        })
        .collect()
}

fn relabel_book(mut book: OrderBookInfo, route: &SpotRoute<'_>) -> OrderBookInfo {
    book.symbol = pair_symbol(route);
    book
}

fn pair_symbol(route: &SpotRoute<'_>) -> String {
    format!("{}/{}", route.base, route.quote)
}

fn truncate_book(book: &mut OrderBookInfo, depth: u32) {
    if depth == 0 {
        return;
    }
    let cap = depth as usize;
    book.bids.truncate(cap);
    book.asks.truncate(cap);
}

fn spot_contexts() -> &'static Cache<String, SpotContext> {
    SPOT_CONTEXTS.get_or_init(|| {
        Cache::builder()
            .max_capacity(8)
            .time_to_live(SPOT_CONTEXT_TTL)
            .build()
    })
}

fn spot_metadata() -> &'static Cache<String, SpotMetadata> {
    SPOT_METADATA.get_or_init(|| {
        Cache::builder()
            .max_capacity(8)
            .time_to_live(SPOT_METADATA_TTL)
            .build()
    })
}

#[cfg(test)]
#[path = "hyperliquid_spot_ws_tests.rs"]
mod tests;
