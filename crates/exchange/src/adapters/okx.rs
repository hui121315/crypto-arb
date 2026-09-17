//! OKX 永续合约 (USDT-margined SWAP) 适配器。
//!
//! V1 范围：公共行情 + 私有读接口（余额 / 持仓 / 挂单）。下单类（V2）不在此处。
//!
//! ## 性能注意
//! OKX REST `/public/funding-rate` 仅支持单 `instId` 查询，无批量接口。
//! 全量 funding 扫描使用 public WebSocket `funding-rate` 频道作为热路径；REST 仅保留给
//! 单 symbol 查询、测试 override、以及 WS 冷启动前的显式查询兜底。
//!
//! [`get_funding_rates`]: ExchangeAdapter::get_funding_rates

use crate::adapter::{
    strip_common_suffixes, ExchangeAdapter, MetadataRefreshOutcome, PublicWsSnapshot,
};
use crate::adapters::contract_orderbook::normalize_contract_book;
use crate::adapters::okx_config::SWAP_SUFFIX;
use crate::adapters::okx_funding::parse_funding;
use crate::adapters::okx_instruments::OkxContractValues;
use crate::adapters::okx_market_data::{
    funding_volume_map, orderbook_info, parse_index_components, parse_spot_tick, parse_ticker,
    spot_ticker_matches, usdt_swap_ticker,
};
use crate::adapters::okx_private_rest as private_rest;
use crate::adapters::okx_public_rest as public_rest;
use crate::adapters::okx_ws_funding::FundingStream as WsFundingStream;
use crate::adapters::okx_ws_mark_index::MarkIndexStream as WsMarkIndexStream;
use crate::adapters::okx_ws_market::MarketStream as WsMarketStream;
use crate::adapters::okx_ws_spot_ticker::SpotTickerStream as WsSpotTickerStream;
use crate::adapters::okx_ws_ticker::TickerStream as WsTickerStream;
use crate::error::{ExchangeError, ExchangeResult};
use crate::http::HttpClient;
use crate::services::RateLimiter;
use async_trait::async_trait;
use common::time::now_ms;
use shared_types::instrument_registry::VenueInstrument;
use shared_types::{
    BalanceInfo, FundingPaymentData, FundingRateData, IndexCompositionSnapshot, MarkIndexInfo,
    OrderBookInfo, OrderInfo, PositionInfo, SpotTick, TickerInfo,
};
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::{Arc, OnceLock};

pub use crate::adapters::okx_config::{OkxConfig, OkxCredentials};

const NAME: &str = "okx";

#[derive(Debug)]
pub struct Okx {
    config: OkxConfig,
    base_url: String,
    http: HttpClient,
    _rate_limiter: Arc<RateLimiter>,
    time_offset_ms: AtomicI64,
    time_synced_at_ms: AtomicI64,
    funding_stream: OnceLock<Arc<WsFundingStream>>,
    mark_index_stream: OnceLock<Arc<WsMarkIndexStream>>,
    market_stream: OnceLock<Arc<WsMarketStream>>,
    ticker_stream: OnceLock<Arc<WsTickerStream>>,
    spot_ticker_stream: OnceLock<Arc<WsSpotTickerStream>>,
    contract_values: OkxContractValues,
}

// ============== Trait 实现 ==============

