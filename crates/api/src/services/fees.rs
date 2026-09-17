use common::AppError;
use dashmap::DashMap;
use shared_types::{normalized_venue_name, FeeProduct, TradeFeeSnapshot, TradeFeeSource};

use arbitrage::algorithms::fee_evidence::{
    matches_standard_fee_fixture, standard_fee_snapshot as standard_schedule_fee_snapshot,
    STANDARD_FEE_TTL_MS,
};

#[derive(Debug, Default)]
pub(crate) struct TradeFeeCache {
    rows: DashMap<TradeFeeKey, TradeFeeSnapshot>,
}

impl TradeFeeCache {
    pub(crate) fn upsert(&self, snapshot: TradeFeeSnapshot) {
        self.rows
            .insert(TradeFeeKey::from_snapshot(&snapshot), snapshot);
    }

    pub(crate) fn fresh(
        &self,
        venue: &str,
        symbol: &str,
        product: FeeProduct,
        account_id: Option<&str>,
        now_ms: i64,
    ) -> Option<TradeFeeSnapshot> {
        self.rows
            .get(&TradeFeeKey::new(venue, symbol, product, account_id))
            .filter(|row| row.is_fresh_verified(now_ms))
            .map(|row| row.clone())
    }
}

pub(crate) fn standard_fee_snapshot(
    venue: &str,
    symbol: &str,
    product: FeeProduct,
    use_maker_fee: bool,
    now_ms: i64,
) -> Option<TradeFeeSnapshot> {
    standard_schedule_fee_snapshot(venue, symbol, product, use_maker_fee, now_ms)
}

pub(crate) fn validate_external_fee_snapshot(
    snapshot: &TradeFeeSnapshot,
    now_ms: i64,
) -> Result<(), AppError> {
    if snapshot.source != TradeFeeSource::OfficialSchedule {
        return Err(fee_snapshot_rejected(
            "external fee snapshots must use official_schedule provenance",
        ));
    }
    if snapshot
        .account_id
        .as_ref()
        .is_some_and(|account_id| !account_id.trim().is_empty())
    {
        return Err(fee_snapshot_rejected(
            "official schedule snapshots must not be account scoped",
        ));
    }
    if snapshot.fetched_at_ms <= 0 || snapshot.fetched_at_ms > now_ms {
        return Err(fee_snapshot_rejected(
            "fee snapshot fetched_at_ms must be a past timestamp",
        ));
    }
    if snapshot.valid_until_ms <= now_ms {
        return Err(fee_snapshot_rejected("fee snapshot is expired"));
    }
    if snapshot.valid_until_ms > snapshot.fetched_at_ms.saturating_add(STANDARD_FEE_TTL_MS) {
        return Err(fee_snapshot_rejected(
            "fee snapshot valid_until_ms exceeds the standard ttl",
        ));
    }
    if !snapshot.is_fresh_verified(now_ms) {
        return Err(fee_snapshot_rejected("fee snapshot is not fresh verified"));
    }
    if !matches_standard_fee_fixture(snapshot) {
        return Err(fee_snapshot_rejected(
            "fee snapshot does not match an official schedule fixture",
        ));
    }
    Ok(())
}

