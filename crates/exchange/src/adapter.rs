//! 交易所适配器统一接口。
//!
//! 所有具体适配器（M3 起：Binance / OKX / Bybit / ...）必须实现 [`ExchangeAdapter`]。
//! V1 范围内只包含公共行情与私有读接口；下单类（`place_order` / `cancel_order`）
//! 在 V2 引入新的 trait `LiveTradingAdapter`，与本 trait 解耦。

use crate::error::{ExchangeError, ExchangeResult};
use crate::transfer_network::{
    CurrencyTransferNetwork, TransferDestinationEvidence, TransferDestinationRequest,
};
use async_trait::async_trait;
use shared_types::instrument_registry::VenueInstrument;
use shared_types::{
    BalanceInfo, FundingPaymentData, FundingRateData, GateCrossExRouteQuote,
    IndexCompositionQuality, IndexCompositionSnapshot, MarkIndexInfo, OrderBookInfo, OrderInfo,
    PositionInfo, SpotTick, TickerInfo,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataRefreshOutcome {
    Refreshed,
    NotRequired,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PublicWsSnapshot<T> {
    Ready(Vec<T>),
    Pending,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicWsSubscribeOutcome {
    Confirmed,
    Requested,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicWsIngestOutcome {
    Ingested,
    AwaitingFirstEvent,
    Unsupported,
}

impl<T> PublicWsSnapshot<T> {
    /// Separate subscription acknowledgement from snapshot availability.
    /// A pending cache only proves that the adapter accepted the touch; it
    /// must not be reported as a confirmed venue subscription.
    pub fn subscribe_outcome(&self) -> PublicWsSubscribeOutcome {
        match self {
            Self::Ready(rows) if !rows.is_empty() => PublicWsSubscribeOutcome::Confirmed,
            Self::Ready(_) => PublicWsSubscribeOutcome::Requested,
            Self::Pending => PublicWsSubscribeOutcome::Requested,
            Self::Unsupported => PublicWsSubscribeOutcome::Unsupported,
        }
    }

    /// Keep event ingestion distinct from the subscription request. An empty
    /// ready snapshot does not prove that the venue delivered a usable frame.
    pub fn ingest_outcome(&self) -> PublicWsIngestOutcome {
        match self {
            Self::Ready(rows) if !rows.is_empty() => PublicWsIngestOutcome::Ingested,
            Self::Ready(_) | Self::Pending => PublicWsIngestOutcome::AwaitingFirstEvent,
            Self::Unsupported => PublicWsIngestOutcome::Unsupported,
        }
    }
}

#[async_trait]
pub trait ExchangeAdapter: Send + Sync {
    /// 交易所唯一标识，例如 `"binance"`、`"okx"`。
    fn name(&self) -> &'static str;

    // ========== 公共接口（无需认证） ==========

    async fn get_funding_rate(&self, symbol: &str) -> ExchangeResult<FundingRateData>;

    /// 批量获取资金费率。`symbols = None` 表示返回该交易所所有可获取的费率。
    async fn get_funding_rates(
        &self,
        symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<FundingRateData>>;

    async fn get_ticker(&self, symbol: &str) -> ExchangeResult<TickerInfo>;

    async fn get_tickers(&self, symbols: Option<&[String]>) -> ExchangeResult<Vec<TickerInfo>>;

    /// Touch the public ticker subscription and return only genuinely WS-backed rows.
    async fn public_ws_ticker_snapshot(
        &self,
        _symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<TickerInfo>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    /// Touch the public funding subscription and return only genuinely WS-backed rows.
    async fn public_ws_funding_snapshot(
        &self,
        _symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<FundingRateData>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    /// Touch official public mark/index subscriptions and return only rows
    /// observed from WS. Implementations must not hide a REST fallback here.
    async fn public_ws_mark_index_snapshot(
        &self,
        _symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<MarkIndexInfo>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    /// Touch the public spot-ticker subscription and return only genuinely
    /// WS-backed rows（bid/ask 需求使全市场流不可用，按符号子集订阅）。
    async fn public_ws_spot_snapshot(
        &self,
        _symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<SpotTick>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    /// Return the adapter's exact selected-symbol WS wait or rejection state.
    /// This keeps a reconnecting subscription distinct from an unsupported pair.
    fn public_ws_spot_problem(&self, _symbol: &str) -> Option<String> {
        None
    }

    /// Preserve an upstream router's exact native route identity. Generic
    /// venue adapters do not implement this surface; Gate `CrossEx` uses it so
    /// `{Exchange}_{Business}_{Base}_{Counter}` is never reconstructed later.
    async fn public_ws_route_quote_snapshot(
        &self,
        _symbols: &[String],
    ) -> ExchangeResult<PublicWsSnapshot<GateCrossExRouteQuote>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    async fn get_mark_index_prices(
        &self,
        _symbols: Option<&[String]>,
    ) -> ExchangeResult<Vec<MarkIndexInfo>> {
        Err(ExchangeError::UnsupportedCapability("mark_index_prices"))
    }

    async fn get_index_composition(
        &self,
        _symbol: &str,
    ) -> ExchangeResult<IndexCompositionSnapshot> {
        Err(ExchangeError::UnsupportedCapability("index_composition"))
    }

    async fn get_spot_tickers(&self, _symbols: Option<&[String]>) -> ExchangeResult<Vec<SpotTick>> {
        Err(ExchangeError::UnsupportedCapability("spot_tickers"))
    }

    async fn get_orderbook(&self, symbol: &str, depth: u32) -> ExchangeResult<OrderBookInfo>;

    /// Touch an on-demand public perpetual-orderbook subscription and return
    /// only a genuinely WS-backed snapshot. Execution callers must fail
    /// closed while this snapshot is pending or unsupported.
    async fn public_ws_orderbook_snapshot(
        &self,
        _symbol: &str,
        _depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    async fn get_spot_orderbook(
        &self,
        _symbol: &str,
        _depth: u32,
    ) -> ExchangeResult<OrderBookInfo> {
        Err(ExchangeError::UnsupportedCapability("spot_orderbook"))
    }

    /// Touch an on-demand public spot-orderbook subscription and return only
    /// a genuinely WS-backed snapshot. Execution callers must fail closed
    /// while this snapshot is pending or unsupported.
    async fn public_ws_spot_orderbook_snapshot(
        &self,
        _symbol: &str,
        _depth: u32,
    ) -> ExchangeResult<PublicWsSnapshot<OrderBookInfo>> {
        Ok(PublicWsSnapshot::Unsupported)
    }

    /// PR-BM/PR-AL: 从交易所官方 instrument/`exchangeInfo` endpoint 拉取全量下单
    /// 规格，映射为 [`VenueInstrument`]——instrument registry 启动期/周期灌库用。
    ///
    /// 默认未实现：没有 instrument feed 的 venue 不覆写，调用方据此判定该 venue
    /// 「尚未接入 registry」、仍由能力/运行态闸门把关。覆写的 venue 必须只产出
    /// 官方核验（`source = OfficialEndpoint`）的条目。
    async fn fetch_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        Err(ExchangeError::NotImplemented("fetch_instruments"))
    }

    /// Fetch only official Spot trading specifications. On-chain comparison
    /// must not wait for an unrelated perpetual metadata endpoint before it
    /// can prove the selected CEX pair's tick, lot and minimum-order rules.
    async fn fetch_spot_instruments(&self) -> ExchangeResult<Vec<VenueInstrument>> {
        let rows = self.fetch_instruments().await?;
        Ok(rows
            .into_iter()
            .filter(|row| {
                row.product_type
                    .as_deref()
                    .is_some_and(|product| product.eq_ignore_ascii_case("spot"))
            })
            .collect())
    }

    /// Fetch official currency/network deposit and withdrawal status. This is
    /// cold reference metadata and must never be called from the scan hot path.
    async fn fetch_transfer_networks(&self) -> ExchangeResult<Vec<CurrencyTransferNetwork>> {
        Err(ExchangeError::UnsupportedCapability("transfer_networks"))
    }

    /// Explicit, on-demand cash inventory and pair-specific fees for stock comparisons.
    /// Never called from public market polling or treated as order permission.
    fn stock_account_fingerprint(&self) -> Option<String> { None }

    async fn prepare_stock_submission(&self) -> ExchangeResult<()> {
        Err(ExchangeError::UnsupportedCapability("stock_paired_submission"))
    }
    async fn warm_stock_receipts(&self) -> ExchangeResult<()> {
        Err(ExchangeError::UnsupportedCapability("stock_receipts"))
    }
    async fn submit_stock_order(&self, _draft: shared_types::stocks::StockPeerOrderDraft, _client: String) -> ExchangeResult<shared_types::stocks::StockPeerOrderReceipt> {
        Err(ExchangeError::UnsupportedCapability("stock_paired_submission"))
    }
    fn track_stock_order(&self, _receipt: shared_types::stocks::StockPeerOrderReceipt) -> ExchangeResult<()> {
        Err(ExchangeError::UnsupportedCapability("stock_receipts"))
    }
    fn stock_order_receipt(&self, _client: &str) -> Option<shared_types::stocks::StockPeerOrderReceipt> { None }
    async fn reconcile_stock_order(&self, _original: &shared_types::stocks::StockPeerOrderReceipt) -> ExchangeResult<Option<shared_types::stocks::StockPeerOrderReceipt>> {
        Err(ExchangeError::UnsupportedCapability("stock_order_history"))
    }
    fn subscribe_stock_receipts(&self) -> ExchangeResult<tokio::sync::broadcast::Receiver<shared_types::stocks::StockPeerOrderReceipt>> {
        Err(ExchangeError::UnsupportedCapability("stock_receipts"))
    }

    async fn stock_cash_account(&self, _native_symbol: &str) -> ExchangeResult<shared_types::stocks::StockPeerAccount> {
        Err(ExchangeError::UnsupportedCapability("stock_cash_account"))
    }

    async fn stock_funding_methods(&self, _native_symbol: &str) -> ExchangeResult<Vec<shared_types::stocks::StockPeerFundingRoute>> {
        Err(ExchangeError::UnsupportedCapability("stock_funding_methods"))
    }

    /// Validate a stock order without entering the matching engine. Not a live ack.
    async fn validate_stock_order(&self, _draft: &shared_types::stocks::StockPeerOrderDraft) -> ExchangeResult<shared_types::stocks::StockPeerOrderCheck> {
        Err(ExchangeError::UnsupportedCapability("stock_order_validation"))
    }

    /// Fetch official transfer metadata only for currencies involved in the
    /// current candidate. Venues with an all-currency endpoint reuse the
    /// cached full response; asset-scoped venues can override this method and
    /// avoid an expensive account-wide fanout.
    async fn fetch_transfer_networks_for(
        &self,
        currencies: &[String],
    ) -> ExchangeResult<Vec<CurrencyTransferNetwork>> {
        let rows = self.fetch_transfer_networks().await?;
        if currencies.is_empty() {
            return Ok(rows);
        }
        let requested = currencies
            .iter()
            .map(|currency| currency.trim().to_ascii_uppercase())
            .filter(|currency| !currency.is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        Ok(rows
            .into_iter()
            .filter(|row| requested.contains(&row.currency.to_ascii_uppercase()))
            .collect())
    }

    /// Fetch the official destination address evidence only after a candidate
    /// needs inventory movement. Implementations must not call a withdrawal
    /// write endpoint here.
    async fn fetch_transfer_destination(
        &self,
        _request: &TransferDestinationRequest,
    ) -> ExchangeResult<TransferDestinationEvidence> {
        Err(ExchangeError::UnsupportedCapability("transfer_destination"))
    }

    // ========== 私有接口（V1 范围：只读） ==========

    /// 触发认证流程（部分交易所如 Deribit 需要 token 交换）。
    /// 默认实现：无需认证。
    async fn authenticate(&mut self) -> ExchangeResult<()> {
        Ok(())
    }

    /// PR-DP-04 follow-up: 冷启动 metadata prewarm 入口。
    ///
    /// lifecycle 启动时按 venue 并发调用，让 24h TTL 的元数据缓存
    /// （Binance `fundingInfo` / Gate `contracts` / KuCoin contract 元
    /// 数据等）在 hot path 之前完成首次 REST 拉取。默认实现 `Ok(())`，
    /// 没有元数据 cache 的 venue（OKX / Bybit / Bitget V3 / Hyperliquid）
    /// 不需要覆写。失败必须由调用方降级处理（保留 `8h fallback` 等
    /// 既有兜底），不能阻塞启动。
    async fn refresh_metadata(&self) -> ExchangeResult<MetadataRefreshOutcome> {
        Ok(MetadataRefreshOutcome::NotRequired)
    }

    async fn get_balance(
        &self,
        _currency: Option<&str>,
    ) -> ExchangeResult<HashMap<String, BalanceInfo>> {
        Err(ExchangeError::NotImplemented("get_balance"))
    }

    async fn get_positions(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        Err(ExchangeError::NotImplemented("get_positions"))
    }

    async fn get_open_orders(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        Err(ExchangeError::NotImplemented("get_open_orders"))
    }

    async fn get_funding_payments(
        &self,
        _symbol: Option<&str>,
        _start_time_ms: Option<i64>,
        _end_time_ms: Option<i64>,
    ) -> ExchangeResult<Vec<FundingPaymentData>> {
        Err(ExchangeError::NotImplemented("get_funding_payments"))
    }

    // ========== 符号转换 ==========

    /// 将交易所原生符号标准化为统一表示（一般取基础币种，如 "BTC"）。
    fn normalize_symbol(&self, symbol: &str) -> String;

    /// 将统一基础币种转换为交易所原生符号（如 "BTC" → "BTCUSDT" for Binance）。
    fn to_exchange_symbol(&self, symbol: &str) -> String;
}

pub(crate) fn unverified_index_composition(
    venue: &str,
    symbol: &str,
    index_id: impl Into<String>,
    source: impl Into<String>,
    reason: impl Into<String>,
) -> IndexCompositionSnapshot {
    IndexCompositionSnapshot {
        venue: venue.to_owned(),
        symbol: strip_common_suffixes(symbol),
        index_id: index_id.into(),
        components: Vec::new(),
        quality: IndexCompositionQuality::Unverified,
        source: source.into(),
        received_at_ms: common::time::now_ms(),
        freshness_ms: None,
        error: Some(reason.into()),
        retry_after_ms: None,
        source_url: None,
        payload_sha256: None,
        schema_version: None,
    }
}

/// 官方 payload 证据：请求 URL 与原始响应字节 sha256，供指数成分证据注册表追溯。
#[derive(Debug, Clone)]
pub(crate) struct PayloadEvidence {
    pub(crate) source_url: String,
    pub(crate) payload_sha256: String,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 读取响应文本并生成 payload 证据；非 2xx 走 `ExchangeError::Http`。
pub(crate) async fn checked_text_with_evidence(
    resp: reqwest::Response,
    source_url: impl Into<String>,
) -> ExchangeResult<(String, PayloadEvidence)> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|error| ExchangeError::Network(error.to_string()))?;
    if !status.is_success() {
        return Err(ExchangeError::Http {
            status: status.as_u16(),
            body,
        });
    }
    let evidence = PayloadEvidence {
        source_url: source_url.into(),
        payload_sha256: sha256_hex(body.as_bytes()),
    };
    Ok((body, evidence))
}

pub(crate) fn attach_payload_evidence(
    mut snapshot: IndexCompositionSnapshot,
    evidence: PayloadEvidence,
) -> IndexCompositionSnapshot {
    snapshot.source_url = Some(evidence.source_url);
    snapshot.payload_sha256 = Some(evidence.payload_sha256);
    snapshot
}

/// 跨适配器共享：在 Hedge Mode 下，把同 symbol 的多条仓位互相 `paired_with` 标记。
///
/// 为什么放在共享层（`adapter.rs`）：Binance / Bybit / OKX / Bitget 都支持 Hedge Mode，
/// 写在每个 adapter 里会复制 ~20 行算法。本函数只做一件事——按 symbol 分组，组内
/// 互相赋值 `paired_with = "{exchange}:{symbol}:{side}"`，下游 portfolio 求净暴露
/// 时知道这是配对仓位，不重复计算。
///
/// 算法：
/// 1. 按 symbol 分组到 `HashMap<symbol, Vec<index>>`。
/// 2. 每组若 ≥ 2 条记录，对每条 `paired_with` = 同组其他记录的 `label` 列表（逗号分隔）。
///
/// One-Way Mode 下每个 `symbol` 仅 1 条记录，`paired_with` 保持 `None`。
pub fn pair_hedge_positions(positions: &mut [shared_types::PositionInfo]) {
    let mut by_symbol: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in positions.iter().enumerate() {
        by_symbol.entry(p.symbol.clone()).or_default().push(i);
    }
    for indices in by_symbol.values() {
        if indices.len() < 2 {
            continue;
        }
        // 先收集所有 (idx, label) 避免后续同时借用 positions
        let labels: Vec<(usize, String)> = indices
            .iter()
            .map(|&i| {
                let p = &positions[i];
                (i, format!("{}:{}:{}", p.exchange, p.symbol, p.side))
            })
            .collect();
        for &(i, _) in &labels {
            let peers: Vec<String> = labels
                .iter()
                .filter(|(j, _)| *j != i)
                .map(|(_, label)| label.clone())
                .collect();
            if !peers.is_empty() {
                positions[i].paired_with = Some(peers.join(","));
            }
        }
    }
}

/// 默认的 normalize 工具：剥离常见后缀。多数适配器可直接复用。
///
/// 修复 P2 1.5 配套：补全 USDC 计价后缀（`USDC` / `-USDC` / `_USDC` / `/USDC` / `-USDC-SWAP`）。
/// 顺序很重要：长后缀必须在短后缀之前匹配，否则 `BTC-USDC-SWAP` 会被 `-SWAP` 先吃掉
/// 留下 `BTC-USDC`，再次扫描才能剥到 `BTC`。当前实现是单次扫描首匹配 break，因此
/// 长后缀必须先列出。
pub fn strip_common_suffixes(symbol: &str) -> String {
    let upper = symbol.to_ascii_uppercase();
    const SUFFIXES: &[&str] = &[
        "-USDT-SWAP",
        "-USDC-SWAP",
        "-USDT-PERP",
        "-USDC-PERP",
        "_USDT_PERP",
        "_USDC_PERP",
        "USDTM",
        "-USDT",
        "-USDC",
        "_USDT",
        "_USDC",
        "/USDT",
        "/USDC",
        "USDT",
        "USDC",
        "-PERP",
        "_PERP",
        "-SWAP",
    ];
    let mut out = upper;
    for s in SUFFIXES {
        if out.ends_with(s) {
            out.truncate(out.len() - s.len());
            break;
        }
    }
    out
}

/// Collapse a venue-reported client order id string to `Option<String>`,
/// treating empty/whitespace and a bare `0` placeholder as "no client id".
pub fn client_order_id_from_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "0" {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Parse a venue-reported reduce-only / close-only string flag. Empty stays
/// `None` (venue did not report it); `0`/`false` is an explicit `false`.
pub fn reduce_only_from_str(raw: &str) -> Option<bool> {
    match raw.trim() {
        "" => None,
        "0" | "false" | "False" | "FALSE" => Some(false),
        _ => Some(true),
    }
}

/// Parse a venue reduce-only flag expressed as `yes`/`no` (Bitget UTA) or the
/// usual `true`/`false`/`1`/`0` text. An empty value collapses to `None`;
/// unrecognised text is returned as `Err(trimmed)` so callers fail closed
/// instead of guessing a direction.
pub fn reduce_only_from_yes_no(raw: &str) -> Result<Option<bool>, &str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else if trimmed.eq_ignore_ascii_case("yes")
        || trimmed.eq_ignore_ascii_case("true")
        || trimmed == "1"
    {
        Ok(Some(true))
    } else if trimmed.eq_ignore_ascii_case("no")
        || trimmed.eq_ignore_ascii_case("false")
        || trimmed == "0"
    {
        Ok(Some(false))
    } else {
        Err(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn client_order_id_from_str_collapses_empty_and_zero() {
        assert_eq!(client_order_id_from_str(""), None);
        assert_eq!(client_order_id_from_str("   "), None);
        assert_eq!(client_order_id_from_str("0"), None);
        assert_eq!(
            client_order_id_from_str(" cli-1 "),
            Some("cli-1".to_owned())
        );
    }

    #[test]
    fn reduce_only_from_str_maps_known_flags() {
        assert_eq!(reduce_only_from_str(""), None);
        assert_eq!(reduce_only_from_str("false"), Some(false));
        assert_eq!(reduce_only_from_str("0"), Some(false));
        assert_eq!(reduce_only_from_str("true"), Some(true));
    }

    #[test]
    fn attach_payload_evidence_fills_source_url_and_hash() {
        let snapshot = unverified_index_composition("mock", "BTC", "BTCUSDT", "test", "none");
        let evidence = PayloadEvidence {
            source_url: "https://example.com/constituents?symbol=BTCUSDT".into(),
            payload_sha256: sha256_hex(b"{}"),
        };
        let snapshot = attach_payload_evidence(snapshot, evidence);
        assert_eq!(
            snapshot.source_url.as_deref(),
            Some("https://example.com/constituents?symbol=BTCUSDT")
        );
        assert_eq!(
            snapshot.payload_sha256.as_deref(),
            Some("44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a")
        );
    }

    #[test]
    fn public_ws_snapshot_exposes_explicit_subscribe_outcome() {
        assert_eq!(
            PublicWsSnapshot::<TickerInfo>::Pending.subscribe_outcome(),
            PublicWsSubscribeOutcome::Requested
        );
        assert_eq!(
            PublicWsSnapshot::<()>::Ready(vec![()]).subscribe_outcome(),
            PublicWsSubscribeOutcome::Confirmed
        );
        assert_eq!(
            PublicWsSnapshot::<TickerInfo>::Ready(Vec::new()).subscribe_outcome(),
            PublicWsSubscribeOutcome::Requested
        );
        assert_eq!(
            PublicWsSnapshot::<TickerInfo>::Unsupported.subscribe_outcome(),
            PublicWsSubscribeOutcome::Unsupported
        );
        assert_eq!(
            PublicWsSnapshot::<()>::Ready(vec![()]).ingest_outcome(),
            PublicWsIngestOutcome::Ingested
        );
        assert_eq!(
            PublicWsSnapshot::<TickerInfo>::Ready(Vec::new()).ingest_outcome(),
            PublicWsIngestOutcome::AwaitingFirstEvent
        );
        assert_eq!(
            PublicWsSnapshot::<TickerInfo>::Unsupported.ingest_outcome(),
            PublicWsIngestOutcome::Unsupported
        );
    }

    #[test]
    fn strip_suffixes() {
        assert_eq!(strip_common_suffixes("BTCUSDT"), "BTC");
        assert_eq!(strip_common_suffixes("BTC-USDT-SWAP"), "BTC");
        assert_eq!(strip_common_suffixes("ETH-PERP"), "ETH");
        assert_eq!(strip_common_suffixes("BTCUSDTM"), "BTC");
        assert_eq!(strip_common_suffixes("SOL"), "SOL");
        assert_eq!(strip_common_suffixes("eth/usdt"), "ETH");
    }

    #[test]
    fn strip_usdc_suffixes() {
        // 修复 P2 1.5 配套：USDC 计价合约也应正确剥离到 base symbol。
        assert_eq!(strip_common_suffixes("BTCUSDC"), "BTC");
        assert_eq!(strip_common_suffixes("ETHUSDC"), "ETH");
        assert_eq!(strip_common_suffixes("BTC-USDC-SWAP"), "BTC");
        assert_eq!(strip_common_suffixes("BTC-USDC-PERP"), "BTC");
        assert_eq!(strip_common_suffixes("BTC-USDC"), "BTC");
        assert_eq!(strip_common_suffixes("BTC_USDC"), "BTC");
        assert_eq!(strip_common_suffixes("BTC/USDC"), "BTC");
    }

    fn make_position(exchange: &str, symbol: &str, side: &str) -> shared_types::PositionInfo {
        shared_types::PositionInfo {
            symbol: symbol.into(),
            exchange: exchange.into(),
            side: side.into(),
            quantity: 1.0,
            entry_price: 100.0,
            mark_price: 100.0,
            unrealized_pnl: 0.0,
            leverage: 1.0,
            liquidation_price: None,
            liquidation_distance_pct: None,
            next_funding_ms: None,
            paired_with: None,
            margin: 0.0,
            maintenance_margin_ratio: 0.0,
            position_mode: None,
            margin_mode: None,
            risk_rate: None,
            available_position: None,
            frozen_position: None,
        }
    }

    #[test]
    fn pair_hedge_positions_links_long_short() {
        let mut positions = vec![
            make_position("bybit", "BTC", "long"),
            make_position("bybit", "BTC", "short"),
            make_position("bybit", "ETH", "long"), // 仅一条，无 pair
        ];
        pair_hedge_positions(&mut positions);

        let btc_long = positions
            .iter()
            .find(|p| p.symbol == "BTC" && p.side == "long")
            .unwrap();
        let btc_short = positions
            .iter()
            .find(|p| p.symbol == "BTC" && p.side == "short")
            .unwrap();
        let eth = positions.iter().find(|p| p.symbol == "ETH").unwrap();
        assert_eq!(btc_long.paired_with.as_deref(), Some("bybit:BTC:short"));
        assert_eq!(btc_short.paired_with.as_deref(), Some("bybit:BTC:long"));
        assert!(eth.paired_with.is_none());
    }
}
