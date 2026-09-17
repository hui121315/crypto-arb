use super::super::kraken_spot_data::{
    parse_book_frame, parse_instrument_frame, parse_ticker_updates, SpotInstrumentFrame,
    SpotTickerUpdate,
};
use super::{
    mark_book_subscription_sent, mark_ticker_problem, method_ack, subscription_ack_accepted,
    subscription_already_exists, CachedTick, KrakenSpotPublicStream, MethodAck, PendingBookRequest,
    PendingTickerRequest,
};
use common::time::now_ms;
use serde_json::Value;
use shared_types::VenueInstrument;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, warn};

impl KrakenSpotPublicStream {
    pub(super) fn on_text(&self, text: &str) {
        let Ok(root) = serde_json::from_str::<Value>(text) else {
            return;
        };
        if self.apply_method_ack(&root) {
            return;
        }
        match root.get("channel").and_then(Value::as_str) {
            Some("instrument") => self.apply_instrument_frame(text),
            Some("ticker") => self.apply_ticker_frame(text),
            Some("book") => self.apply_book_frame(text),
            _ => {}
        }
    }

    fn apply_method_ack(&self, root: &Value) -> bool {
        let Some(ack) = method_ack(root) else {
            return false;
        };
        if let Some((_, request)) = self.ticker_requests.remove(&ack.req_id) {
            self.apply_ticker_subscription_ack(ack, &request);
            return true;
        }
        let Some((_, request)) = self.book_requests.remove(&ack.req_id) else {
            warn_unmatched_rejection(&ack);
            return true;
        };
        if request.subscribe {
            self.apply_book_subscription_ack(ack, &request);
        }
        true
    }

    fn apply_ticker_subscription_ack(&self, ack: MethodAck, request: &PendingTickerRequest) {
        if !request.subscribe {
            return;
        }
        if subscription_ack_accepted(&ack) || subscription_already_exists(&ack) {
            self.accept_ticker_subscription(&ack, request);
            return;
        }
        self.reject_ticker_subscription(ack, request);
    }

    fn accept_ticker_subscription(&self, ack: &MethodAck, request: &PendingTickerRequest) {
        debug!(
            req_id = ack.req_id,
            symbols = ?request.symbols,
            already_subscribed = subscription_already_exists(ack),
            "kraken spot ticker subscription acknowledged"
        );
        for symbol in &request.symbols {
            if self.ticks.contains_key(symbol) {
                self.ticker_problems.remove(symbol);
            } else {
                self.ticker_problems.insert(
                    symbol.clone(),
                    format!("Kraken {symbol} 行情订阅已确认，等待首个最优买卖价"),
                );
            }
        }
    }

    fn reject_ticker_subscription(&self, ack: MethodAck, request: &PendingTickerRequest) {
        let error = ack
            .error
            .unwrap_or_else(|| "交易所未返回具体原因".to_owned());
        mark_ticker_problem(
            &self.ticker_problems,
            &request.symbols,
            &format!("订阅被拒绝：{error}"),
        );
        warn!(
            req_id = ack.req_id,
            symbols = ?request.symbols,
            %error,
            "kraken spot ticker subscription rejected"
        );
    }

    fn apply_book_subscription_ack(&self, ack: MethodAck, request: &PendingBookRequest) {
        if subscription_ack_accepted(&ack) {
            self.mark_book_subscription_ready(request);
        } else if subscription_already_exists(&ack) {
            self.resolve_existing_book_subscription(request);
        } else {
            self.reject_book_subscription(ack, request);
        }
    }

    fn mark_book_subscription_ready(&self, request: &PendingBookRequest) {
        mark_book_subscription_sent(
            &self.book_subscriptions,
            &request.symbol,
            request.depth,
            true,
        );
        self.book_problems.remove(&request.symbol);
    }

