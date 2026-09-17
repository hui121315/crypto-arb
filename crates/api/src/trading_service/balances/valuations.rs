use super::*;
use dashmap::mapref::entry::Entry;

const ACCOUNT_EVIDENCE_VALID_MS: i64 = 60_000;
const ACCOUNT_EVIDENCE_REFRESH_AFTER_MS: i64 = 45_000;
const ACCOUNT_EVIDENCE_RETRY_MS: i64 = 15_000;
const ACCOUNT_EVIDENCE_FAILURE_RETRY_MS: i64 = 60_000;
const HYPERLIQUID: &str = "hyperliquid";
const HYPERLIQUID_SPOT: &str = "hyperliquid:spot";
const HYPERLIQUID_ACCOUNT_EVIDENCE_LOCK: &str = "internal:hyperliquid-account-evidence";

impl TradingService {
    pub(crate) fn schedule_account_evidence_refresh(
        self: &Arc<Self>,
        balances: &[VenueBalanceInfo],
    ) {
        let currencies = hyperliquid_nonzero_currencies(balances);
        if currencies.is_empty() {
            return;
        }
        let now_ms = common::time::now_ms();
        if self.hyperliquid_account_evidence_current(&currencies, now_ms)
            || !self.claim_account_evidence_refresh(HYPERLIQUID, now_ms)
        {
            return;
        }
        let service = Arc::clone(self);
        tokio::spawn(async move {
            service
                .refresh_hyperliquid_account_evidence_if_due(&currencies)
                .await;
        });
    }

    pub(crate) fn schedule_hyperliquid_account_evidence_refresh(self: &Arc<Self>) {
        let now_ms = common::time::now_ms();
        if !self.hyperliquid_account_reader_configured()
            || self.hyperliquid_account_summary_valid(now_ms)
            || !self.claim_account_evidence_refresh(HYPERLIQUID, now_ms)
        {
            return;
        }

        let service = Arc::clone(self);
        tokio::spawn(async move {
            let refresh_lock = service.balance_fetch_lock(HYPERLIQUID_ACCOUNT_EVIDENCE_LOCK);
            let _guard = refresh_lock.lock().await;
            if service.hyperliquid_account_summary_valid(common::time::now_ms()) {
                service.clear_account_evidence_refresh_backoff(HYPERLIQUID);
                return;
            }
            service
                .fetch_and_record_hyperliquid_account_evidence()
                .await;
        });
    }

