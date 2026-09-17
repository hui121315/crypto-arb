//! KuCoin Futures contract metadata cache for order sizing.

use super::kucoin_market_data::{
    contract_identity, contract_spec, ContractActive, KucoinContractSpec,
};
use super::kucoin_public_rest;
use super::kucoin_trade_data::validation_error;
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use common::time::now_ms;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

const CONTRACT_MULTIPLIER_TTL_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Default)]
pub(super) struct KucoinContractMultipliers {
    specs_by_native: DashMap<String, KucoinContractSpec>,
    active_by_native: DashMap<String, String>,
    native_by_base_quote: DashMap<String, String>,
    native_by_normalized: DashMap<String, String>,
    fetched_at_ms: AtomicI64,
}

impl KucoinContractMultipliers {
    pub(super) fn cached_native_symbol(&self, symbol: &str) -> Option<String> {
        if !self.is_fresh() {
            return None;
        }
        let native = symbol.to_ascii_uppercase();
        if self.active_by_native.contains_key(&native) {
            return Some(native);
        }
        if let Some(quote) = requested_quote(&native) {
            let normalized = requested_base(&native, quote);
            return self
                .native_by_base_quote
                .get(&base_quote_key(&normalized, quote))
                .map(|native| native.clone());
        }
        let normalized = crate::adapter::strip_common_suffixes(symbol);
        self.native_by_normalized
            .get(&normalized)
            .map(|native| native.clone())
    }

    pub(super) async fn order_unit(
        &self,
        http: &HttpClient,
        base_url: &str,
        symbol: &str,
    ) -> ExchangeResult<f64> {
        if let Some(unit) = self.cached_unit(symbol) {
            return Ok(unit);
        }
        self.refresh_all(http, base_url).await?;
        let spec = self.cached_spec(symbol)?;
        Ok(spec.order_unit)
    }

    pub(super) async fn native_symbol(
        &self,
        http: &HttpClient,
        base_url: &str,
        symbol: &str,
    ) -> ExchangeResult<String> {
        if let Some(native) = self.cached_native_symbol(symbol) {
            return Ok(native);
        }
        self.refresh_all(http, base_url).await?;
        self.cached_native_symbol(symbol)
            .ok_or_else(|| unsupported_contract(symbol))
    }

    /// PR-DP-04 D-2: 全量 `/api/v1/contracts/active` 一次性 prewarm 所有 multiplier，
    /// 让 lifecycle 冷启动 prewarm 之后 hot path 不再每个新 symbol 单调 REST。
    /// 24h TTL 共用 `fetched_at_ms`：refresh 成功即整体续期，部分 row 跳过 (status / unit 非法)
    /// 不影响其他 row 的 fresh 判定。
    pub(super) async fn refresh_all(
        &self,
        http: &HttpClient,
        base_url: &str,
    ) -> ExchangeResult<()> {
        let items = kucoin_public_rest::contracts(http, base_url).await?;
        self.apply_contracts(items);
        Ok(())
    }

    fn apply_contracts(&self, items: Vec<ContractActive>) {
        let mut identities = Vec::new();
        let mut base_counts = HashMap::<String, usize>::new();
        let mut specs = Vec::new();
        for item in items {
            if let Some(identity) = contract_identity(&item) {
                *base_counts
                    .entry(identity.normalized_symbol.clone())
                    .or_default() += 1;
                identities.push(identity);
            }
            if let Some(spec) = contract_spec(&item) {
                specs.push(spec);
            }
        }
        self.specs_by_native.clear();
        self.active_by_native.clear();
        self.native_by_base_quote.clear();
        self.native_by_normalized.clear();
        for spec in specs {
            self.specs_by_native
                .insert(spec.native_symbol.clone(), spec);
        }
        for identity in identities {
            let native = identity.native_symbol;
            let base = identity.normalized_symbol;
            self.active_by_native.insert(native.clone(), base.clone());
            self.native_by_base_quote.insert(
                base_quote_key(&base, &identity.quote_currency),
                native.clone(),
            );
            if base_counts.get(&base) == Some(&1) {
                self.native_by_normalized.insert(base, native);
            }
        }
        self.fetched_at_ms.store(now_ms(), Ordering::Relaxed);
    }

    fn cached_unit(&self, symbol: &str) -> Option<f64> {
        self.cached_native_symbol(symbol).and_then(|native| {
            self.specs_by_native
                .get(&native)
                .map(|spec| spec.order_unit)
        })
    }

    fn cached_spec(&self, symbol: &str) -> ExchangeResult<KucoinContractSpec> {
        let native = self
            .cached_native_symbol(symbol)
            .ok_or_else(|| unsupported_contract(symbol))?;
        self.specs_by_native
            .get(&native)
            .map(|spec| spec.clone())
            .ok_or_else(|| {
                validation_error(format!(
                    "kucoin contract {native} lacks complete native sizing constraints"
                ))
            })
    }

    fn is_fresh(&self) -> bool {
        let last = self.fetched_at_ms.load(Ordering::Relaxed);
        last != 0 && now_ms().saturating_sub(last) < CONTRACT_MULTIPLIER_TTL_MS
    }
}

fn requested_quote(requested: &str) -> Option<&'static str> {
    const QUOTES: &[(&str, &str)] = &[
        ("-USDT-SWAP", "USDT"),
        ("-USDC-SWAP", "USDC"),
        ("-USD-SWAP", "USD"),
        ("-USDT-PERP", "USDT"),
        ("-USDC-PERP", "USDC"),
        ("-USD-PERP", "USD"),
        ("_USDT_PERP", "USDT"),
        ("_USDC_PERP", "USDC"),
        ("_USD_PERP", "USD"),
        ("-USDT", "USDT"),
        ("-USDC", "USDC"),
        ("-USD", "USD"),
        ("_USDT", "USDT"),
        ("_USDC", "USDC"),
        ("_USD", "USD"),
        ("/USDT", "USDT"),
        ("/USDC", "USDC"),
        ("/USD", "USD"),
        ("USDTM", "USDT"),
        ("USDCM", "USDC"),
        ("USDM", "USD"),
        ("USDT", "USDT"),
        ("USDC", "USDC"),
    ];
    QUOTES
        .iter()
        .find_map(|(suffix, quote)| requested.ends_with(suffix).then_some(*quote))
}

fn base_quote_key(base: &str, quote: &str) -> String {
    format!("{}|{}", normalize_base(base), quote.to_ascii_uppercase())
}

fn requested_base(symbol: &str, quote: &str) -> String {
    let suffixes = [
        format!("-{quote}-SWAP"),
        format!("-{quote}-PERP"),
        format!("_{quote}_PERP"),
        format!("-{quote}"),
        format!("_{quote}"),
        format!("/{quote}"),
        format!("{quote}M"),
        quote.to_owned(),
    ];
    suffixes
        .iter()
        .find_map(|suffix| symbol.strip_suffix(suffix))
        .unwrap_or(symbol)
        .to_owned()
}

fn normalize_base(base: &str) -> String {
    match base.to_ascii_uppercase().as_str() {
        "XBT" => "BTC".to_owned(),
        other => other.to_owned(),
    }
}

fn unsupported_contract(symbol: &str) -> crate::error::ExchangeError {
    validation_error(format!(
        "kucoin contract {symbol} is unknown or ambiguous; use an exact native symbol or BASE-QUOTE"
    ))
}

#[cfg(test)]
#[path = "kucoin_contracts_tests.rs"]
mod tests;
