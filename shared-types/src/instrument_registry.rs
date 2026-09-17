//! Venue instrument registry DTOs.
//!
//! `VenueInstrument` 是“每家交易所每个标的”的注册表事实源条目，负责 symbol
//! 身份解析（native/canonical/display）、资产类别分类与下单关键规格的归集。
//!
//! 核心 fail-closed 契约（对应 PR-AL）：一个标的只有在带有官方 endpoint 核验
//! 证据、处于 `Trading` 状态、canonical symbol 已解析、且下单必需规格齐备时，
//! 才允许据此**构建对冲**（`is_hedge_constructible`）；否则一律降级为
//! `observation-only`——绝不因为某 symbol 出现在 ticker 里就允许建腿。

use crate::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
use serde::{Deserialize, Serialize};

/// 标的资产类别——用于把加密与股票/指数/金属/能源/外汇区分开。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentAssetClass {
    Crypto,
    Equity,
    Index,
    Metal,
    Energy,
    Forex,
    Unknown,
}

/// 单家交易所单个标的的注册表条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueInstrument {
    pub venue: String,
    /// 交易所原生 ticker（用于真正发送请求）。
    pub native_symbol: String,
    /// 跨交易所归一化标识（用于撮合两腿）。
    pub canonical_symbol: String,
    /// 前端展示用标签。
    pub display_symbol: String,
    pub asset_class: InstrumentAssetClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_size: Option<f64>,
    /// Explicit execution boundary. Official metadata can remain visible for
    /// inverse or venue-special products whose sizing/write path is not yet
    /// representable without making them hedge-constructible.
    #[serde(default = "default_execution_supported")]
    pub execution_supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_tick: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty_step: Option<f64>,
    /// 最小下单数量（以合约张数计）。部分交易所（如 OKX）官方只给最小张数
    /// `minSz` 而不给最小名义；此字段承载该下限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_qty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_notional: Option<f64>,
    pub listing_status: InstrumentListingStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funding_interval_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builder_dex: Option<String>,
    pub source: InstrumentMetadataSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub checked_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
}

/// Authoritative shared instrument contract used by hedge compilation.
///
/// `VenueInstrument` remains the storage-oriented name for compatibility with
/// existing adapter ingestion. New execution code should use this alias so the
/// product contract reads as an instrument specification rather than a cache
/// implementation detail.
pub type InstrumentSpec = VenueInstrument;

/// Registry evidence older than this cannot authorize scanner or execution paths.
pub const INSTRUMENT_SPEC_FRESHNESS_MS: i64 = 12 * 60 * 60 * 1_000;

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

const fn default_execution_supported() -> bool {
    true
}

fn finite_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

impl VenueInstrument {
    /// 结构性校验：三类 symbol 与 venue 非空、`checked_at_ms` 为正、任何已给出
    /// 的精度字段必须有限且为正（缺失允许，缺失 ≠ 非法）。
    pub fn is_structurally_valid(&self) -> bool {
        if !(non_empty(&self.venue)
            && non_empty(&self.native_symbol)
            && non_empty(&self.canonical_symbol)
            && non_empty(&self.display_symbol))
        {
            return false;
        }
        if self.checked_at_ms <= 0 {
            return false;
        }
        [
            self.contract_size,
            self.price_tick,
            self.qty_step,
            self.min_qty,
            self.min_notional,
        ]
        .into_iter()
        .flatten()
        .all(finite_positive)
    }

    /// Official provenance must bind the row to a concrete endpoint schema.
    pub fn has_official_provenance(&self) -> bool {
        self.source == InstrumentMetadataSource::OfficialEndpoint
            && self.checked_at_ms > 0
            && self.source_url.as_deref().is_some_and(non_empty)
            && self.schema_version.as_deref().is_some_and(non_empty)
    }

    /// Future timestamps and expired evidence fail closed.
    pub fn is_fresh_at(&self, now_ms: i64, max_age_ms: i64) -> bool {
        max_age_ms > 0
            && now_ms >= self.checked_at_ms
            && now_ms.saturating_sub(self.checked_at_ms) < max_age_ms
    }

    /// fail-closed：是否可据此条目构建对冲腿。
    ///
    /// 仅当结构有效、来源为官方 endpoint、上市状态为 `Trading`、canonical
    /// symbol 已解析，且下单必需规格齐备时返回 true：`price_tick`/`qty_step`
    /// 必须给出，且最小下单下限 `min_notional`/`min_qty` **至少其一**给出
    /// （已在结构校验中保证有限且为正）。部分交易所只提供最小名义、部分只
    /// 提供最小张数，二者任一即可 fail-closed 阻断过小订单。
    pub fn is_hedge_constructible(&self) -> bool {
        self.is_structurally_valid()
            && self.execution_supported
            && self.has_official_provenance()
            && self.listing_status == InstrumentListingStatus::Trading
            && non_empty(&self.canonical_symbol)
            && self.price_tick.is_some()
            && self.qty_step.is_some()
            && (self.min_notional.is_some() || self.min_qty.is_some())
    }

    /// Runtime authorization requires the static contract and fresh evidence.
    pub fn is_hedge_constructible_at(&self, now_ms: i64, max_age_ms: i64) -> bool {
        self.is_hedge_constructible() && self.is_fresh_at(now_ms, max_age_ms)
    }

