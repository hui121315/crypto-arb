//! per-venue 执行质量。

use crate::problem::ApiProblem;
use serde::{Deserialize, Serialize};

mod credentials;
mod health;
mod id;
mod operation;
mod operation_kind;
mod operation_kind_labels;
mod quality;
mod runtime_health;
mod runtime_health_snapshot;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_operation_kind;
#[cfg(test)]
mod tests_runtime_health;

pub use credentials::*;
pub use health::*;
pub use id::*;
pub use operation::*;
pub use operation_kind::*;
pub use quality::*;
pub use runtime_health::*;
pub use runtime_health_snapshot::*;

/// USD 本位结算币判定（含 builder DEX 的 USDT0 / USDH）。
///
/// `run_cost` 的 USD 口径审计与 `execution` 的报价优先级共同引用此定义——
/// 历史上两处各写一份且互相矛盾（`run_cost` 漏掉 `USDT0`/`USDH`，导致
/// `hyperliquid` builder DEX 腿的 fee/funding `amount_usd` 恒为 `NULL`，USD
/// 口径成本被系统性少算）。
pub fn is_usd_pegged_settlement_currency(currency: &str) -> bool {
    let currency = currency.trim();
    ["USD", "USDC", "USDT", "USDT0", "USDH"]
        .iter()
        .any(|candidate| currency.eq_ignore_ascii_case(candidate))
}

pub fn normalized_venue_name(venue: &str) -> String {
    venue.trim().to_ascii_lowercase()
}

pub fn venue_names_equal(left: &str, right: &str) -> bool {
    normalized_venue_name(left) == normalized_venue_name(right)
}

pub fn venue_family(venue: &str) -> &str {
    let venue = venue.trim();
    venue.split_once(':').map_or(venue, |(base, _)| base)
}

pub fn venue_family_id(venue: &str) -> Option<VenueId> {
    VenueId::from_exchange_name(venue_family(venue))
}

pub fn is_hyperliquid_builder_venue(venue: &str) -> bool {
    venue_family_id(venue) == Some(VenueId::Hyperliquid)
        && venue
            .trim()
            .split_once(':')
            .is_some_and(|(_, dex)| !dex.trim().is_empty())
}