    fn resolve_existing_book_subscription(&self, request: &PendingBookRequest) {
        if self.books.contains_key(&request.symbol) {
            self.mark_book_subscription_ready(request);
        } else {
            mark_book_subscription_sent(
                &self.book_subscriptions,
                &request.symbol,
                request.depth,
                false,
            );
            self.book_problems.insert(
                request.symbol.clone(),
                format!(
                    "Kraken {} 已订阅但本地缺少首个盘口，正在重新同步",
                    request.symbol
                ),
            );
            self.spawn_book_subscription("unsubscribe", &request.symbol, request.depth);
        }
    }

    fn reject_book_subscription(&self, ack: MethodAck, request: &PendingBookRequest) {
        let error = ack
            .error
            .unwrap_or_else(|| "交易所未返回具体原因".to_owned());
        self.books.remove(&request.symbol);
        mark_book_subscription_sent(
            &self.book_subscriptions,
            &request.symbol,
            request.depth,
            false,
        );
        self.book_problems.insert(
            request.symbol.clone(),
            format!("Kraken 拒绝 {} 盘口订阅：{error}", request.symbol),
        );
        warn!(
            req_id = ack.req_id,
            symbol = %request.symbol,
            %error,
            "kraken spot book subscription rejected"
        );
    }

    fn apply_instrument_frame(&self, text: &str) {
        match parse_instrument_frame(text) {
            Ok(frame) if !frame.rows.is_empty() || !frame.asset_classes.is_empty() => {
                let current = self.instruments.load_full();
                self.instruments
                    .store(Arc::new(merge_instrument_frame(&current, frame)));
            }
            Ok(_) => {}
            Err(error) => warn!(error = %error, "kraken spot instrument parse failed"),
        }
    }

    fn apply_ticker_frame(&self, text: &str) {
        match parse_ticker_updates(text) {
            Ok(rows) => {
                let observed_at_ms = now_ms();
                rows.into_iter()
                    .for_each(|row| self.apply_ticker_update(row, observed_at_ms));
            }
            Err(error) => warn!(error = %error, "kraken spot ticker parse failed"),
        }
    }

    fn apply_ticker_update(&self, row: SpotTickerUpdate, observed_at_ms: i64) {
        self.ticker_problems.remove(&row.native_symbol);
        let symbol = row.native_symbol.clone();
        let first_frame = self
            .ticks
            .insert(
                row.native_symbol,
                CachedTick {
                    tick: row.tick,
                    observed_at_ms,
                },
            )
            .is_none();
        if first_frame {
            debug!(%symbol, "kraken spot ticker first frame received");
        }
    }

    fn apply_book_frame(&self, text: &str) {
        match parse_book_frame(text) {
            Ok(rows) => rows.iter().for_each(|row| self.apply_book(row)),
            Err(error) => warn!(error = %error, "kraken spot book parse failed"),
        }
    }
}

fn merge_instrument_frame(
    current: &[VenueInstrument],
    frame: SpotInstrumentFrame,
) -> Vec<VenueInstrument> {
    if frame.snapshot {
        return frame.rows;
    }
    let asset_classes = current
        .iter()
        .filter(|row| row.asset_class != shared_types::InstrumentAssetClass::Unknown)
        .map(|row| (row.canonical_symbol.clone(), row.asset_class))
        .collect::<BTreeMap<_, _>>();
    let mut by_symbol = current
        .iter()
        .cloned()
        .map(|row| (row.native_symbol.clone(), row))
        .collect::<BTreeMap<_, _>>();
    for mut row in frame.rows {
        if !frame.asset_classes.contains_key(&row.canonical_symbol) {
            if let Some(previous) = by_symbol.get(&row.native_symbol) {
                row.asset_class = previous.asset_class;
                row.execution_supported = previous.execution_supported;
            } else if let Some(class) = asset_classes.get(&row.canonical_symbol) {
                // A new quote market inherits the known base asset, not a crypto default.
                row.asset_class = *class;
                row.execution_supported = *class == shared_types::InstrumentAssetClass::Crypto;
            }
        }
        by_symbol.insert(row.native_symbol.clone(), row);
    }
    for row in by_symbol.values_mut() {
        if let Some(class) = frame.asset_classes.get(&row.canonical_symbol) {
            row.asset_class = *class;
            row.execution_supported = *class == shared_types::InstrumentAssetClass::Crypto;
        }
    }
    by_symbol.into_values().collect()
}

