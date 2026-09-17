use super::*;

impl MarketDataCache {
    pub(crate) fn record_ws_runtime(&self, sample: WsRuntimeSample<'_>) {
        self.record_ws_runtime_at(sample, common::time::now_ms());
    }

    pub(super) fn record_ws_runtime_at(&self, sample: WsRuntimeSample<'_>, now_ms: i64) {
        let key = MarketRuntimeKey {
            venue: sample.venue.to_owned(),
            operation: sample.operation,
        };
        if sample.requested_symbols.is_empty() || sample.missing_symbols.is_empty() {
            self.ws_warmups.remove(&key);
            let requested = if sparse_event_stream_is_live(&sample) {
                sample.rows
            } else {
                sample.requested_symbols.len()
            };
            self.record_runtime_success(
                sample.venue,
                sample.operation,
                sample.source,
                requested,
                sample.rows,
            );
            return;
        }

        let oldest_missing_age_ms = self.update_ws_warmup(key, sample.missing_symbols, now_ms);
        let grace_ms = sample.grace_ms.max(1);
        let warming = oldest_missing_age_ms < grace_ms;
        let quality = if warming {
            MarketQuality::Warming
        } else {
            MarketQuality::Missing
        };
        let message = ws_partial_message(&sample, oldest_missing_age_ms, grace_ms, warming);
        self.upsert_runtime_health(MarketRuntimeHealth {
            venue: sample.venue.to_owned(),
            operation: sample.operation,
            quality,
            source: sample.source,
            requested: sample.requested_symbols.len() as u64,
            rows: sample.rows as u64,
            retry_after_ms: warming.then(|| (grace_ms - oldest_missing_age_ms) as u64),
            last_error: Some(message.clone()),
            problem: (!warming).then(|| {
                ExchangeProblem::new(sample.venue, sample.operation, message)
                    .with_source("public_ws_subscription")
            }),
            observed_at_ms: now_ms,
        });
    }

    fn update_ws_warmup(
        &self,
        key: MarketRuntimeKey,
        missing_symbols: &[String],
        now_ms: i64,
    ) -> i64 {
        let missing = missing_symbols
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let mut state = self.ws_warmups.entry(key).or_default();
        state
            .missing_since_ms
            .retain(|symbol, _| missing.contains(symbol.as_str()));
        for symbol in missing_symbols {
            state
                .missing_since_ms
                .entry(symbol.clone())
                .or_insert(now_ms);
        }
        state.last_seen_ms = now_ms;
        state
            .missing_since_ms
            .values()
            .map(|started_at_ms| now_ms.saturating_sub(*started_at_ms))
            .max()
            .unwrap_or_default()
    }

    pub(super) fn clear_ws_warmup(&self, venue: &str, operation: &'static str) {
        self.ws_warmups.remove(&MarketRuntimeKey {
            venue: venue.to_owned(),
            operation,
        });
    }
}

fn sparse_event_stream_is_live(sample: &WsRuntimeSample<'_>) -> bool {
    sample.venue == "kucoin" && sample.operation == "ws_spot_snapshot" && sample.rows > 0
}

fn ws_partial_message(
    sample: &WsRuntimeSample<'_>,
    age_ms: i64,
    grace_ms: i64,
    warming: bool,
) -> String {
    let phase = if warming {
        "awaiting first websocket event"
    } else {
        "websocket first-event deadline exceeded"
    };
    let symbols = sample
        .missing_symbols
        .iter()
        .take(3)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{} {phase}: rows {}/{}; missing {symbols}; age {age_ms}ms/{grace_ms}ms",
        sample.operation,
        sample.rows,
        sample.requested_symbols.len()
    )
}
