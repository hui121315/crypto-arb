use super::*;

impl TradingService {
    pub(super) fn kucoin_funding_symbols(
        &self,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> Result<Vec<String>, exchange::ExchangeError> {
        let positions = self.position_cache.fresh_all(
            &["kucoin".to_owned()],
            self.account_cache_epoch(),
            common::time::now_ms(),
        );
        kucoin_funding_symbols_from_context(
            self.journal.list(),
            positions,
            start_time_ms,
            end_time_ms,
        )
    }
}

fn kucoin_funding_symbols_from_context(
    orders: Vec<OrderRecord>,
    positions: Option<Vec<PositionInfo>>,
    start_time_ms: Option<i64>,
    end_time_ms: Option<i64>,
) -> Result<Vec<String>, exchange::ExchangeError> {
    let start = start_time_ms.unwrap_or(i64::MIN);
    let end = end_time_ms.unwrap_or(i64::MAX);
    let position_cache_missing = positions.is_none();
    let mut symbols = orders
        .into_iter()
        .filter(|order| {
            let recent_activity = order.updated_at_ms >= start && order.updated_at_ms <= end;
            let startup_anchor = position_cache_missing
                && !order.intent.reduce_only
                && order.intent.created_at_ms <= end
                && order.updated_at_ms <= end;
            order.intent.mode == ExecutionMode::Live
                && is_kucoin_venue(&order.intent.exchange)
                && matches!(
                    order.state,
                    LiveOrderState::PartiallyFilled | LiveOrderState::Filled
                )
                && (recent_activity || startup_anchor)
        })
        .filter_map(|order| normalized_symbol(&order.intent.symbol))
        .collect::<BTreeSet<_>>();
    if let Some(positions) = positions {
        symbols.extend(
            positions
                .into_iter()
                .filter(|position| is_kucoin_venue(&position.exchange))
                .filter_map(|position| normalized_symbol(&position.symbol)),
        );
    }
    if symbols.len() > live_adapters::KUCOIN_FUNDING_SYMBOL_LIMIT {
        return Err(exchange::ExchangeError::Parse(format!(
            "kucoin funding payment candidate count {} exceeds bounded fanout limit {}",
            symbols.len(),
            live_adapters::KUCOIN_FUNDING_SYMBOL_LIMIT
        )));
    }
    Ok(symbols.into_iter().collect())
}

fn normalized_symbol(symbol: &str) -> Option<String> {
    let symbol = symbol.trim().to_ascii_uppercase();
    (!symbol.is_empty()).then_some(symbol)
}

fn is_kucoin_venue(venue: &str) -> bool {
    let normalized = normalized_venue_name(venue);
    normalized == "kucoin"
        || normalized
            .split_once(':')
            .is_some_and(|(family, _)| family == "kucoin")
}