#[async_trait]
impl ExchangeAdapter for Okx {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn refresh_metadata(&self) -> ExchangeResult<MetadataRefreshOutcome> {
        self.contract_values
            .refresh_all(&self.http, &self.base_url)
            .await?;
        self.sync_instrument_updates();
        Ok(MetadataRefreshOutcome::Refreshed)
    }

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData> {
        let inst_id = self.to_exchange_symbol(symbol);
        if let Some(stream) = self.ws_funding() {
            stream.touch_many(std::slice::from_ref(&inst_id));
            if let Some(row) = stream.latest(&inst_id, 0.0) {
                return Ok(row);
            }
        }
        let item = public_rest::funding_rate(&self.http, &self.base_url, &inst_id).await?;
        parse_funding(&item, 0.0).ok_or_else(|| {
            ExchangeError::Parse(format!(
                "okx funding {inst_id} missing rate or settlement window"
            ))
        })
    }

    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>> {
        let is_full_scan = symbols.is_none();
        let inst_ids: Vec<String> = match symbols {
            Some(syms) => syms.iter().map(|s| self.to_exchange_symbol(s)).collect(),
            None => public_rest::swap_inst_ids_rest(&self.http, &self.base_url).await?,
        };

        if inst_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ws_funding = self.ws_funding();
        if let Some(stream) = &ws_funding {
            stream.touch_many(&inst_ids);
            // 部分新鲜即服务：REST 聚合同样只返回存在的行，语义一致。
            if stream.fresh_count(&inst_ids) > 0 {
                let volume_map = funding_volume_map(
                    public_rest::swap_tickers(&self.http, &self.base_url).await?,
                );
                return Ok(stream.snapshot(&inst_ids, &volume_map));
            }
            if is_full_scan {
                return Ok(Vec::new());
            }
        }

        let (tickers, items) = tokio::try_join!(
            public_rest::swap_tickers(&self.http, &self.base_url),
            self.fetch_funding_rate_items_fast(&inst_ids)
        )?;
        let volume_map = funding_volume_map(tickers);

        let result = items
            .into_iter()
            .filter_map(|it| {
                let v = volume_map.get(&it.inst_id).copied().unwrap_or(0.0);
                parse_funding(&it, v)
            })
            .collect();

        Ok(result)
    }

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo> {
        let inst_id = self.to_exchange_symbol(symbol);
        // PR-DP-12 follow-up: try the WS `tickers` cache first; on a miss
        // fall back to the existing per-symbol REST endpoint.
        if let Some(stream) = self.ws_ticker() {
            stream.touch_many(std::slice::from_ref(&inst_id));
            if let Some(row) = stream.latest(&inst_id) {
                return Ok(row);
            }
        }
        let item = public_rest::ticker(&self.http, &self.base_url, &inst_id).await?;
        parse_ticker(&item).ok_or_else(|| {
            ExchangeError::Parse(format!("okx ticker {inst_id} missing bid/ask/last price"))
        })
    }

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>> {
        // PR-DP-12 follow-up: WS fast-path for explicit watchlist symbols.
        // Full-scan (`symbols == None`) keeps the REST aggregate because
        // there is no symbol universe cache yet.
        if let Some(symbols) = symbols.filter(|s| !s.is_empty()) {
            if let Some(stream) = self.ws_ticker() {
                let inst_ids: Vec<String> =
                    symbols.iter().map(|s| self.to_exchange_symbol(s)).collect();
                stream.touch_many(&inst_ids);
                // 部分新鲜即服务：REST 聚合本身也只返回存在的行，语义一致。
                let rows = stream.snapshot(&inst_ids);
                if !rows.is_empty() {
                    return Ok(rows);
                }
            }
        }
        let items = public_rest::swap_tickers(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(usdt_swap_ticker)
            .filter_map(|i| parse_ticker(&i))
            .collect())
    }