fn fee_snapshot_rejected(reason: &'static str) -> AppError {
    AppError::BadRequest(format!("invalid fee snapshot provenance: {reason}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TradeFeeKey {
    venue: String,
    symbol: String,
    product: FeeProduct,
    account_id: String,
}

impl TradeFeeKey {
    fn new(venue: &str, symbol: &str, product: FeeProduct, account_id: Option<&str>) -> Self {
        Self {
            venue: normalized_venue_name(venue),
            symbol: symbol.to_ascii_uppercase(),
            product,
            account_id: account_id.unwrap_or_default().to_owned(),
        }
    }

    fn from_snapshot(snapshot: &TradeFeeSnapshot) -> Self {
        Self::new(
            &snapshot.venue,
            &snapshot.symbol,
            snapshot.product,
            snapshot.account_id.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn returns_only_fresh_verified_snapshot() {
        let cache = TradeFeeCache::default();
        cache.upsert(snapshot(
            "Binance",
            1_000,
            10_000,
            TradeFeeSource::AccountApi,
        ));

        assert!(cache
            .fresh("binance", "btc", FeeProduct::Perp, None, 2_000)
            .is_some());
        assert!(cache
            .fresh("binance", "btc", FeeProduct::Perp, None, 20_000)
            .is_none());
    }

    #[test]
    fn unverified_snapshot_is_not_fresh() {
        let cache = TradeFeeCache::default();
        cache.upsert(snapshot(
            "binance",
            1_000,
            10_000,
            TradeFeeSource::Unverified,
        ));

        assert!(cache
            .fresh("binance", "BTC", FeeProduct::Perp, None, 2_000)
            .is_none());
    }

    #[test]
    fn manual_snapshot_is_not_fresh() {
        let cache = TradeFeeCache::default();
        cache.upsert(snapshot("binance", 1_000, 10_000, TradeFeeSource::Manual));

        assert!(cache
            .fresh("binance", "BTC", FeeProduct::Perp, None, 2_000)
            .is_none());
    }

    #[test]
    fn standard_fee_snapshot_uses_execution_fee_side() -> Result<(), &'static str> {
        let maker = standard_fee_snapshot("hyperliquid:xyz", "MU", FeeProduct::Perp, true, 100)
            .ok_or("standard maker fee missing")?;
        let taker = standard_fee_snapshot("hyperliquid:xyz", "MU", FeeProduct::Perp, false, 100)
            .ok_or("standard taker fee missing")?;

        assert_eq!(maker.source, TradeFeeSource::OfficialSchedule);
        assert!(maker.open_fee_bps < taker.open_fee_bps);
        assert!(maker.is_fresh_verified(101));
        assert!(maker.evidence.is_some());
        Ok(())
    }

    #[test]
    fn standard_fee_snapshot_fails_closed_without_matching_evidence() {
        assert!(standard_fee_snapshot("okx", "BTC-USDT", FeeProduct::Margin, false, 100).is_none());
        assert!(
            standard_fee_snapshot("unknown", "BTC-USDT", FeeProduct::Perp, false, 100).is_none()
        );
    }

    #[test]
    fn standard_fee_snapshot_keeps_builder_venue_evidence_on_parent() -> Result<(), &'static str> {
        let snapshot = standard_fee_snapshot("hyperliquid:xyz", "MU", FeeProduct::Perp, false, 100)
            .ok_or("builder standard fee missing")?;

        assert_eq!(snapshot.venue, "hyperliquid:xyz");
        assert!(snapshot
            .evidence
            .as_ref()
            .is_some_and(|evidence| evidence.evidence_id.contains("hyperliquid")));
        Ok(())
    }

    #[test]
    fn external_upsert_rejects_account_api_provenance() {
        let snapshot = snapshot("binance", 1_000, 10_000, TradeFeeSource::AccountApi);

        assert!(validate_external_fee_snapshot(&snapshot, 2_000).is_err());
    }

    #[test]
    fn external_upsert_rejects_unmatched_official_schedule() -> Result<(), &'static str> {
        let mut snapshot =
            standard_fee_snapshot("binance", "BTCUSDT", FeeProduct::Perp, false, 1_000)
                .ok_or("standard fee missing")?;
        snapshot.maker_fee_bps += 0.1;

        assert!(validate_external_fee_snapshot(&snapshot, 2_000).is_err());
        Ok(())
    }

    #[test]
    fn external_upsert_allows_standard_schedule_fixture() -> Result<(), &'static str> {
        let snapshot = standard_fee_snapshot("binance", "BTCUSDT", FeeProduct::Perp, false, 1_000)
            .ok_or("standard fee missing")?;

        validate_external_fee_snapshot(&snapshot, 2_000).map_err(|_| "snapshot rejected")?;
        Ok(())
    }

    fn snapshot(
        venue: &str,
        fetched_at_ms: i64,
        valid_until_ms: i64,
        source: TradeFeeSource,
    ) -> TradeFeeSnapshot {
        TradeFeeSnapshot {
            venue: venue.into(),
            symbol: "BTC".into(),
            product: FeeProduct::Perp,
            account_id: None,
            maker_fee_bps: 1.0,
            taker_fee_bps: 4.0,
            open_fee_bps: 4.0,
            close_fee_bps: 4.0,
            source,
            fetched_at_ms,
            valid_until_ms,
            freshness_ms: Some(0),
            evidence: None,
            verification_problem: None,
            note: None,
        }
    }
}
