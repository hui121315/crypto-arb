use super::super::gate_crossex_data::{
    parse_book_delta, parse_book_snapshot, parse_funding_frame, parse_reference_frame,
    parse_ticker_frame,
};
use super::{GateCrossExPublicStream, Timed};
use common::time::now_ms;
use serde_json::Value;
use tracing::warn;

impl GateCrossExPublicStream {
    pub(super) fn on_text(&self, text: &str) {
        let Ok(root) = serde_json::from_str::<Value>(text) else {
            return;
        };
        if let Some(error) = root.get("error").filter(|value| !value.is_null()) {
            warn!(error = %error, "Gate CrossEx public WS request rejected");
            return;
        }
        let Some(channel) = root.get("channel").and_then(Value::as_str) else {
            return;
        };
        match channel {
            "ticker" => self.apply_ticker_frame(text),
            "funding_rate" => self.apply_funding_frame(text),
            "mark_price" | "index_price" | "open_interest" => {
                self.apply_reference_frame(text);
            }
            "order_book_update" => self.apply_book_delta_frame(text),
            value if value.starts_with("order_book_") => self.apply_book_snapshot_frame(text),
            _ => {}
        }
    }

    fn apply_ticker_frame(&self, text: &str) {
        match parse_ticker_frame(text) {
            Ok(Some(row)) => {
                self.tickers.insert(
                    row.route.native_symbol.clone(),
                    Timed {
                        value: row,
                        observed_at_ms: now_ms(),
                    },
                );
            }
            Ok(None) => {}
            Err(error) => warn!(error = %error, "Gate CrossEx ticker parse failed"),
        }
    }

    fn apply_funding_frame(&self, text: &str) {
        match parse_funding_frame(text) {
            Ok(Some(row)) => {
                self.funding.insert(
                    row.route.native_symbol.clone(),
                    Timed {
                        value: row,
                        observed_at_ms: now_ms(),
                    },
                );
            }
            Ok(None) => {}
            Err(error) => warn!(error = %error, "Gate CrossEx funding parse failed"),
        }
    }

    fn apply_reference_frame(&self, text: &str) {
        match parse_reference_frame(text) {
            Ok(Some(row)) => self.apply_reference(row),
            Ok(None) => {}
            Err(error) => warn!(error = %error, "Gate CrossEx reference parse failed"),
        }
    }

    fn apply_book_snapshot_frame(&self, text: &str) {
        match parse_book_snapshot(text) {
            Ok(Some(row)) => {
                self.full_books.insert(
                    row.route.native_symbol.clone(),
                    Timed {
                        value: row,
                        observed_at_ms: now_ms(),
                    },
                );
            }
            Ok(None) => {}
            Err(error) => warn!(error = %error, "Gate CrossEx full book parse failed"),
        }
    }

    fn apply_book_delta_frame(&self, text: &str) {
        match parse_book_delta(text) {
            Ok(Some(row)) => self.apply_incremental_book(&row),
            Ok(None) => {}
            Err(error) => warn!(error = %error, "Gate CrossEx incremental book parse failed"),
        }
    }
}