    async fn public_ws_ticker_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        let Some(stream) = self.ws_ticker() else {
            return Ok(PublicWsSnapshot::Unsupported);
        };
        let inst_ids = symbols
            .iter()
            .map(|symbol| self.to_exchange_symbol(symbol))
            .collect::<Vec<_>>();
        stream.touch_many(&inst_ids);
        let rows = stream.snapshot(&inst_ids);
        Ok((!rows.is_empty())
            .then_some(rows)
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    async fn public_ws_funding_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<FundingRateData>> {
        let Some(stream) = self.ws_funding() else {
            return Ok(PublicWsSnapshot::Unsupported);
        };
        let inst_ids = symbols
            .iter()
            .map(|symbol| self.to_exchange_symbol(symbol))
            .collect::<Vec<_>>();
        stream.touch_many(&inst_ids);
        let rows = stream.snapshot(&inst_ids, &HashMap::new());
        Ok((!rows.is_empty())
            .then_some(rows)
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    async fn public_ws_mark_index_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<MarkIndexInfo>> {
        let Some(stream) = self.ws_mark_index() else {
            return Ok(PublicWsSnapshot::Unsupported);
        };
        let inst_ids = symbols
            .iter()
            .map(|symbol| self.to_exchange_symbol(symbol))
            .collect::<Vec<_>>();
        stream.touch_many(&inst_ids);
        let rows = stream.snapshot(&inst_ids);
        Ok((!rows.is_empty())
            .then_some(rows)
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    async fn get_mark_index_prices(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        let inst_ids = symbols.map(|rows| {
            rows.iter()
                .map(|symbol| self.to_exchange_symbol(symbol))
                .collect::<Vec<_>>()
        });
        if let (Some(stream), Some(ids)) = (self.ws_mark_index(), inst_ids.as_ref()) {
            stream.touch_many(ids);
            let rows = stream.snapshot(ids);
            if !rows.is_empty() {
                return Ok(rows);
            }
        }
        public_rest::mark_index_prices(&self.http, &self.base_url, inst_ids.as_deref()).await
    }

    async fn get_index_composition(
        &self,
        symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        let index = okx_index_symbol(symbol);
        let (body, evidence) =
            public_rest::index_components(&self.http, &self.base_url, &index).await?;
        Ok(crate::adapter::attach_payload_evidence(
            parse_index_components(body),
            evidence,
        ))
    }

    async fn public_ws_spot_snapshot(
        &self,
        symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        let (Some(stream), Some(inst_ids)) = (
            self.ws_spot_ticker(),
            crate::adapters::okx_ws_spot_ticker::spot_inst_ids(symbols),
        ) else {
            return Ok(PublicWsSnapshot::Unsupported);
        };
        stream.touch_many(&inst_ids);
        let rows = stream.snapshot(&inst_ids);
        Ok((!rows.is_empty())
            .then_some(rows)
            .map_or(PublicWsSnapshot::Pending, PublicWsSnapshot::Ready))
    }

    fn public_ws_spot_problem(&self, _symbol: &str) -> Option<String> {
        self.ws_spot_ticker()?
            .connection_problem()
            .map(|problem| format!("OKX Spot WS 连接失败：{problem}"))
    }

    async fn get_spot_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        if let Some(symbols) = symbols.filter(|s| !s.is_empty()) {
            if let (Some(stream), Some(inst_ids)) = (
                self.ws_spot_ticker(),
                crate::adapters::okx_ws_spot_ticker::spot_inst_ids(symbols),
            ) {
                stream.touch_many(&inst_ids);
                let rows = stream.snapshot(&inst_ids);
                if !rows.is_empty() {
                    return Ok(rows);
                }
            }
        }
        let items = public_rest::spot_tickers(&self.http, &self.base_url).await?;
        Ok(items
            .into_iter()
            .filter(|item| spot_ticker_matches(item, symbols))
            .filter_map(|item| parse_spot_tick(&item))
            .collect())
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        let inst_id = self.to_exchange_symbol(symbol);
        let ws_book = self.ws_orderbook(symbol, depth);
        self.sync_instrument_updates();
        let contract_value = self
            .contract_values
            .contract_value(&self.http, &self.base_url, &inst_id)
            .await?;
        if let Some(book) = ws_book {
            return normalize_contract_book(book, contract_value);
        }

        let sz = depth.clamp(1, 400).to_string();
        let item =
            public_rest::orderbook(&self.http, &self.base_url, &inst_id, &sz, "books").await?;

        normalize_contract_book(
            orderbook_info(strip_common_suffixes(&inst_id), item),
            contract_value,
        )
    }

    async fn public_ws_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        let Some(book) = self.ws_orderbook(symbol, depth) else {
            return Ok(PublicWsSnapshot::Pending);
        };
        let inst_id = self.to_exchange_symbol(symbol);
        self.sync_instrument_updates();
        let contract_value = self
            .contract_values
            .contract_value(&self.http, &self.base_url, &inst_id)
            .await?;
        Ok(PublicWsSnapshot::Ready(vec![normalize_contract_book(
            book,
            contract_value,
        )?]))
    }

    async fn get_spot_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo> {
        if let Some(book) =
            super::okx_ws_spot_depth::latest_spot_orderbook(&self.config, symbol, depth)
        {
            return Ok(book);
        }
        let inst_id = crate::spot::native_pair_symbol(symbol, '-')
            .ok_or_else(|| ExchangeError::UnsupportedSymbol(symbol.to_owned()))?;
        // Official: <https://www.okx.com/docs-v5/en/#order-book-trading-market-data-get-order-book>
        let sz = depth.clamp(1, 400).to_string();
        let item =
            public_rest::orderbook(&self.http, &self.base_url, &inst_id, &sz, "spot books").await?;
        let symbol = crate::spot::native_pair_symbol(&inst_id, '/').unwrap_or(inst_id);
        Ok(orderbook_info(symbol, item))
    }

    async fn public_ws_spot_orderbook_snapshot(
        &self,
        symbol: &str,
        depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(
            match super::okx_ws_spot_depth::latest_spot_orderbook(&self.config, symbol, depth) {
                Some(book) => PublicWsSnapshot::Ready(vec![book]),
                None => PublicWsSnapshot::Pending,
            },
        )
    }

    async fn get_balance(
        &self,
        currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        self.sync_server_time_best_effort().await;
        let path = "/api/v5/account/balance";
        let headers = self.build_signed_headers("GET", path, "")?;
        private_rest::balances(&self.signed_request(path, &headers), currency).await
    }

    async fn get_funding_payments(
        &self,
        symbol: Option<&str>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        self.sync_server_time_best_effort().await;
        let inst_id = symbol.map(|value| self.to_exchange_symbol(value));
        let path = crate::adapters::funding_payments::okx_funding_bills_path(
            inst_id.as_deref(),
            start_time_ms,
            end_time_ms,
        );
        let headers = self.build_signed_headers("GET", &path, "")?;
        private_rest::funding_payments(&self.signed_request(&path, &headers)).await
    }

    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        self.sync_server_time_best_effort().await;
        let path = "/api/v5/account/positions?instType=SWAP";
        let headers = self.build_signed_headers("GET", path, "")?;
        let target = symbol.map(|s| self.to_exchange_symbol(s));
        private_rest::positions(&self.signed_request(path, &headers), target.as_deref()).await
    }

    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        self.sync_server_time_best_effort().await;
        // 修复 P2 2.7：用 `url::form_urlencoded` 标准编码（OKX `instId` 实际不含特殊字符，
        // 但保持与 `okx_live` 一致的实现风格，且为未来支持自定义 `instId` 留空间）。
        // `Serializer` 非 `Send`，限制到 block 内不跨越 await。
        let path = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("instType", "SWAP");
            if let Some(s) = symbol {
                serializer.append_pair("instId", &self.to_exchange_symbol(s));
            }
            format!("/api/v5/trade/orders-pending?{}", serializer.finish())
        };
        let headers = self.build_signed_headers("GET", &path, "")?;
        private_rest::open_orders(&self.signed_request(&path, &headers)).await
    }

    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let checked_at_ms = now_ms();
        let (_, spot) = tokio::try_join!(
            self.contract_values.refresh_all(&self.http, &self.base_url),
            super::spot_instruments::okx(&self.http, &self.base_url, checked_at_ms),
        )?;
        self.sync_instrument_updates();
        Ok(self
            .contract_values
            .venue_instruments(checked_at_ms)
            .into_iter()
            .chain(spot)
            .collect())
    }

    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        super::spot_instruments::okx(&self.http, &self.base_url, now_ms()).await
    }

    async fn fetch_transfer_networks(&self) -> ExchangeResult<Vec<crate::CurrencyTransferNetwork>> {
        self.sync_server_time_best_effort().await;
        super::okx_transfer_networks::fetch(&self.http, &self.base_url, &[], |path| {
            self.build_signed_headers("GET", path, "")
        })
        .await
    }

    async fn fetch_transfer_networks_for(
        &self,
        currencies: &[String],
    ) -> ExchangeResult<Vec<crate::CurrencyTransferNetwork>> {
        self.sync_server_time_best_effort().await;
        super::okx_transfer_networks::fetch(&self.http, &self.base_url, currencies, |path| {
            self.build_signed_headers("GET", path, "")
        })
        .await
    }

    fn normalize_symbol(&self, symbol: &str) -> String {
        strip_common_suffixes(symbol)
    }

    fn to_exchange_symbol(&self, symbol: &str) -> String {
        let norm = strip_common_suffixes(symbol);
        format!("{norm}{SWAP_SUFFIX}")
    }
}

fn okx_index_symbol(symbol: &str) -> String {
    let norm = strip_common_suffixes(symbol);
    format!("{norm}-USDT")
}

#[path = "okx_support.rs"]
mod support;

#[cfg(test)]
#[path = "okx_tests.rs"]
mod tests;