fn warn_unmatched_rejection(ack: &MethodAck) {
    if ack.success || subscription_already_exists(ack) {
        return;
    }
    warn!(
        req_id = ack.req_id,
        error = ack.error.as_deref().unwrap_or("unknown error"),
        "kraken spot ws request rejected"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xstock_pair_update_keeps_asset_class_without_assets_array() {
        let snapshot = parse_instrument_frame(include_str!(
            "../../../fixtures/kraken/spot_v2_instrument_muxusd.json"
        ))
        .unwrap();
        let current = merge_instrument_frame(&[], snapshot);
        let update = parse_instrument_frame(&instrument_frame(
            "update",
            &[("MUx/USD", "MUx", "USD", 0.01)],
        ))
        .unwrap();
        let merged = merge_instrument_frame(&current, update);
        assert_eq!(
            merged[0].asset_class,
            shared_types::InstrumentAssetClass::Equity
        );
        assert!(!merged[0].execution_supported);
        let update = parse_instrument_frame(&instrument_frame(
            "update",
            &[
                ("MUx/USDC", "MUx", "USDC", 0.01),
                ("UNCLASSIFIED/USD", "UNCLASSIFIED", "USD", 0.01),
            ],
        ))
        .unwrap();
        let merged = merge_instrument_frame(&merged, update);
        let stock = merged
            .iter()
            .find(|r| r.native_symbol == "MUx/USDC")
            .unwrap();
        assert_eq!(
            stock.asset_class,
            shared_types::InstrumentAssetClass::Equity
        );
        assert!(!stock.execution_supported);
        let unknown = merged
            .iter()
            .find(|r| r.native_symbol == "UNCLASSIFIED/USD")
            .unwrap();
        assert_eq!(
            unknown.asset_class,
            shared_types::InstrumentAssetClass::Unknown
        );
        assert!(!unknown.execution_supported);
    }

    #[test]
    fn instrument_update_keeps_unmentioned_snapshot_pairs() {
        let snapshot = parse_instrument_frame(&instrument_frame(
            "snapshot",
            &[
                ("BTC/USD", "BTC", "USD", 0.1),
                ("PUPS/USD", "PUPS", "USD", 0.000_001),
            ],
        ))
        .expect("parse snapshot");
        let current = merge_instrument_frame(&[], snapshot);
        let update = parse_instrument_frame(&instrument_frame(
            "update",
            &[("BTC/USD", "BTC", "USD", 0.01)],
        ))
        .expect("parse update");

        let merged = merge_instrument_frame(&current, update);

        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|row| row.native_symbol == "PUPS/USD"));
        assert_eq!(
            merged
                .iter()
                .find(|row| row.native_symbol == "BTC/USD")
                .and_then(|row| row.price_tick),
            Some(0.01),
        );
    }

    fn instrument_frame(frame_type: &str, pairs: &[(&str, &str, &str, f64)]) -> String {
        let pairs = pairs
            .iter()
            .map(|(symbol, base, quote, price_increment)| {
                serde_json::json!({
                    "symbol": symbol,
                    "base": base,
                    "quote": quote,
                    "status": "online",
                    "price_increment": price_increment,
                    "qty_increment": 0.00001,
                    "qty_min": 1.0,
                    "cost_min": 0.5,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "channel": "instrument",
            "type": frame_type,
            "data": {"pairs": pairs},
        })
        .to_string()
    }
}
