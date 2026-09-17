#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! 交易所适配层。
//!
//! `M2` 阶段建立基础设施：
//!
//! - `HTTP` / `WS`
//! - 限频
//! - 适配器接口
//! - 聚合器
//!
//! `M3` 阶段在此之上实现具体交易所适配器。

pub mod adapter;
pub mod adapters;
pub mod aggregator;
pub mod client_order_id_policy;
pub mod error;
pub mod http;
pub mod http_metrics;
pub mod instrument_evidence;
pub mod live;
pub mod rest_registry;
pub mod services;
pub mod signing;
pub mod spot;
pub mod spot_coverage;
pub mod transfer_network;
pub mod venue_capability;
pub mod venue_spec;
pub mod ws;

pub use adapter::{
    strip_common_suffixes, ExchangeAdapter, MetadataRefreshOutcome, PublicWsIngestOutcome,
    PublicWsSnapshot, PublicWsSubscribeOutcome,
};
pub use adapters::{
    hyperliquid_signer_session_health, Binance, BinanceConfig, BinanceCredentials, Bitget,
    BitgetConfig, BitgetCredentials, Bybit, BybitConfig, BybitCredentials, Gate, GateConfig,
    GateCredentials, GateCrossEx, GateCrossExConfig, GateCrossExCredentials, Hyperliquid,
    HyperliquidAccountAbstraction, HyperliquidConfig, HyperliquidCredentialRelation,
    HyperliquidCredentials, HyperliquidMarket, HyperliquidSignerSessionHealth, Kraken,
    KrakenConfig, KrakenCredentials, KrakenFuturesCredentials, KrakenSpotCredentials, Kucoin,
    KucoinConfig, KucoinCredentials, Okx, OkxConfig, OkxCredentials, OkxLive, OkxLiveConfig,
    OkxLiveCredentials,
};
pub use aggregator::{Aggregator, FanoutReport, FanoutVenueResult};
pub use client_order_id_policy::client_order_id_policy;
pub use error::{ExchangeError, ExchangeResult};
pub use http::{HttpClient, HttpClientBuilder};
pub use http_metrics::{
    http_outcome_metrics_snapshot, http_quality_window_snapshot, http_request_metrics_snapshot,
    HttpLatencyBucketSnapshot, HttpOutcomeMetricSnapshot, HttpQualityWindowSnapshot,
    HttpRequestMetricSnapshot,
};
pub use instrument_evidence::{
    official_instrument_evidence_matches, spot_instrument_evidence_matches,
};
pub use live::{
    ExchangeCapabilities, LiveTradingAdapter, PrivateWsRuntimeStatus, VenueAccountRead,
    VenueAccountReadIssue,
};
pub use rest_registry::rest_endpoint_registry;
pub use services::host_gate::{host_gate_snapshots, HostGateSnapshot};
pub use services::{rate_limiter_snapshots, RateLimiter, RateLimiterSnapshot};
pub use spot_coverage::{
    plan_symbol_coverage, CoveragePlan, SymbolCoverage, SymbolCoverageStatus, VenueListing,
};
pub use transfer_network::{
    canonical_network_id, contracts_compatible, parse_optional_decimal, CurrencyTransferNetwork,
    DepositStatus, DepositStatusEvidence, DepositStatusRequest, TransferDestinationEvidence,
    TransferDestinationRequest, TransferDestinationStatus, TransferDirection,
    WithdrawalSourceBalance, WithdrawalSourceBalanceRequest, WithdrawalStatus,
    WithdrawalStatusEvidence, WithdrawalStatusRequest, WithdrawalSubmission,
    WithdrawalSubmitRequest, WithdrawalWalletType, TRANSFER_NETWORK_FRESHNESS_MS,
};
pub use venue_capability::{
    static_exchange_capabilities, static_venue_capability_matrix,
    static_venue_capability_matrix_for_product, venue_capability_matrix,
    venue_capability_matrix_for_product, LIVE_VENUE_FAMILIES,
};

pub use venue_spec::{
    endpoint_evidence, endpoint_weight, EndpointDataKind, EndpointEvidenceSnapshot, EndpointSpec,
    EndpointUseCase, HttpMethod, RateScope, VenueDefaults, VenueId, VenueSpec,
};
pub use ws::{
    trading_ws_operation_registry, trading_ws_venues, WsConfig, WsEvent, WsHeartbeat,
    WsInboundCodec, WsManager, WsServerPing, WsState, TRADING_WS_VENUE_COUNT,
};
