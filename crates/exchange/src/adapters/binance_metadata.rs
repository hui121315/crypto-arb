//! Binance metadata refresh helpers for funding intervals and order filters.

use super::binance_exchange_info::{
    instrument_specs_from_response, instruments_from_response, BinanceInstrumentSpec,
    ExchangeInfoCache,
};
use super::binance_funding_info::{funding_interval_map, FundingIntervalCache};
use super::binance_public_rest as public_rest;
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use common::time::now_ms;
use shared_types::instrument_registry::VenueInstrument;

pub(super) async fn refresh_funding_intervals(
    http: &HttpClient,
    base_url: &str,
    cache: &FundingIntervalCache,
) -> ExchangeResult<()> {
    if cache.is_fresh(now_ms()) {
        return Ok(());
    }

    let items = public_rest::funding_info(http, base_url).await?;
    cache.replace(funding_interval_map(items), now_ms());
    Ok(())
}

pub(super) fn funding_interval_for(cache: &FundingIntervalCache, symbol: &str) -> u32 {
    cache.interval_for(symbol)
}

/// 拉取官方 `exchangeInfo` 并映射为 [`VenueInstrument`] 列表（instrument
/// registry 启动期/周期刷新用）。不走 cache——registry 刷新有独立周期，且需要
/// 一次性拿到全量 symbol，而非按需单条。
pub(super) async fn fetch_instruments(
    http: &HttpClient,
    base_url: &str,
) -> ExchangeResult<Vec<VenueInstrument>> {
    let info = public_rest::exchange_info(http, base_url).await?;
    Ok(instruments_from_response(&info, now_ms()))
}

pub(super) async fn instrument_spec(
    http: &HttpClient,
    base_url: &str,
    cache: &ExchangeInfoCache,
    symbol: &str,
) -> ExchangeResult<BinanceInstrumentSpec> {
    if cache.is_fresh(now_ms()) {
        return cache.resolve(symbol);
    }

    let info = public_rest::exchange_info(http, base_url).await?;
    let new_map = instrument_specs_from_response(info);
    cache.replace(new_map, now_ms());
    cache.resolve(symbol)
}
