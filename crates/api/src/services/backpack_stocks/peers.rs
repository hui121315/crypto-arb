use super::*;
use crate::services::{instrument_registry::InstrumentRegistry, market_data::MarketDataCache};
use shared_types::{InstrumentAssetClass, VenueInstrument};

impl BackpackStocks {
    pub(crate) fn with_peer_feed(
        mut self,
        aggregator: Arc<exchange::Aggregator>,
        subscriptions: Arc<crate::services::market_subscriptions::MarketSubscriptions>,
    ) -> Self {
        self.peer_feed = Some((aggregator, subscriptions));
        self
    }

    pub(super) fn peer_feed_enabled(&self, selection: &StockPeerSelection) -> bool {
        use crate::services::market_subscriptions::MarketSubscriptionFeed;
        self.peer_feed.as_ref().is_none_or(|(_, s)| {
            s.enabled(
                &selection.venue,
                match selection.product {
                    StockPeerProduct::Spot => MarketSubscriptionFeed::Spot,
                    StockPeerProduct::Perpetual => MarketSubscriptionFeed::Perp,
                },
            )
        })
    }

    pub(super) async fn poll_peer_ws(&self) {
        let (Some((registry, market)), Some((aggregator, _))) =
            (&self.peer_sources, &self.peer_feed)
        else {
            return;
        };
        let selection = self
            .snapshot
            .read()
            .peer
            .as_ref()
            .map(|p| p.selection.clone());
        let Some(selection) =
            selection.filter(|s| s.product == StockPeerProduct::Spot && self.peer_feed_enabled(s))
        else {
            return;
        };
        let Some(adapter) = aggregator.get(&selection.venue) else {
            return;
        };
        let mut symbols = vec![selection.native_symbol.clone()];
        if let Some(spec) = exact_spec(registry, &selection) {
            if let Some(quote) = spec
                .quote_asset
                .as_deref()
                .filter(|q| matches!(*q, "USD" | "USDT"))
            {
                symbols.push(format!("USDC/{quote}"));
            }
        }
        // Snapshot only touches existing adapter subscriptions. No REST/depth/private fallback.
        if let Ok(Ok(exchange::PublicWsSnapshot::Ready(rows))) = tokio::time::timeout(
            Duration::from_millis(300),
            adapter.public_ws_spot_snapshot(&symbols),
        )
        .await
        {
            market.store_spot_ticks(&rows, crate::services::market_data::MarketSource::WsPush);
        }
    }
    pub(crate) fn with_peer_markets(
        mut self,
        registry: Arc<InstrumentRegistry>,
        market: Arc<MarketDataCache>,
    ) -> Self {
        self.peer_sources = Some((registry, market));
        self
    }

    pub(crate) fn peer_catalog(
        &self,
        request: StockPeerCatalogRequest,
    ) -> Result<StockPeerCatalog, String> {
        if request.venue.len() > 80 || request.search.len() > 100 {
            return Err("市场搜索条件过长".into());
        }
        let (registry, _) = self.peer_sources.as_ref().ok_or("共享市场注册表未接入")?;
        let all = registry.venue_instruments(&request.venue);
        let registry_count = all.len();
        let search = request.search.trim().to_ascii_uppercase();
        let mut rows = all
            .into_iter()
            .filter(|r| {
                request.product.matches(r.product_type.as_deref())
                    && (r.native_symbol.to_ascii_uppercase().contains(&search)
                        || r.canonical_symbol.to_ascii_uppercase().contains(&search))
            })
            .collect::<Vec<_>>();
        let matched = rows.len();
        rows.truncate(80);
        Ok(StockPeerCatalog {
            request,
            rows,
            matched,
            registry_count,
        })
    }