    /// 是否只能作为 observation-only（不可构建对冲）。
    pub fn is_observation_only(&self) -> bool {
        !self.is_hedge_constructible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constructible() -> VenueInstrument {
        VenueInstrument {
            venue: "binance".to_owned(),
            native_symbol: "BTCUSDT".to_owned(),
            canonical_symbol: "BTC/USDT".to_owned(),
            display_symbol: "BTC-USDT Perp".to_owned(),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("perp".to_owned()),
            quote_asset: Some("USDT".to_owned()),
            settle_asset: Some("USDT".to_owned()),
            margin_asset: Some("USDT".to_owned()),
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(0.1),
            qty_step: Some(0.001),
            min_qty: None,
            min_notional: Some(5.0),
            listing_status: InstrumentListingStatus::Trading,
            funding_interval_ms: Some(28_800_000),
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some("https://api.binance.com/exchangeInfo".to_owned()),
            checked_at_ms: 1_700_000_000_000,
            schema_version: Some("v1".to_owned()),
        }
    }

    #[test]
    fn official_trading_full_spec_is_hedge_constructible() {
        let env = constructible();
        assert!(env.is_hedge_constructible());
        assert!(!env.is_observation_only());
    }

    #[test]
    fn missing_endpoint_or_schema_provenance_is_observation_only() {
        let mut missing_url = constructible();
        missing_url.source_url = None;
        assert!(!missing_url.has_official_provenance());
        assert!(missing_url.is_observation_only());

        let mut missing_schema = constructible();
        missing_schema.schema_version = Some("  ".to_owned());
        assert!(!missing_schema.has_official_provenance());
        assert!(missing_schema.is_observation_only());
    }

    #[test]
    fn runtime_freshness_rejects_expired_or_future_evidence() {
        let env = constructible();
        assert!(env.is_hedge_constructible_at(
            env.checked_at_ms + INSTRUMENT_SPEC_FRESHNESS_MS - 1,
            INSTRUMENT_SPEC_FRESHNESS_MS,
        ));
        assert!(!env.is_hedge_constructible_at(
            env.checked_at_ms + INSTRUMENT_SPEC_FRESHNESS_MS,
            INSTRUMENT_SPEC_FRESHNESS_MS,
        ));
        assert!(
            !env.is_hedge_constructible_at(env.checked_at_ms - 1, INSTRUMENT_SPEC_FRESHNESS_MS,)
        );
    }

    #[test]
    fn cached_or_manual_source_is_observation_only() {
        for source in [
            InstrumentMetadataSource::CachedSnapshot,
            InstrumentMetadataSource::Manual,
            InstrumentMetadataSource::Unverified,
        ] {
            let mut env = constructible();
            env.source = source;
            assert!(env.is_observation_only());
            assert!(env.is_structurally_valid());
        }
    }

    #[test]
    fn non_trading_listing_is_observation_only() {
        for status in [
            InstrumentListingStatus::PreLaunch,
            InstrumentListingStatus::Suspended,
            InstrumentListingStatus::Delisted,
            InstrumentListingStatus::Unknown,
        ] {
            let mut env = constructible();
            env.listing_status = status;
            assert!(env.is_observation_only());
        }
    }

    #[test]
    fn explicit_execution_boundary_is_observation_only() {
        let mut env = constructible();
        env.execution_supported = false;
        assert!(env.is_structurally_valid());
        assert!(env.is_observation_only());
    }

    #[test]
    fn missing_required_spec_is_observation_only() {
        for clear in [0u8, 1, 2] {
            let mut env = constructible();
            match clear {
                0 => env.price_tick = None,
                1 => env.qty_step = None,
                // 同时清空两个最小下限 → 无任何 floor → observation-only。
                _ => {
                    env.min_notional = None;
                    env.min_qty = None;
                }
            }
            assert!(env.is_observation_only());
            assert!(env.is_structurally_valid());
        }
    }

    #[test]
    fn min_qty_only_is_hedge_constructible() {
        // 部分交易所（如 OKX）只给最小张数、不给最小名义；
        // 有 min_qty 即可构建对冲腿。
        let mut env = constructible();
        env.min_notional = None;
        env.min_qty = Some(1.0);
        assert!(env.is_hedge_constructible());
        assert!(!env.is_observation_only());
        assert!(env.is_structurally_valid());
    }

    #[test]
    fn bad_min_qty_is_structurally_invalid() {
        let mut env = constructible();
        env.min_qty = Some(0.0);
        assert!(!env.is_structurally_valid());
        let mut env = constructible();
        env.min_qty = Some(f64::NAN);
        assert!(!env.is_structurally_valid());
    }

    #[test]
    fn equity_observation_only_until_verified() {
        // 非加密标的（如股票）即便分类正确，未带官方核验也只能观察。
        let mut env = constructible();
        env.asset_class = InstrumentAssetClass::Equity;
        env.source = InstrumentMetadataSource::Unverified;
        assert!(env.is_observation_only());
    }

    #[test]
    fn empty_symbol_or_bad_precision_is_structurally_invalid() {
        let mut env = constructible();
        env.canonical_symbol = "  ".to_owned();
        assert!(!env.is_structurally_valid());
        assert!(env.is_observation_only());

        let mut env = constructible();
        env.price_tick = Some(f64::NAN);
        assert!(!env.is_structurally_valid());

        let mut env = constructible();
        env.min_notional = Some(0.0);
        assert!(!env.is_structurally_valid());

        let mut env = constructible();
        env.checked_at_ms = 0;
        assert!(!env.is_structurally_valid());
    }
}
