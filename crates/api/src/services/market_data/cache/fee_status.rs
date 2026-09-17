use super::*;
use arbitrage::algorithms::fee_evidence;
use shared_types::{FeeProduct, MarketDataSourceKind};

pub(super) fn push_fee_schedule_status_rows(
    rows: &mut Vec<MarketDataSnapshotStatusRow>,
    now_ms: i64,
) {
    for venue in fee_evidence::STANDARD_FEE_VENUE_FAMILIES {
        let mut received = 0u64;
        let mut latest_checked_at_ms: Option<i64> = None;
        let mut missing_products: Vec<&str> = Vec::new();
        for product in fee_evidence::STANDARD_FEE_PRODUCTS {
            match fee_evidence::standard_fee_schedule_evidence(venue, product) {
                Some(evidence) => {
                    received += 1;
                    if latest_is_before(latest_checked_at_ms, evidence.checked_at_ms) {
                        latest_checked_at_ms = Some(evidence.checked_at_ms);
                    }
                }
                None => missing_products.push(fee_product_label(product)),
            }
        }
        let quality = if received == fee_evidence::STANDARD_FEE_PRODUCTS.len() as u64 {
            SharedMarketDataQuality::Fresh
        } else {
            SharedMarketDataQuality::Missing
        };
        let last_error = (!missing_products.is_empty()).then(|| {
            format!(
                "official fee schedule evidence missing for products: {}",
                missing_products.join(", ")
            )
        });
        rows.push(MarketDataSnapshotStatusRow {
            venue: venue.to_owned(),
            operation: MarketDataSnapshotOperation::FeeSchedule,
            health: MarketDataHealth {
                quality,
                source: MarketDataSourceKind::LocalCache,
                freshness_ms: latest_checked_at_ms
                    .map(|checked_at| now_ms.saturating_sub(checked_at).max(0)),
                retry_after_ms: None,
                last_error,
                observed_at_ms: now_ms,
                coverage: Some(coverage(
                    fee_evidence::STANDARD_FEE_PRODUCTS.len() as u64,
                    received,
                )),
                problem: None,
            },
        });
    }
}

fn fee_product_label(product: FeeProduct) -> &'static str {
    match product {
        FeeProduct::Spot => "spot",
        FeeProduct::Perp => "perp",
        FeeProduct::Margin => "margin",
        FeeProduct::Unknown => "unknown",
    }
}