    pub(crate) fn watch_peer(
        &self,
        request: StockPeerWatchRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let mut snapshot = self.snapshot.write();
        let security = snapshot
            .security
            .as_ref()
            .filter(|s| s.asset == request.asset)
            .ok_or("股票选择已变化，请重选对比市场")?;
        let peer = if let Some(selection) = request.selection {
            let (registry, market) = self.peer_sources.as_ref().ok_or("共享行情未接入")?;
            if selection.native_symbol.len() > 100 || selection.venue.len() > 80 {
                return Err("市场标识过长".into());
            }
            let spec = exact_spec(registry, &selection)
                .ok_or("所选原生市场不在官方注册表中；请刷新目录")?;
            Some(read_peer(
                security,
                selection,
                Some(spec),
                market,
                common::time::now_ms(),
            ))
        } else {
            None
        };
        if snapshot.peer.as_ref().map(|p| &p.selection) != peer.as_ref().map(|p| &p.selection) {
            self.generation.fetch_add(1, Ordering::SeqCst);
            snapshot.peer_preflight=None;
            snapshot.peer_funding=None;
            snapshot.peer_order_checks.clear();
        }
        snapshot.peer = peer;
        snapshot.observed_at_ms =
            common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        drop(snapshot);
        self.refresh_peer(common::time::now_ms());
        self.publish(hub);
        Ok(self.snapshot())
    }

    pub(super) fn refresh_peer(&self, now: i64) {
        let Some((registry, market)) = self.peer_sources.as_ref() else {
            return;
        };
        let selected = {
            let snapshot = self.snapshot.read();
            snapshot
                .security
                .clone()
                .zip(snapshot.peer.as_ref().map(|p| p.selection.clone()))
        };
        let Some((security, selection)) = selected else {
            return;
        };
        let mut peer = read_peer(
            &security,
            selection.clone(),
            exact_spec(registry, &selection),
            market,
            now,
        );
        if !self.peer_feed_enabled(&selection) {
            peer.quote = None;
            peer.quote_conversion = None;
            peer.problem = Some("该场所行情订阅已关闭，保留市场选择但不继续取价".into());
        }
        let mut snapshot = self.snapshot.write();
        if snapshot.security.as_ref() == Some(&security)
            && snapshot
                .peer
                .as_ref()
                .is_some_and(|p| p.selection == selection)
        {
            snapshot.peer = Some(peer);
        }
    }
}

fn exact_spec(
    registry: &InstrumentRegistry,
    selection: &StockPeerSelection,
) -> Option<VenueInstrument> {
    registry.public_native_instrument(
        &selection.venue,
        &selection.native_symbol,
        match selection.product {
            StockPeerProduct::Spot => shared_types::FeeProduct::Spot,
            StockPeerProduct::Perpetual => shared_types::FeeProduct::Perp,
        },
    )
}

