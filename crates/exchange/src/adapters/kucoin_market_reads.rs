use super::Kucoin;
use crate::adapter::ExchangeAdapter;
use crate::adapters::kucoin_market_data::parse_mark_index;
use crate::adapters::kucoin_public_rest as public_rest;
use crate::error::ExchangeResult;
use shared_types::MarkIndexInfo;
use std::collections::HashSet;

impl Kucoin {
    pub(super) async fn rest_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        let requested = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<HashSet<_>>()
        });
        let items = public_rest::contracts(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(|row| row.symbol.ends_with("USDTM"))
            .filter(|row| match requested.as_ref() {
                Some(ids) => ids.contains(&row.symbol),
                None => true,
            })
            .filter_map(|row| parse_mark_index(&row))
            .collect())
    }
}