    pub(crate) fn account_summaries(&self) -> Vec<VenueAccountSummary> {
        let mut rows = self
            .account_summaries
            .iter()
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.venue.cmp(&b.venue));
        rows
    }

    pub(in crate::trading_service) fn record_account_summaries(
        &self,
        summaries: Vec<VenueAccountSummary>,
    ) {
        for summary in summaries {
            self.account_summaries
                .insert(normalized_venue_name(&summary.venue), summary);
        }
    }

    pub(in crate::trading_service) fn record_asset_valuations(
        &self,
        rows: Vec<VenueAssetValuation>,
    ) {
        let mut grouped: HashMap<String, Vec<VenueAssetValuation>> = HashMap::new();
        for row in rows.into_iter().filter(valid_asset_valuation) {
            grouped
                .entry(normalized_venue_name(&row.venue))
                .or_default()
                .push(row);
        }
        for (venue, rows) in grouped {
            self.replace_asset_valuations(&venue, rows);
        }
    }

    pub(in crate::trading_service) fn replace_asset_valuations(
        &self,
        venue: &str,
        rows: Vec<VenueAssetValuation>,
    ) {
        let venue = normalized_venue_name(venue);
        self.asset_valuations
            .retain(|(current, _), _| current != &venue);
        for mut row in rows.into_iter().filter(valid_asset_valuation) {
            row.venue = venue.clone();
            let currency = row.currency.trim().to_ascii_uppercase();
            row.currency = currency.clone();
            self.asset_valuations.insert((venue.clone(), currency), row);
        }
    }

    pub(crate) fn asset_valuations_for_rows(
        &self,
        balances: &[VenueBalanceInfo],
    ) -> Vec<VenueAssetValuation> {
        let mut rows = balances
            .iter()
            .filter_map(|balance| {
                let key = (
                    normalized_venue_name(&balance.venue),
                    balance.currency.trim().to_ascii_uppercase(),
                );
                self.asset_valuations
                    .get(&key)
                    .map(|entry| entry.value().clone())
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.venue.cmp(&b.venue).then(a.currency.cmp(&b.currency)));
        rows
    }

    async fn refresh_hyperliquid_account_evidence_if_due(&self, currencies: &[String]) {
        let refresh_lock = self.balance_fetch_lock(HYPERLIQUID_ACCOUNT_EVIDENCE_LOCK);
        let _guard = refresh_lock.lock().await;
        if self.hyperliquid_account_evidence_current(currencies, common::time::now_ms()) {
            return;
        }
        self.fetch_and_record_hyperliquid_account_evidence().await;
    }

    async fn fetch_and_record_hyperliquid_account_evidence(&self) {
        let venues = vec![HYPERLIQUID.to_owned()];
        match self
            .fetch_adapter_account_evidence_for_venues(&venues)
            .await
        {
            Ok(read) => {
                if self.record_hyperliquid_account_evidence(read) {
                    self.clear_account_evidence_refresh_backoff(HYPERLIQUID);
                } else {
                    self.defer_account_evidence_refresh(HYPERLIQUID);
                }
            }
            Err(error) => {
                self.defer_account_evidence_refresh(HYPERLIQUID);
                tracing::warn!(
                    %error,
                    venue = HYPERLIQUID,
                    "account evidence refresh failed"
                );
            }
        }
    }

    fn record_hyperliquid_account_evidence(&self, read: VenueAccountRead) -> bool {
        self.record_account_summaries(read.summaries);
        self.record_asset_valuations(read.asset_valuations);
        let valid = self.hyperliquid_account_summary_valid(common::time::now_ms());
        if !valid {
            tracing::warn!(
                venue = HYPERLIQUID,
                "account evidence refresh returned no usable consolidated summary"
            );
        }
        valid
    }

    fn hyperliquid_account_reader_configured(&self) -> bool {
        self.account_reader_venues().into_iter().any(|venue| {
            let venue = normalized_venue_name(&venue);
            venue == HYPERLIQUID || venue.starts_with("hyperliquid:")
        })
    }

    fn hyperliquid_account_summary_valid(&self, now_ms: i64) -> bool {
        self.account_summaries
            .get(HYPERLIQUID_SPOT)
            .is_some_and(|summary| {
                usable_hyperliquid_summary(summary.value())
                    && evidence_is_valid(summary.observed_at_ms, now_ms)
            })
    }

    fn hyperliquid_account_evidence_current(&self, currencies: &[String], now_ms: i64) -> bool {
        let summary_current = self
            .account_summaries
            .get(HYPERLIQUID_SPOT)
            .is_some_and(|summary| {
                usable_hyperliquid_summary(summary.value())
                    && evidence_is_current(summary.observed_at_ms, now_ms)
            });
        summary_current
            && currencies.iter().all(|currency| {
                self.asset_valuations
                    .get(&(HYPERLIQUID_SPOT.to_owned(), currency.clone()))
                    .is_some_and(|row| evidence_is_current(row.observed_at_ms, now_ms))
            })
    }

    fn claim_account_evidence_refresh(&self, venue: &str, now_ms: i64) -> bool {
        match self
            .account_evidence_refresh_after_ms
            .entry(normalized_venue_name(venue))
        {
            Entry::Occupied(mut entry) => {
                if now_ms < *entry.get() {
                    return false;
                }
                entry.insert(now_ms.saturating_add(ACCOUNT_EVIDENCE_RETRY_MS));
                true
            }
            Entry::Vacant(entry) => {
                entry.insert(now_ms.saturating_add(ACCOUNT_EVIDENCE_RETRY_MS));
                true
            }
        }
    }

    fn defer_account_evidence_refresh(&self, venue: &str) {
        self.account_evidence_refresh_after_ms.insert(
            normalized_venue_name(venue),
            common::time::now_ms().saturating_add(ACCOUNT_EVIDENCE_FAILURE_RETRY_MS),
        );
    }

    fn clear_account_evidence_refresh_backoff(&self, venue: &str) {
        self.account_evidence_refresh_after_ms
            .remove(&normalized_venue_name(venue));
    }
}

fn hyperliquid_nonzero_currencies(balances: &[VenueBalanceInfo]) -> Vec<String> {
    let mut currencies = balances
        .iter()
        .filter(|row| normalized_venue_name(&row.venue) == HYPERLIQUID_SPOT)
        .filter(|row| row.total.is_finite() && row.total > 0.0)
        .map(|row| row.currency.trim().to_ascii_uppercase())
        .filter(|currency| !currency.is_empty())
        .collect::<Vec<_>>();
    currencies.sort_unstable();
    currencies.dedup();
    currencies
}

fn usable_hyperliquid_summary(summary: &VenueAccountSummary) -> bool {
    summary.problem.is_none()
        && matches!(
            summary.account_type.trim().to_ascii_lowercase().as_str(),
            "unifiedaccount" | "portfoliomargin"
        )
}

fn evidence_is_valid(observed_at_ms: i64, now_ms: i64) -> bool {
    observed_at_ms > 0 && now_ms.saturating_sub(observed_at_ms) <= ACCOUNT_EVIDENCE_VALID_MS
}

fn evidence_is_current(observed_at_ms: i64, now_ms: i64) -> bool {
    observed_at_ms > 0 && now_ms.saturating_sub(observed_at_ms) <= ACCOUNT_EVIDENCE_REFRESH_AFTER_MS
}

fn valid_asset_valuation(row: &VenueAssetValuation) -> bool {
    !row.venue.trim().is_empty()
        && !row.currency.trim().is_empty()
        && row.usd_value.is_finite()
        && row.observed_at_ms > 0
}

#[cfg(test)]
#[path = "valuations_tests.rs"]
mod tests;
