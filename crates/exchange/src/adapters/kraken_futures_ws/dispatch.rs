use super::super::kraken_futures_book::FuturesBookState;
use super::super::kraken_futures_data::{
    parse_book_frame, parse_ticker_frame, FuturesBookFrame, FuturesTickerUpdate,
};
use super::{CachedTicker, KrakenFuturesPublicStream};
use common::time::now_ms;
use serde_json::Value;
use tracing::warn;

impl KrakenFuturesPublicStream {
    pub(super) fn on_text(&self, text: &str) {
        match parse_ticker_frame(text) {
            Ok(Some(update)) => {
                self.apply_ticker_frame(text, update);
                return;
            }
            Ok(None) => {}
            Err(error) => {
                warn!(error = %error, "kraken futures ticker parse failed");
                return;
            }
        }
        self.apply_book_frame(text);
    }

    fn apply_ticker_frame(&self, text: &str, update: FuturesTickerUpdate) {
        let Some(product_id) = serde_json::from_str::<Value>(text).ok().and_then(|row| {
            row.get("product_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }) else {
            return;
        };
        self.tickers.insert(
            product_id,
            CachedTicker {
                update,
                observed_at_ms: now_ms(),
            },
        );
    }

    fn apply_book_frame(&self, text: &str) {
        match parse_book_frame(text) {
            Ok(FuturesBookFrame::Snapshot(snapshot)) => {
                self.books.insert(
                    snapshot.product_id.clone(),
                    FuturesBookState::from_snapshot(snapshot),
                );
            }
            Ok(FuturesBookFrame::Delta(delta)) => self.apply_book_delta(&delta),
            Ok(FuturesBookFrame::Ignore) => {}
            Err(error) => warn!(error = %error, "kraken futures book parse failed"),
        }
    }
}
