use super::{include_discovery_perp, Binance};
use crate::adapter::ExchangeAdapter;
use crate::adapters::binance_market_data::{parse_mark_index, parse_open_interest};
use crate::adapters::binance_public_rest as public_rest;
use crate::error::ExchangeResult;
use futures::{stream, StreamExt};
use shared_types::MarkIndexInfo;
use std::collections::{HashMap, HashSet};

const OPEN_INTEREST_REST_CONCURRENCY: usize = 4;

impl Binance {
    pub(super) async fn rest_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let premiums = public_rest::premium_indexes(&self.http, &self.base_url).await?;
        let rows = premiums
            .into_iter()
            .filter(|item| include_discovery_perp(&item.symbol, requested.as_ref()))
            .filter_map(|item| parse_mark_index(&item))
            .collect();
        self.enrich_open_interest(rows, symbols).await
    }

    pub(super) async fn enrich_open_interest(
        &self,
        mut rows: Vec<MarkIndexInfo>,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        let Some(symbols) = symbols.filter(|rows| !rows.is_empty()) else {
            return Ok(rows);
        };
        let exchange_symbols = symbols
            .iter()
            .map(|symbol| self.to_exchange_symbol(symbol))
            .collect::<Vec<_>>();
        let interest = self.open_interest_map(&exchange_symbols).await?;
        for row in &mut rows {
            let exchange_symbol = self.to_exchange_symbol(&row.symbol);
            if let Some((value, timestamp)) = interest.get(&exchange_symbol) {
                row.open_interest = Some(*value);
                if row.timestamp == 0 {
                    row.timestamp = *timestamp;
                }
            }
        }
        Ok(rows)
    }

    async fn open_interest_map(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<HashMap<String, (f64, i64)>> {
        let requests = symbols
            .iter()
            .map(|symbol| public_rest::open_interest(&self.http, &self.base_url, symbol))
            .collect::<Vec<_>>();
        let results = stream::iter(requests)
            .buffer_unordered(OPEN_INTEREST_REST_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        let mut out = HashMap::with_capacity(results.len());
        for result in results {
            if let Some((symbol, value, timestamp)) = parse_open_interest(&result?) {
                out.insert(symbol, (value, timestamp));
            }
        }
        Ok(out)
    }
}