fn read_peer(
    security: &StockSecurity,
    selection: StockPeerSelection,
    spec: Option<VenueInstrument>,
    market: &MarketDataCache,
    now: i64,
) -> StockPeerComparison {
    let mut problem = spec
        .is_none()
        .then(|| "原生市场已不在注册表中，旧对比不可执行".into());
    let quote = if selection.product == StockPeerProduct::Spot {
        spot_quote(market, &selection.venue, &selection.native_symbol, now)
    } else {
        let read = market.ticker_read(&selection.venue, &selection.native_symbol, now);
        read.value.and_then(|t| {
            // The older perpetual cache can collapse different quotes. Reject an ambiguous row.
            if t.symbol != selection.native_symbol {
                problem = Some("永续缓存未保留此精确原生市场身份，不能借用同名合约价格".into());
                return None;
            }
            Some(StockPeerQuote {
                symbol: t.symbol,
                bid: t.bid.to_string(),
                ask: t.ask.to_string(),
                bid_quantity: None,
                ask_quantity: None,
                source: read.source.as_str().into(),
                source_at_ms: Some(t.timestamp),
                received_at_ms: now.saturating_sub(read.freshness_ms.unwrap_or(i64::MAX)),
            })
        })
    };
    if quote.is_none() && problem.is_none() {
        problem = Some("共享缓存尚无该市场报价；请确认场所订阅已开启".into());
    }
    let quote_conversion = spec
        .as_ref()
        .and_then(|s| s.quote_asset.as_deref())
        .filter(|q| matches!(*q, "USD" | "USDT"))
        .and_then(|q| spot_quote(market, &selection.venue, &format!("USDC/{q}"), now));
    let identity = identity(security, spec.as_ref());
    // WS v2 uses the xStock base-asset display quantity. Kraken's xStock FAQ
    // distinguishes that share quantity from withdrawal tokens (qty / multiplier).
    // Do not enable the order compiler or apply this basis to legacy/other venues.
    let share_unit_verified = identity.underlying_verified && spec.as_ref().is_some_and(|s| {
        s.venue == "kraken"
            && s.product_type.as_deref() == Some("spot")
            && s.asset_class == InstrumentAssetClass::Equity
            && s.source_url.as_deref()
                == Some(
                    "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument",
                )
            && s.schema_version.as_deref() == Some("kraken-spot-ws-v2-instrument-2026-08-06")
            && s.is_fresh_at(
                now,
                shared_types::instrument_registry::INSTRUMENT_SPEC_FRESHNESS_MS,
            )
    });
    StockPeerComparison {
        selection,
        instrument: spec,
        identity,
        share_unit_verified,
        quote,
        quote_conversion,
        problem,
    }
}

pub(super) fn spot_quote(
    market: &MarketDataCache,
    venue: &str,
    symbol: &str,
    now: i64,
) -> Option<StockPeerQuote> {
    let read = market.spot_tick_read(venue, symbol, now);
    let t = read.value?;
    Some(StockPeerQuote {
        symbol: t.symbol,
        bid: t.bid.normalize().to_string(),
        ask: t.ask.normalize().to_string(),
        bid_quantity: t.bid_size.map(|v| v.normalize().to_string()),
        ask_quantity: t.ask_size.map(|v| v.normalize().to_string()),
        source: read.source.as_str().into(),
        source_at_ms: t.exchange_ts_ms,
        received_at_ms: t.received_at_ms,
    })
}

fn identity(security: &StockSecurity, spec: Option<&VenueInstrument>) -> StockPeerIdentity {
    // Separate product ISIN and underlying ISIN; this is NOT a token transfer mapping.
    if let Ok(profile) = shared_types::stocks::identity::backpack_issuer(security) {
        if let Some(xstock) = profile.kraken.as_ref().filter(|x| {
            spec.is_some_and(|s| {
                s.venue == "kraken"
                    && s.product_type.as_deref() == Some("spot")
                    && s.canonical_symbol == x.base.to_ascii_uppercase()
                    && s.quote_asset.as_ref().is_some_and(|q| s.native_symbol == format!("{}/{q}", x.base))
                    && s.asset_class == InstrumentAssetClass::Equity
                    && s.has_official_provenance()
            })
        }) {
            return StockPeerIdentity {
                underlying_verified: true,
                underlying_isin: Some(xstock.underlying_isin.into()),
                product_isin: Some(xstock.product_isin.into()),
                issuer: Some("Backed Assets (JE) Limited".into()),
                sources: vec![xstock.source.into(), profile.redemption_source.into(),
                    "https://support.kraken.com/articles/xstocks-faq".into(),
                    "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/instrument".into(),
                    "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/ticker".into(),
                    "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/add_order".into()],
                reason: format!("同为 {} 经济标的，Kraken 为 Backed xStock，Backpack 为另一种证券权益；不能直接互相充值", profile.native_name),
            };
        }
    }
    StockPeerIdentity {
        underlying_verified: false,
        underlying_isin: None,
        product_isin: None,
        issuer: None,
        sources: vec![],
        reason: "可自选市场观察；尚无证券标识与发行方的官方对应证明，同名或相近价格不代表同一资产"
            .into(),
    }
}

#[cfg(test)]
mod tests;
