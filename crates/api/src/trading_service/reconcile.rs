use super::*;

const QUERYABLE_AMBIGUOUS_SUBMIT_GRACE_MS: i64 = 60_000;

impl TradingService {
    pub(crate) async fn reconcile_open_orders(
        &self,
    ) -> Result<Vec<trading::ReconcileDiff>, exchange::ExchangeError> {
        let local = self.journal.list();
        let remote = self.engine.adapter().get_open_orders(None).await?;
        Ok(trading::diff_orders(&local, &remote))
    }

    async fn reconcile_runtime_open_orders(
        &self,
    ) -> Result<Vec<trading::ReconcileDiff>, exchange::ExchangeError> {
        let local = self.journal.list();
        let remote = self.list_open_orders().await?;
        Ok(trading::diff_orders(&local, &remote))
    }

    pub(crate) async fn reconcile_and_refresh_missing_orders(
        &self,
    ) -> Result<ReconcileOutcome, exchange::ExchangeError> {
        let mut refreshed = Vec::new();
        let mut refresh_failures = Vec::new();
        let mut attempted = BTreeSet::new();
        for internal_id in self.journal.list().into_iter().filter_map(|record| {
            (record.state == LiveOrderState::Unknown
                || contract_order_requires_authoritative_refresh(&record))
            .then_some(record.intent.id)
        }) {
            attempted.insert(internal_id.clone());
            self.refresh_reconcile_order(internal_id, &mut refreshed, &mut refresh_failures)
                .await;
        }
        let diffs = self.reconcile_runtime_open_orders().await?;
        for internal_id in repairable_reconcile_internal_ids(&diffs) {
            if attempted.insert(internal_id.clone()) {
                self.refresh_reconcile_order(internal_id, &mut refreshed, &mut refresh_failures)
                    .await;
            }
        }
        Ok(ReconcileOutcome {
            diffs,
            refreshed,
            refresh_failures,
        })
    }

    async fn refresh_reconcile_order(
        &self,
        internal_order_id: String,
        refreshed: &mut Vec<OrderRecord>,
        failures: &mut Vec<ReconcileRefreshFailure>,
    ) {
        let venue = self
            .journal
            .get(&internal_order_id)
            .map(|record| normalized_venue_name(&record.intent.exchange))
            .unwrap_or_default();
        match self.refresh_order_state(&internal_order_id).await {
            Ok(Some(record)) => refreshed.push(record),
            Ok(None) => {
                if let Some(record) =
                    self.fail_confirmed_missing_queryable_submit(&internal_order_id)
                {
                    refreshed.push(record);
                }
            }
            Err(error) => record_reconcile_refresh_failure(
                failures,
                internal_order_id,
                venue,
                error.to_string(),
            ),
        }
    }

    fn fail_confirmed_missing_queryable_submit(
        &self,
        internal_order_id: &str,
    ) -> Option<OrderRecord> {
        if self.adapter_name() != LIVE_ROUTER_ADAPTER_ID {
            return None;
        }
        let record = self.journal.get(internal_order_id)?;
        let now_ms = common::time::now_ms();
        if !is_expired_missing_queryable_submit(&record, now_ms) {
            return None;
        }
        let venue = normalized_venue_name(&record.intent.exchange);
        let client_order_field = client_order_field(&venue);
        let message = format!(
            "{venue} order was not found by {client_order_field} after the {}ms ambiguity window; submission was not retried",
            QUERYABLE_AMBIGUOUS_SUBMIT_GRACE_MS
        );
        let updated = self.journal.update_state_from_source(
            internal_order_id,
            LiveOrderState::Failed,
            Some(message),
            now_ms,
            OrderUpdateSource::Reconcile,
        );
        if updated.is_some() {
            tracing::warn!(
                internal_order_id,
                client_order_id = %record.intent.client_order_id,
                %venue,
                "expired ambiguous submission confirmed missing by order query"
            );
        }
        updated
    }
}

fn contract_order_requires_authoritative_refresh(record: &OrderRecord) -> bool {
    let venue = normalized_venue_name(&record.intent.exchange);
    if record.intent.mode != ExecutionMode::Live || !matches!(venue.as_str(), "gate" | "kucoin") {
        return false;
    }
    let tolerance = record.intent.quantity.abs().max(f64::EPSILON) * 1e-9;
    let impossible_overfill = record.filled_quantity.is_some_and(|filled| {
        filled.is_finite() && filled > record.intent.quantity.abs() + tolerance
    });
    let has_fill = record.state == LiveOrderState::Filled
        || record
            .filled_quantity
            .is_some_and(|filled| filled.is_finite() && filled > 0.0);
    impossible_overfill || (has_fill && record.last_update_source != OrderUpdateSource::OrderQuery)
}

fn is_expired_missing_queryable_submit(record: &OrderRecord, now_ms: i64) -> bool {
    let venue = normalized_venue_name(&record.intent.exchange);
    matches!(
        record.state,
        LiveOrderState::Submitted | LiveOrderState::Unknown
    ) && matches!(venue.as_str(), "bitget" | "kucoin" | "okx")
        && record.exchange_order_id.is_none()
        && record.intent.created_at_ms > 0
        && now_ms.saturating_sub(record.intent.created_at_ms) >= QUERYABLE_AMBIGUOUS_SUBMIT_GRACE_MS
}

fn client_order_field(venue: &str) -> &'static str {
    if venue == "okx" {
        "clOrdId"
    } else {
        "clientOid"
    }
}

fn record_reconcile_refresh_failure(
    failures: &mut Vec<ReconcileRefreshFailure>,
    internal_order_id: String,
    venue: String,
    error: String,
) {
    tracing::warn!(
        %error,
        order_id = %internal_order_id,
        "reconcile order refresh failed"
    );
    failures.push(ReconcileRefreshFailure {
        internal_order_id,
        venue,
        error,
    });
}
