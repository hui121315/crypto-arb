pub use shared_types::{VenueDefaults, VenueId, UNRECORDED_EVIDENCE_MARKER};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
}

impl HttpMethod {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "DELETE" => Some(Self::Delete),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateScope {
    Ip,
    Account,
    Connection,
}

impl RateScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ip => "ip",
            Self::Account => "account",
            Self::Connection => "connection",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointUseCase {
    HotPathFallback,
    ColdStart,
    Baseline,
    History,
    Metadata,
    Calibration,
    PrivateRead,
    TradeWrite,
}

impl EndpointUseCase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HotPathFallback => "hot_path_fallback",
            Self::ColdStart => "cold_start",
            Self::Baseline => "baseline",
            Self::History => "history",
            Self::Metadata => "metadata",
            Self::Calibration => "calibration",
            Self::PrivateRead => "private_read",
            Self::TradeWrite => "trade_write",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointDataKind {
    OrderBook,
    ServerTime,
    InstrumentMetadata,
    FundingRate,
    FundingPayment,
    MarkIndex,
    OpenInterest,
    PerpTicker,
    SpotTicker,
    OrderAck,
    OrderStatus,
    TradeFill,
    AccountConfig,
    AccountFeeRate,
    AccountBalance,
    AccountPosition,
}

impl EndpointDataKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OrderBook => "order_book",
            Self::ServerTime => "server_time",
            Self::InstrumentMetadata => "instrument_metadata",
            Self::FundingRate => "funding_rate",
            Self::FundingPayment => "funding_payment",
            Self::MarkIndex => "mark_index",
            Self::OpenInterest => "open_interest",
            Self::PerpTicker => "perp_ticker",
            Self::SpotTicker => "spot_ticker",
            Self::OrderAck => "order_ack",
            Self::OrderStatus => "order_status",
            Self::TradeFill => "trade_fill",
            Self::AccountConfig => "account_config",
            Self::AccountFeeRate => "account_fee_rate",
            Self::AccountBalance => "account_balance",
            Self::AccountPosition => "account_position",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointSpec {
    pub venue: VenueId,
    pub method: HttpMethod,
    pub path: &'static str,
    pub doc_url: &'static str,
    pub weight: u32,
    pub rate_scope: RateScope,
    pub use_case: EndpointUseCase,
    pub data_kind: EndpointDataKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointEvidenceSnapshot {
    pub method: String,
    pub path: String,
    pub checked_at: String,
    pub doc_version: String,
    pub schema_hash: String,
    pub fixture_id: String,
    pub parser_test: String,
    pub request_builder_test: String,
    pub auth_kind: String,
    pub doc_urls: Vec<String>,
    pub use_cases: Vec<String>,
    pub data_kinds: Vec<String>,
    pub rate_scopes: Vec<String>,
    pub weight: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EndpointEvidenceMeta {
    checked_at: &'static str,
    doc_version: &'static str,
    schema_hash: &'static str,
    fixture_id: &'static str,
    parser_test: &'static str,
    request_builder_test: &'static str,
    auth_kind: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EndpointEvidenceEntry {
    venue: VenueId,
    method: HttpMethod,
    path: &'static str,
    use_case: EndpointUseCase,
    data_kind: EndpointDataKind,
    meta: EndpointEvidenceMeta,
}

impl EndpointEvidenceEntry {
    fn matches(self, spec: &EndpointSpec) -> bool {
        self.venue == spec.venue
            && self.method == spec.method
            && self.path == spec.path
            && self.use_case == spec.use_case
            && self.data_kind == spec.data_kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HyperliquidOperationTransport {
    Info,
    WebSocket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HyperliquidDexScope {
    OptionalPerpDex,
    Spot,
    AllPerpDexes,
    NotDexScoped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HyperliquidOperationEvidence {
    transport: HyperliquidOperationTransport,
    operation: &'static str,
    dex_scope: HyperliquidDexScope,
    doc_url: &'static str,
    use_case: EndpointUseCase,
    data_kind: EndpointDataKind,
    meta: EndpointEvidenceMeta,
}

impl HyperliquidOperationEvidence {
    fn matches_endpoint_spec(self, spec: &EndpointSpec) -> bool {
        self.transport == HyperliquidOperationTransport::Info
            && spec.venue == VenueId::Hyperliquid
            && spec.method == HttpMethod::Post
            && spec.path == "/info"
            && spec.doc_url == self.doc_url
            && spec.use_case == self.use_case
            && spec.data_kind == self.data_kind
    }
}

const BINANCE_USDM_DEPTH_CHECKED_AT: &str = "2026-06-03";
const BINANCE_USDM_DEPTH_DOC_VERSION: &str = "binance-usdm-futures-order-book-2026-06-03";
const BINANCE_USDM_DEPTH_SCHEMA_HASH: &str =
    "sha256:96d4fd6d973f203dfeb60ed85dbcd8cac6b56096c06f3bc698911b81c3069815";
const BINANCE_USDM_DEPTH_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_order_book_depth_btcusdt.json";
const BINANCE_USDM_DEPTH_TEST: &str = "orderbook_parses_official_fixture_levels";
const BINANCE_USDM_SERVER_TIME_CHECKED_AT: &str = "2026-06-03";
const BINANCE_USDM_SERVER_TIME_DOC_VERSION: &str =
    "binance-usdm-futures-check-server-time-2026-06-03";
const BINANCE_USDM_SERVER_TIME_SCHEMA_HASH: &str =
    "sha256:ab3c03f5f73ddb47c0f7c3e29ab661e2eefb54d64077a56e23bf1da6b8ae53c5";
const BINANCE_USDM_SERVER_TIME_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_server_time.json";
const BINANCE_USDM_SERVER_TIME_TEST: &str = "server_time_parses_official_fixture_and_uses_no_query";
const BINANCE_USDM_EXCHANGE_INFO_CHECKED_AT: &str = "2026-07-11";
const BINANCE_USDM_EXCHANGE_INFO_DOC_VERSION: &str =
    "binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11";
const BINANCE_USDM_EXCHANGE_INFO_SCHEMA_HASH: &str =
    "sha256:10c40d2f5bce94b4f4dcf57226e90fd559f5f63dc79fc7867b6b9866f59a9525";
const BINANCE_USDM_EXCHANGE_INFO_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_exchange_info_usdt_usdc.json";
const BINANCE_USDM_EXCHANGE_INFO_PARSER_TEST: &str =
    "registry_projection_uses_compiled_usdt_and_usdc_specs";
const BINANCE_USDM_EXCHANGE_INFO_REQUEST_TEST: &str =
    "exchange_info_uses_official_path_without_query";
const BINANCE_USDM_COMMISSION_RATE_CHECKED_AT: &str = "2026-07-11";
const BINANCE_USDM_COMMISSION_RATE_DOC_VERSION: &str =
    "binance-usdm-futures-account-commission-rate-2026-07-11";
const BINANCE_USDM_COMMISSION_RATE_SCHEMA_HASH: &str =
    "sha256:28c1d83fde43698bca438bdb828eda321af35765923a6ea049863f0bc3cb518f";
const BINANCE_USDM_COMMISSION_RATE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_account_commission_rate_btcusdt.json";
const BINANCE_USDM_COMMISSION_RATE_PARSER_TEST: &str =
    "parses_official_commission_fixture_without_zeroing_rates";
const BINANCE_USDM_COMMISSION_RATE_REQUEST_TEST: &str =
    "commission_rate_signed_request_uses_official_path";
const BINANCE_SPOT_TICKER_24HR_CHECKED_AT: &str = "2026-06-03";
const BINANCE_SPOT_TICKER_24HR_DOC_VERSION: &str =
    "binance-spot-24hr-ticker-price-change-statistics-2026-06-03";
const BINANCE_SPOT_TICKER_24HR_SCHEMA_HASH: &str =
    "sha256:1624d6700a9cb106669813e8d9e48fa72550167c6abc46c871db73f63821d8d8";
const BINANCE_SPOT_TICKER_24HR_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/spot_ticker_24hr_full.json";
const BINANCE_SPOT_TICKER_24HR_PARSER_TEST: &str =
    "binance_spot_ticker_24hr_parses_official_fixture";
const BINANCE_SPOT_TICKER_24HR_REQUEST_TEST: &str =
    "binance_spot_ticker_24hr_uses_official_path_without_query";
const BINANCE_USDM_PREMIUM_INDEX_CHECKED_AT: &str = "2026-06-03";
const BINANCE_USDM_PREMIUM_INDEX_DOC_VERSION: &str = "binance-usdm-futures-mark-price-2026-06-03";
const BINANCE_USDM_PREMIUM_INDEX_SCHEMA_HASH: &str =
    "sha256:eb2273bc5de40fea978307b7d5f460b71c906789725b0988af08b71208079deb";
const BINANCE_USDM_PREMIUM_INDEX_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_premium_index_btcusdt.json";
const BINANCE_USDM_PREMIUM_INDEX_PARSER_TEST: &str =
    "premium_index_parses_official_fixture_funding_and_mark_index";
const BINANCE_USDM_PREMIUM_INDEX_REQUEST_TEST: &str =
    "premium_indexes_uses_official_path_without_query";
const BINANCE_USDM_OPEN_INTEREST_CHECKED_AT: &str = "2026-06-03";
const BINANCE_USDM_OPEN_INTEREST_DOC_VERSION: &str =
    "binance-usdm-futures-open-interest-2026-06-03";
const BINANCE_USDM_OPEN_INTEREST_SCHEMA_HASH: &str =
    "sha256:f3e3c005e2a3a88d7723c606293c47ef08c4cded7969687d2c5ae5f7190636fb";
const BINANCE_USDM_OPEN_INTEREST_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_open_interest_btcusdt.json";
const BINANCE_USDM_OPEN_INTEREST_PARSER_TEST: &str = "open_interest_parses_official_fixture";
const BINANCE_USDM_OPEN_INTEREST_REQUEST_TEST: &str =
    "open_interest_uses_official_path_and_symbol_query";
const BINANCE_USDM_TICKER_24HR_CHECKED_AT: &str = "2026-06-03";
const BINANCE_USDM_TICKER_24HR_DOC_VERSION: &str =
    "binance-usdm-futures-24hr-ticker-price-change-statistics-2026-06-03";
const BINANCE_USDM_TICKER_24HR_SCHEMA_HASH: &str =
    "sha256:808109f971aad3eb8fac7ba1773f496ab0eb56af8caae1d8dad95a45728b63e6";
const BINANCE_USDM_TICKER_24HR_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_ticker_24hr_btcusdt.json";
const BINANCE_USDM_TICKER_24HR_PARSER_TEST: &str =
    "binance_usdm_ticker_24hr_parses_official_fixture";
const BINANCE_USDM_TICKER_24HR_REQUEST_TEST: &str =
    "usdm_ticker_24hr_uses_official_path_without_query";
const OKX_PUBLIC_TIME_CHECKED_AT: &str = "2026-06-03";
const OKX_PUBLIC_TIME_DOC_VERSION: &str = "okx-v5-public-get-system-time-2026-06-03";
const OKX_PUBLIC_TIME_SCHEMA_HASH: &str =
    "sha256:cfad0f259e94475017ec3bf7113726b106a4441706d6e76bc966c15b4ff9a5bb";
const OKX_PUBLIC_TIME_FIXTURE_ID: &str = "crates/exchange/fixtures/okx/public_time.json";
const OKX_PUBLIC_TIME_TEST: &str = "public_time_parses_official_fixture_and_uses_no_query";
const OKX_MARKET_BOOKS_CHECKED_AT: &str = "2026-06-03";
const OKX_MARKET_BOOKS_DOC_VERSION: &str = "okx-v5-market-data-get-order-book-2026-06-03";
const OKX_MARKET_BOOKS_SCHEMA_HASH: &str =
    "sha256:c62a9a951d7c2ec8c81799a7018847c71360fab0c4fc43b13316d4bcd4bce677";
const OKX_MARKET_BOOKS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/okx/market_books_btc_usdt_swap.json";
const OKX_MARKET_BOOKS_PARSER_TEST: &str = "okx_orderbook_parses_official_fixture_levels";
const OKX_MARKET_BOOKS_REQUEST_TEST: &str =
    "okx_orderbook_rest_parses_official_fixture_and_uses_inst_id_sz_query";
const OKX_PUBLIC_INSTRUMENTS_CHECKED_AT: &str = "2026-06-03";
const OKX_PUBLIC_INSTRUMENTS_DOC_VERSION: &str = "okx-v5-public-get-instruments-2026-06-03";
const OKX_PUBLIC_INSTRUMENTS_SCHEMA_HASH: &str =
    "sha256:12f958835984aafc2c8280c93e33fbefcadba9351f113275351faf51c9ac76eb";
const OKX_PUBLIC_INSTRUMENTS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/okx/public_instruments_swap.json";
const OKX_PUBLIC_INSTRUMENTS_PARSER_TEST: &str = "okx_instrument_rule_parses_official_swap_fixture";
const OKX_PUBLIC_INSTRUMENTS_REQUEST_TEST: &str =
    "swap_inst_ids_rest_parses_official_fixture_and_uses_swap_query";
const OKX_MARKET_TICKERS_CHECKED_AT: &str = "2026-06-03";
const OKX_MARKET_TICKERS_DOC_VERSION: &str = "okx-v5-market-data-get-tickers-2026-06-03";
const OKX_MARKET_TICKERS_SCHEMA_HASH: &str =
    "sha256:ce2386f39d6d4284a467f5b66a689290d7f5c8f9d8c68058eff5e43f49621945";
const OKX_MARKET_TICKERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/okx/market_tickers_swap_spot_btc_eth_usdt.json";
const OKX_MARKET_TICKERS_PARSER_TEST: &str =
    "okx_market_tickers_parse_official_swap_and_spot_fixture";
const OKX_MARKET_TICKERS_REQUEST_TEST: &str = "okx_market_tickers_use_inst_type_queries";
const OKX_FUNDING_RATE_CHECKED_AT: &str = "2026-06-03";
const OKX_FUNDING_RATE_DOC_VERSION: &str = "okx-v5-public-data-get-funding-rate-2026-06-03";
const OKX_PUBLIC_MARK_INDEX_CHECKED_AT: &str = "2026-06-03";
const OKX_MARK_PRICE_DOC_VERSION: &str = "okx-v5-public-data-get-mark-price-2026-06-03";
const OKX_INDEX_TICKERS_DOC_VERSION: &str = "okx-v5-public-data-get-index-tickers-2026-06-03";
const OKX_OPEN_INTEREST_DOC_VERSION: &str = "okx-v5-public-data-get-open-interest-2026-06-03";
const OKX_PUBLIC_BASELINE_SCHEMA_HASH: &str =
    "sha256:ae1dba76dc86eb171ad0311d79e0461b51503230be3bd6f0d4969078b0c2dd7f";
const OKX_PUBLIC_BASELINE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/okx/public_funding_mark_index_oi_btc_eth_usdt.json";
const OKX_FUNDING_RATE_PARSER_TEST: &str = "okx_funding_rate_parses_official_fixture_interval";
const OKX_FUNDING_RATE_REQUEST_TEST: &str = "okx_funding_rate_uses_official_path_and_inst_id_query";
const OKX_MARK_INDEX_PARSER_TEST: &str = "okx_mark_index_open_interest_parse_official_fixture";
const OKX_MARK_INDEX_REQUEST_TEST: &str = "okx_mark_index_prices_use_official_public_queries";
const BYBIT_SERVER_TIME_CHECKED_AT: &str = "2026-06-03";
const BYBIT_SERVER_TIME_DOC_VERSION: &str = "bybit-v5-get-server-time-2026-06-03";
const BYBIT_SERVER_TIME_SCHEMA_HASH: &str =
    "sha256:1d29f70422c2ba49f8376edf961697bba0d9b60404c175f64e26e4f44c2ac34c";
const BYBIT_SERVER_TIME_FIXTURE_ID: &str = "crates/exchange/fixtures/bybit/server_time.json";
const BYBIT_SERVER_TIME_TEST: &str = "bybit_server_time_parses_official_fixture_and_uses_no_query";
const BYBIT_ORDERBOOK_CHECKED_AT: &str = "2026-06-03";
const BYBIT_ORDERBOOK_DOC_VERSION: &str = "bybit-v5-get-orderbook-2026-06-03";
const BYBIT_ORDERBOOK_SCHEMA_HASH: &str =
    "sha256:28590d90c8a573426e7e66d849afe931815ed8a19ce2855deee9519c314d86b3";
const BYBIT_ORDERBOOK_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bybit/orderbook_linear_btcusdt.json";
const BYBIT_ORDERBOOK_PARSER_TEST: &str = "bybit_orderbook_parses_official_linear_fixture";
const BYBIT_ORDERBOOK_REQUEST_TEST: &str = "bybit_orderbook_rest_uses_linear_symbol_limit_query";
const BYBIT_INSTRUMENTS_CHECKED_AT: &str = "2026-07-13";
const BYBIT_INSTRUMENTS_DOC_VERSION: &str = "bybit-v5-get-instruments-info-2026-07-13";
const BYBIT_INSTRUMENTS_SCHEMA_HASH: &str =
    "sha256:5bd6c38c7d43fd01780a8066901ff0ff548b02c25cd263f57b61713da4b6f0f3";
const BYBIT_INSTRUMENTS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bybit/instruments_info_linear_identity_matrix.json";
const BYBIT_INSTRUMENTS_PARSER_TEST: &str =
    "bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts";
const BYBIT_INSTRUMENTS_REQUEST_TEST: &str =
    "funding_rates_and_instruments_uses_linear_instruments_query";
const BYBIT_MARKET_TICKERS_CHECKED_AT: &str = "2026-06-03";
const BYBIT_MARKET_TICKERS_DOC_VERSION: &str = "bybit-v5-get-tickers-2026-06-03";
const BYBIT_MARKET_TICKERS_SCHEMA_HASH: &str =
    "sha256:f970f706b977866e13fe68a080a7f860ddb9911e5adc7f624cbd454b915ee7f1";
const BYBIT_MARKET_TICKERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bybit/market_tickers_linear_spot_btcusdt.json";
const BYBIT_MARKET_TICKERS_PARSER_TEST: &str =
    "bybit_market_tickers_official_fixture_closes_perp_funding_spot_debt";
const BYBIT_MARKET_TICKERS_REQUEST_TEST: &str = "bybit_market_tickers_uses_linear_and_spot_queries";
const BITGET_SERVER_TIME_CHECKED_AT: &str = "2026-06-03";
const BITGET_SERVER_TIME_DOC_VERSION: &str = "bitget-common-get-server-time-2026-06-03";
const BITGET_SERVER_TIME_SCHEMA_HASH: &str =
    "sha256:6bf033a6f551b96c422f46cf6f176c1a5219bd8e1df0258e0efd8533fb6c39ef";
const BITGET_SERVER_TIME_FIXTURE_ID: &str = "crates/exchange/fixtures/bitget/server_time.json";
const BITGET_SERVER_TIME_TEST: &str =
    "bitget_server_time_parses_official_fixture_and_uses_no_query";
const BITGET_UTA_ORDERBOOK_CHECKED_AT: &str = "2026-06-03";
const BITGET_UTA_ORDERBOOK_DOC_VERSION: &str = "bitget-uta-get-orderbook-2026-06-03";
const BITGET_UTA_ORDERBOOK_SCHEMA_HASH: &str =
    "sha256:acd772c2dcf8a9e427cdeedc8622fa2db0123cb0c7fd9ac9bf3865aecb08c944";
const BITGET_UTA_ORDERBOOK_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_orderbook_usdt_futures_btcusdt.json";
const BITGET_UTA_ORDERBOOK_PARSER_TEST: &str =
    "bitget_uta_orderbook_parses_official_fixture_levels";
const BITGET_UTA_ORDERBOOK_REQUEST_TEST: &str =
    "bitget_uta_orderbook_uses_official_path_category_symbol_limit_query";
const BITGET_UTA_INSTRUMENTS_CHECKED_AT: &str = "2026-06-03";
const BITGET_UTA_INSTRUMENTS_DOC_VERSION: &str = "bitget-uta-get-instruments-2026-06-03";
const BITGET_UTA_INSTRUMENTS_SCHEMA_HASH: &str =
    "sha256:7df78c9aa6e3fee915145a180e251761f831c4affcab637d0323b4b853eb5a4d";
const BITGET_UTA_INSTRUMENTS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_instruments_usdt_futures_btcusdt.json";
const BITGET_UTA_INSTRUMENTS_PARSER_TEST: &str =
    "bitget_uta_instruments_parses_official_fixture_metadata";
const BITGET_UTA_INSTRUMENTS_REQUEST_TEST: &str =
    "bitget_uta_instruments_use_official_path_category_query";
const BITGET_UTA_CURRENT_FUNDING_CHECKED_AT: &str = "2026-06-03";
const BITGET_UTA_CURRENT_FUNDING_DOC_VERSION: &str =
    "bitget-uta-get-current-funding-rate-2026-06-03";
const BITGET_UTA_CURRENT_FUNDING_SCHEMA_HASH: &str =
    "sha256:806a1cb7d237884b4144caa40e32409ff42e642ff5aedb3171378825f1765cd7";
const BITGET_UTA_CURRENT_FUNDING_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_current_fund_rate_btcusdt.json";
const BITGET_UTA_CURRENT_FUNDING_PARSER_TEST: &str =
    "bitget_uta_current_funding_parses_official_fixture";
const BITGET_UTA_CURRENT_FUNDING_REQUEST_TEST: &str =
    "bitget_uta_current_funding_uses_official_symbol_query";
const BITGET_UTA_TICKERS_CHECKED_AT: &str = "2026-06-03";
const BITGET_UTA_TICKERS_DOC_VERSION: &str = "bitget-uta-get-tickers-2026-06-03";
const BITGET_UTA_TICKERS_SCHEMA_HASH: &str =
    "sha256:001a60dcbff93a4423c05ed344eb434e8db9f87ee4213f9a981cf6e00355d3c0";
const BITGET_UTA_TICKERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_tickers_usdt_futures_spot_btcusdt.json";
const BITGET_UTA_TICKERS_PARSER_TEST: &str =
    "bitget_uta_tickers_parse_official_futures_and_spot_fixture";
const BITGET_UTA_TICKERS_REQUEST_TEST: &str =
    "bitget_uta_tickers_use_official_path_category_queries";
const HYPERLIQUID_L2BOOK_CHECKED_AT: &str = "2026-06-03";
const HYPERLIQUID_L2BOOK_DOC_VERSION: &str = "hyperliquid-info-l2book-2026-06-03";
const HYPERLIQUID_L2BOOK_SCHEMA_HASH: &str =
    "sha256:ca8d9ead4d832f01a30897dbf6d4ff11c3816c17ecddd0f36491b9d40ef4317e";
const HYPERLIQUID_L2BOOK_FIXTURE_ID: &str = "crates/exchange/fixtures/hyperliquid/l2book_btc.json";
const HYPERLIQUID_L2BOOK_PARSER_TEST: &str = "hyperliquid_l2book_parses_official_fixture_levels";
const HYPERLIQUID_L2BOOK_REQUEST_TEST: &str = "get_orderbook_l2book";
const HYPERLIQUID_META_CTXS_CHECKED_AT: &str = "2026-06-03";
const HYPERLIQUID_META_CTXS_DOC_VERSION: &str =
    "hyperliquid-perpetuals-meta-and-asset-ctxs-2026-06-03";
const HYPERLIQUID_META_CTXS_SCHEMA_HASH: &str =
    "sha256:b5b76216f848ed73eb687ad9610d29e90a93234b07dd7193c1ae9d94c65891b4";
const HYPERLIQUID_META_CTXS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/meta_and_asset_ctxs_btc_eth.json";
const HYPERLIQUID_META_CTXS_PARSER_TEST: &str =
    "hyperliquid_meta_and_asset_ctxs_parses_official_fixture_metadata_ticker_funding";
const HYPERLIQUID_META_CTXS_REQUEST_TEST: &str =
    "hyperliquid_meta_body_uses_official_body_with_optional_dex";
const HYPERLIQUID_TICKERS_REQUEST_TEST: &str =
    "hyperliquid_get_tickers_requests_meta_and_asset_ctxs";
const HYPERLIQUID_SPOT_META_CTXS_CHECKED_AT: &str = "2026-06-03";
const HYPERLIQUID_SPOT_META_CTXS_DOC_VERSION: &str =
    "hyperliquid-spot-meta-and-asset-ctxs-2026-06-03";
const HYPERLIQUID_SPOT_META_CTXS_SCHEMA_HASH: &str =
    "sha256:4342019a816e691b3e14385f4cd61fd32c47ac61ad741e62c6d8c2f975de9a4b";
const HYPERLIQUID_SPOT_META_CTXS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json";
const HYPERLIQUID_SPOT_META_CTXS_PARSER_TEST: &str =
    "hyperliquid_spot_meta_and_asset_ctxs_parses_official_fixture";
const HYPERLIQUID_SPOT_TICKERS_REQUEST_TEST: &str =
    "hyperliquid_get_spot_tickers_requests_spot_meta_and_asset_ctxs";
const HYPERLIQUID_PREDICTED_FUNDINGS_CHECKED_AT: &str = "2026-06-03";
const HYPERLIQUID_PREDICTED_FUNDINGS_DOC_VERSION: &str =
    "hyperliquid-perpetuals-predicted-fundings-2026-06-03";
const HYPERLIQUID_PREDICTED_FUNDINGS_SCHEMA_HASH: &str =
    "sha256:bf35a0cdd8c6f65c7dab0d86571a2cdbe59540e0f5472bea5cd476d714806031";
const HYPERLIQUID_PREDICTED_FUNDINGS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/predicted_fundings_avax.json";
const HYPERLIQUID_PREDICTED_FUNDINGS_PARSER_TEST: &str =
    "hyperliquid_predicted_fundings_parses_official_fixture_hl_perp";
const HYPERLIQUID_PREDICTED_FUNDINGS_REQUEST_TEST: &str =
    "hyperliquid_get_funding_rates_requests_meta_and_predicted_fundings";
const HYPERLIQUID_PLACE_ORDER_CHECKED_AT: &str = "2026-06-30";
const HYPERLIQUID_PLACE_ORDER_DOC_VERSION: &str = "hyperliquid-exchange-order-action-2026-06-30";
const HYPERLIQUID_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:23d821d90ef17f9139028cfe0bc0f9247b992089b3022a9a10b6159608ed3cbf";
const HYPERLIQUID_PLACE_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/exchange_order_resting.json";
const HYPERLIQUID_PLACE_ORDER_PARSER_TEST: &str =
    "hyperliquid_place_order_ack_parses_official_fixture";
const HYPERLIQUID_PLACE_ORDER_REQUEST_TEST: &str =
    "order_action_matches_hyperliquid_compact_schema";
const HYPERLIQUID_GET_ORDER_CHECKED_AT: &str = "2026-06-30";
const HYPERLIQUID_GET_ORDER_DOC_VERSION: &str = "hyperliquid-info-order-status-2026-06-30";
const HYPERLIQUID_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:875f9a4b3f868c1936d4ee274e3183001a9200521c39dc9067034b1d07a6a19a";
const HYPERLIQUID_GET_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/info_order_status_filled.json";
const HYPERLIQUID_GET_ORDER_PARSER_TEST: &str =
    "order_status_official_envelope_parses_filled_order";
const HYPERLIQUID_GET_ORDER_REQUEST_TEST: &str =
    "get_order_derives_public_client_id_to_official_cloid";
const HYPERLIQUID_ACCOUNT_BALANCE_CHECKED_AT: &str = "2026-07-02";
const HYPERLIQUID_ACCOUNT_BALANCE_DOC_VERSION: &str =
    "hyperliquid-info-clearinghouse-and-spot-clearinghouse-state-2026-07-02";
const HYPERLIQUID_ACCOUNT_BALANCE_SCHEMA_HASH: &str =
    "sha256:ad29e1740237966f26cf76d9eddc87224edcc4618d294ea46e706716192ee0f0";
const HYPERLIQUID_ACCOUNT_BALANCE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/info_clearinghouse_state_account_balance.json";
const HYPERLIQUID_ACCOUNT_BALANCE_PARSER_TEST: &str =
    "hyperliquid_account_balance_parses_official_fixtures";
const HYPERLIQUID_ACCOUNT_POSITION_CHECKED_AT: &str = "2026-07-02";
const HYPERLIQUID_ACCOUNT_POSITION_DOC_VERSION: &str =
    "hyperliquid-info-clearinghouse-state-account-position-2026-07-02";
const HYPERLIQUID_ACCOUNT_POSITION_PARSER_TEST: &str =
    "hyperliquid_account_position_parses_official_fixture";
const HYPERLIQUID_INFO_META_AND_ASSET_CTXS_DOC_URL: &str =
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-perpetuals-asset-contexts-includes-mark-price-current-funding-open-interest-etc";
const HYPERLIQUID_INFO_OPEN_ORDERS_DOC_URL: &str =
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#retrieve-a-users-open-orders";
const HYPERLIQUID_INFO_FRONTEND_OPEN_ORDERS_DOC_URL: &str =
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#retrieve-a-users-open-orders-with-additional-frontend-info";
const HYPERLIQUID_INFO_ORDER_STATUS_DOC_URL: &str =
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#query-order-status-by-oid-or-cloid";
const HYPERLIQUID_INFO_CLEARINGHOUSE_STATE_DOC_URL: &str =
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-users-perpetuals-account-summary";
const HYPERLIQUID_INFO_SPOT_CLEARINGHOUSE_STATE_DOC_URL: &str =
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/spot#retrieve-a-users-token-balances";
const HYPERLIQUID_WS_SUBSCRIPTIONS_DOC_URL: &str =
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions";
const HYPERLIQUID_INFO_OPERATION_CHECKED_AT: &str = "2026-07-12";
const HYPERLIQUID_INFO_OPERATION_REQUEST_TEST: &str =
    "hyperliquid_info_operation_request_contracts_are_distinct";
const HYPERLIQUID_OPERATION_FIXTURE_PARSER_TEST: &str =
    "hyperliquid_operation_fixtures_parse_and_keep_dex_scope";
const HYPERLIQUID_WS_OPERATION_REQUEST_TEST: &str =
    "hyperliquid_ws_operation_request_contracts_remain_ws_only";
const HYPERLIQUID_OPEN_ORDERS_DOC_VERSION: &str = "hyperliquid-info-open-orders-dex-2026-07-12";
const HYPERLIQUID_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:0bb032ff7d509fcdb288c596bcb1f2a186ad116b736a8d2badd8a8023c829273";
const HYPERLIQUID_OPEN_ORDERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/info_open_orders_dex.json";
const HYPERLIQUID_FRONTEND_OPEN_ORDERS_DOC_VERSION: &str =
    "hyperliquid-info-frontend-open-orders-dex-2026-07-12";
const HYPERLIQUID_FRONTEND_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:82f8ddebbfd3b17b9164e6709e487aefc1048341619f8d74a740fda82054db7d";
const HYPERLIQUID_FRONTEND_OPEN_ORDERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/info_frontend_open_orders_dex.json";
const HYPERLIQUID_SPOT_CLEARINGHOUSE_STATE_DOC_VERSION: &str =
    "hyperliquid-info-spot-clearinghouse-state-2026-07-12";
const HYPERLIQUID_SPOT_CLEARINGHOUSE_STATE_SCHEMA_HASH: &str =
    "sha256:c55e0f86b418e05204bf91e0379c82f02fc9fc7c72aea89b54fc24fd86188d6d";
const HYPERLIQUID_SPOT_CLEARINGHOUSE_STATE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/info_spot_clearinghouse_state_account_balance.json";
const HYPERLIQUID_WS_ALL_DEXS_ASSET_CTXS_DOC_VERSION: &str =
    "hyperliquid-ws-all-dexs-asset-ctxs-2026-07-12";
const HYPERLIQUID_WS_ALL_DEXS_ASSET_CTXS_SCHEMA_HASH: &str =
    "sha256:d6201da58fe964960fa8b123e99c44051b57cfffaac3cc00282ef2633b30cbbb";
const HYPERLIQUID_WS_ALL_DEXS_ASSET_CTXS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/ws_all_dexs_asset_ctxs_evidence.json";
const HYPERLIQUID_WS_ALL_DEXS_CLEARINGHOUSE_DOC_VERSION: &str =
    "hyperliquid-ws-all-dexs-clearinghouse-state-2026-07-12";
const HYPERLIQUID_WS_ALL_DEXS_CLEARINGHOUSE_SCHEMA_HASH: &str =
    "sha256:6a29c5cdfcb02a17f525add9d529c5e75e4fb1f1be2bb99af35bfa58ae43f6dc";
const HYPERLIQUID_WS_ALL_DEXS_CLEARINGHOUSE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/hyperliquid/ws_all_dexs_clearinghouse_evidence.json";
const GATE_SERVER_TIME_CHECKED_AT: &str = "2026-06-03";
const GATE_SERVER_TIME_DOC_VERSION: &str = "gate-apiv4-get-server-current-time-2026-06-03";
const GATE_SERVER_TIME_SCHEMA_HASH: &str =
    "sha256:3f245cc394c090f3d1f68e1c5afade2d72ffe18d6413c15a0f94650bf15b93f0";
const GATE_SERVER_TIME_FIXTURE_ID: &str = "crates/exchange/fixtures/gate/server_time.json";
const GATE_SERVER_TIME_TEST: &str = "gate_server_time_parses_official_fixture_and_uses_no_query";
const GATE_CONTRACTS_CHECKED_AT: &str = "2026-06-03";
const GATE_CONTRACTS_DOC_VERSION: &str = "gate-apiv4-list-futures-contracts-2026-06-03";
const GATE_CONTRACTS_SCHEMA_HASH: &str =
    "sha256:8d34ecc69829afdec05cad79d47cd69c8afab63a52a274c26d77cd50ce025d4f";
const GATE_CONTRACTS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/futures_usdt_contracts_btc_usdt.json";
const GATE_CONTRACTS_PARSER_TEST: &str =
    "gate_contracts_parses_official_fixture_metadata_and_funding";
const GATE_CONTRACTS_REQUEST_TEST: &str =
    "gate_contracts_and_tickers_uses_official_contracts_path_without_query";
const GATE_ORDERBOOK_CHECKED_AT: &str = "2026-06-03";
const GATE_ORDERBOOK_DOC_VERSION: &str =
    "gate-apiv4-query-futures-market-depth-information-2026-06-03";
const GATE_ORDERBOOK_SCHEMA_HASH: &str =
    "sha256:6d2d9f450255d4e1c3fe0b5fd3c858fb2d3eb011de1580f6681bcca17299b874";
const GATE_ORDERBOOK_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/futures_usdt_order_book_btc_usdt.json";
const GATE_ORDERBOOK_PARSER_TEST: &str = "gate_futures_orderbook_parses_official_fixture_levels";
const GATE_ORDERBOOK_REQUEST_TEST: &str =
    "gate_futures_orderbook_uses_official_path_contract_and_limit_query";
const GATE_FUTURES_TICKERS_CHECKED_AT: &str = "2026-06-03";
const GATE_FUTURES_TICKERS_DOC_VERSION: &str =
    "gate-apiv4-get-all-futures-trading-statistics-2026-06-03";
const GATE_FUTURES_TICKERS_SCHEMA_HASH: &str =
    "sha256:5107efe33cfd9f0939d1854a5f8c858145ce66153610ad6acd85e969a71a3774";
const GATE_FUTURES_TICKERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/futures_usdt_tickers_btc_eth_usdt.json";
const GATE_FUTURES_TICKERS_PARSER_TEST: &str =
    "gate_futures_tickers_parse_official_fixture_prices_and_mark";
const GATE_FUTURES_TICKERS_REQUEST_TEST: &str =
    "gate_futures_tickers_uses_official_path_without_query";
const GATE_SPOT_TICKERS_CHECKED_AT: &str = "2026-06-03";
const GATE_SPOT_TICKERS_DOC_VERSION: &str = "gate-apiv4-list-spot-tickers-2026-06-03";
const GATE_SPOT_TICKERS_SCHEMA_HASH: &str =
    "sha256:d42985986293d2a2ed6e2f291ff34b8510c83eab4a9105c57055cc6ac89e878a";
const GATE_SPOT_TICKERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/spot_tickers_btc_eth_usdt.json";
const GATE_SPOT_TICKERS_PARSER_TEST: &str =
    "gate_spot_tickers_parse_official_fixture_keeps_missing_sizes_none";
const GATE_SPOT_TICKERS_REQUEST_TEST: &str = "gate_spot_tickers_uses_official_path_without_query";
const HTX_SERVER_TIME_CHECKED_AT: &str = "2026-06-03";
const HTX_SERVER_TIME_DOC_VERSION: &str = "htx-usdt-swap-get-current-system-timestamp-2026-06-03";
const HTX_SERVER_TIME_SCHEMA_HASH: &str =
    "sha256:3feb0873bf921e89c6a39233685eb15daa5317265f08743e47610af679a44bdb";
const HTX_SERVER_TIME_FIXTURE_ID: &str = "crates/exchange/fixtures/htx/server_time.json";
const HTX_SERVER_TIME_TEST: &str = "htx_server_time_parses_official_fixture_and_uses_no_query";
const HTX_MARKET_DEPTH_CHECKED_AT: &str = "2026-06-03";
const HTX_MARKET_DEPTH_DOC_VERSION: &str = "htx-usdt-swap-get-market-depth-2026-06-03";
const HTX_MARKET_DEPTH_SCHEMA_HASH: &str =
    "sha256:fb9c7e5ee5d9665cd3957e9fff1e6283aabf8a79c202165c37b49af57acefebe";
const HTX_MARKET_DEPTH_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/market_depth_btc_usdt_step6.json";
const HTX_MARKET_DEPTH_PARSER_TEST: &str = "htx_market_depth_parses_official_fixture_levels";
const HTX_MARKET_DEPTH_REQUEST_TEST: &str =
    "htx_orderbook_uses_official_path_contract_code_and_step6_query";
const HTX_CONTRACT_INFO_CHECKED_AT: &str = "2026-06-03";
const HTX_CONTRACT_INFO_DOC_VERSION: &str = "htx-usdt-swap-query-swap-info-2026-06-03";
const HTX_CONTRACT_INFO_SCHEMA_HASH: &str =
    "sha256:cc5917216224ae43ae2acad67741b3f35b208c2f28e57f0f7171868223d775a8";
const HTX_CONTRACT_INFO_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_contract_info_btc_usdt.json";
const HTX_CONTRACT_INFO_PARSER_TEST: &str = "htx_instrument_rule_parses_official_contract_fixture";
const HTX_CONTRACT_INFO_REQUEST_TEST: &str =
    "htx_swap_contract_info_uses_official_path_without_query";
const HTX_BATCH_FUNDING_CHECKED_AT: &str = "2026-06-03";
const HTX_BATCH_FUNDING_DOC_VERSION: &str = "htx-usdt-swap-query-batch-funding-rate-2026-06-03";
const HTX_BATCH_FUNDING_SCHEMA_HASH: &str =
    "sha256:c55ac941fd8314e45dcefb30884a90ac18bd973211982014002751a8e0e7612e";
const HTX_BATCH_FUNDING_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_batch_funding_rate.json";
const HTX_BATCH_FUNDING_PARSER_TEST: &str = "htx_batch_funding_rate_parses_official_fixture";
const HTX_BATCH_FUNDING_REQUEST_TEST: &str =
    "htx_batch_funding_rate_and_tickers_use_official_paths_without_query";
const HTX_MARKET_DETAIL_MERGED_CHECKED_AT: &str = "2026-06-03";
const HTX_MARKET_DETAIL_MERGED_DOC_VERSION: &str =
    "htx-usdt-swap-get-market-data-overview-2026-06-03";
const HTX_MARKET_DETAIL_MERGED_SCHEMA_HASH: &str =
    "sha256:f176aabc4f8f323d6495466c6543abfe882cef8a0dd39ff55badae41cff67ed8";
const HTX_MARKET_DETAIL_MERGED_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/market_detail_merged_btc_usdt.json";
const HTX_MARKET_DETAIL_MERGED_PARSER_TEST: &str =
    "htx_market_detail_merged_parses_official_fixture";
const HTX_MARKET_DETAIL_MERGED_REQUEST_TEST: &str =
    "htx_market_detail_merged_uses_official_path_and_contract_code_query";
const HTX_MARK_PRICE_KLINE_CHECKED_AT: &str = "2026-06-03";
const HTX_MARK_PRICE_KLINE_DOC_VERSION: &str =
    "htx-usdt-swap-get-kline-data-of-mark-price-2026-06-03";
const HTX_MARK_PRICE_KLINE_SCHEMA_HASH: &str =
    "sha256:959fc117a2c962d8d4c1d8445f5b223beeb6a9e3e179b2cf19f63117db67e7d1";
const HTX_MARK_PRICE_KLINE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/mark_price_kline_btc_usdt.json";
const HTX_MARK_PRICE_KLINE_PARSER_TEST: &str = "htx_mark_price_kline_parses_official_fixture";
const HTX_MARK_PRICE_KLINE_REQUEST_TEST: &str = "htx_mark_price_kline_uses_official_path_and_query";
const HTX_SWAP_INDEX_CHECKED_AT: &str = "2026-06-03";
const HTX_SWAP_INDEX_DOC_VERSION: &str =
    "htx-usdt-swap-query-swap-index-price-information-2026-06-03";
const HTX_SWAP_INDEX_SCHEMA_HASH: &str =
    "sha256:6f6f1675fd147acde3797c6f8fbb583041314df1b9ed433dc7f05d950e4a5021";
const HTX_SWAP_INDEX_FIXTURE_ID: &str = "crates/exchange/fixtures/htx/swap_index_btc_usdt.json";
const HTX_SWAP_INDEX_PARSER_TEST: &str = "htx_swap_index_parses_official_fixture";
const HTX_SWAP_INDEX_REQUEST_TEST: &str = "htx_swap_index_uses_official_path_without_query";
const HTX_OPEN_INTEREST_CHECKED_AT: &str = "2026-06-03";
const HTX_OPEN_INTEREST_DOC_VERSION: &str =
    "htx-usdt-swap-get-swap-open-interest-information-2026-06-03";
const HTX_OPEN_INTEREST_SCHEMA_HASH: &str =
    "sha256:9eb08bcee6fa4a8d182ad4be0deabc9c6acecd45a71bcc2726254d380ad093f7";
const HTX_OPEN_INTEREST_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_open_interest_btc_usdt.json";
const HTX_OPEN_INTEREST_PARSER_TEST: &str = "htx_swap_open_interest_parses_official_fixture";
const HTX_OPEN_INTEREST_REQUEST_TEST: &str =
    "htx_swap_open_interest_uses_official_path_without_query";
const HTX_SPOT_MARKET_TICKERS_CHECKED_AT: &str = "2026-06-03";
const HTX_SPOT_MARKET_TICKERS_DOC_VERSION: &str = "htx-spot-get-market-tickers-2026-06-03";
const HTX_SPOT_MARKET_TICKERS_SCHEMA_HASH: &str =
    "sha256:36c913011da0a001d3398d21d2fa5dfae5bbda43806e3ee226dd1c8b976752b2";
const HTX_SPOT_MARKET_TICKERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/spot_market_tickers.json";
const HTX_SPOT_MARKET_TICKERS_PARSER_TEST: &str = "htx_spot_market_tickers_parses_official_fixture";
const HTX_SPOT_MARKET_TICKERS_REQUEST_TEST: &str =
    "htx_spot_market_tickers_use_official_path_without_query";
const KUCOIN_SERVER_TIME_CHECKED_AT: &str = "2026-06-03";
const KUCOIN_SERVER_TIME_DOC_VERSION: &str = "kucoin-futures-get-server-time-2026-06-03";
const KUCOIN_SERVER_TIME_SCHEMA_HASH: &str =
    "sha256:ad228065d5e1bee6a4bfd7d6da8d4fc72d6e5ed057ea5adc2069605332c543d2";
const KUCOIN_SERVER_TIME_FIXTURE_ID: &str = "crates/exchange/fixtures/kucoin/server_time.json";
const KUCOIN_SERVER_TIME_TEST: &str =
    "kucoin_server_time_parses_official_fixture_and_uses_no_query";
const KUCOIN_DEPTH20_CHECKED_AT: &str = "2026-06-03";
const KUCOIN_DEPTH20_DOC_VERSION: &str = "kucoin-futures-get-part-orderbook-2026-06-03";
const KUCOIN_DEPTH20_SCHEMA_HASH: &str =
    "sha256:7a25800ba89748c3942baf6df567118e5a15e563342aa14dfd220b856d0771ee";
const KUCOIN_DEPTH20_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/futures_depth20_xbtusdtm.json";
const KUCOIN_DEPTH20_PARSER_TEST: &str = "kucoin_depth20_parses_official_fixture_levels";
const KUCOIN_DEPTH20_REQUEST_TEST: &str = "kucoin_depth20_uses_official_path_and_symbol_query";
const KUCOIN_CONTRACTS_ACTIVE_CHECKED_AT: &str = "2026-06-03";
const KUCOIN_CONTRACTS_ACTIVE_DOC_VERSION: &str = "kucoin-futures-get-all-symbols-2026-06-03";
const KUCOIN_CONTRACTS_ACTIVE_SCHEMA_HASH: &str =
    "sha256:ffd6fe27987d000d61c116dbc848e4f643087f2bad800c60a4a8e8b4b52801a6";
const KUCOIN_CONTRACTS_ACTIVE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/contracts_active_xbt_eth_usdtm.json";
const KUCOIN_CONTRACTS_ACTIVE_PARSER_TEST: &str =
    "contracts_active_fixture_preserves_official_multiplier_and_funding";
const KUCOIN_CONTRACTS_ACTIVE_REQUEST_TEST: &str =
    "kucoin_contracts_active_uses_official_path_without_query";
const KUCOIN_CONTRACTS_NATIVE_CHECKED_AT: &str = "2026-07-11";
const KUCOIN_CONTRACTS_NATIVE_DOC_VERSION: &str =
    "kucoin-futures-native-contract-matrix-2026-07-11";
const KUCOIN_CONTRACTS_NATIVE_SCHEMA_HASH: &str =
    "sha256:612f1509f278052c31888787baae9f3556086bff1280ee430f8e992aff641d7c";
const KUCOIN_CONTRACTS_NATIVE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/contracts_active_native_matrix.json";
const KUCOIN_CONTRACTS_NATIVE_PARSER_TEST: &str =
    "official_matrix_maps_usdt_usdc_and_verified_equity_contracts";
const KUCOIN_FUTURES_ALL_TICKERS_CHECKED_AT: &str = "2026-06-03";
const KUCOIN_FUTURES_ALL_TICKERS_DOC_VERSION: &str = "kucoin-futures-get-all-tickers-2026-06-03";
const KUCOIN_FUTURES_ALL_TICKERS_SCHEMA_HASH: &str =
    "sha256:0ce92253b1be93e2e0554e354917a566b4435954643a1ec6338e203040b18c10";
const KUCOIN_FUTURES_ALL_TICKERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/futures_all_tickers_xbt_eth_usdtm.json";
const KUCOIN_FUTURES_ALL_TICKERS_PARSER_TEST: &str =
    "kucoin_futures_all_tickers_parses_official_fixture_quotes";
const KUCOIN_FUTURES_ALL_TICKERS_REQUEST_TEST: &str =
    "kucoin_futures_all_tickers_uses_official_path_without_query";
const KUCOIN_SPOT_MARKET_ALL_TICKERS_CHECKED_AT: &str = "2026-06-03";
const KUCOIN_SPOT_MARKET_ALL_TICKERS_DOC_VERSION: &str = "kucoin-spot-get-all-tickers-2026-06-03";
const KUCOIN_SPOT_MARKET_ALL_TICKERS_SCHEMA_HASH: &str =
    "sha256:b353998af8a464261543191d29b735e66537c7e50f37df2c50143d3f10d2a36a";
const KUCOIN_SPOT_MARKET_ALL_TICKERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/spot_market_all_tickers_btc_eth_usdt.json";
const KUCOIN_SPOT_MARKET_ALL_TICKERS_PARSER_TEST: &str =
    "kucoin_spot_all_tickers_parses_official_fixture_quotes";
const KUCOIN_SPOT_MARKET_ALL_TICKERS_REQUEST_TEST: &str =
    "kucoin_spot_market_all_tickers_uses_official_path_without_query";
const PUBLIC_AUTH_KIND: &str = "public";
const SIGNED_AUTH_KIND: &str = "signed";
const USER_ADDRESS_AUTH_KIND: &str = "user_address";

const RECORDED_ENDPOINT_EVIDENCE: &[EndpointEvidenceEntry] = &[
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/depth",
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_DEPTH_CHECKED_AT,
            doc_version: BINANCE_USDM_DEPTH_DOC_VERSION,
            schema_hash: BINANCE_USDM_DEPTH_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_DEPTH_FIXTURE_ID,
            parser_test: BINANCE_USDM_DEPTH_TEST,
            request_builder_test: BINANCE_USDM_DEPTH_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/time",
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_SERVER_TIME_CHECKED_AT,
            doc_version: BINANCE_USDM_SERVER_TIME_DOC_VERSION,
            schema_hash: BINANCE_USDM_SERVER_TIME_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_SERVER_TIME_FIXTURE_ID,
            parser_test: BINANCE_USDM_SERVER_TIME_TEST,
            request_builder_test: BINANCE_USDM_SERVER_TIME_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/exchangeInfo",
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_EXCHANGE_INFO_CHECKED_AT,
            doc_version: BINANCE_USDM_EXCHANGE_INFO_DOC_VERSION,
            schema_hash: BINANCE_USDM_EXCHANGE_INFO_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_EXCHANGE_INFO_FIXTURE_ID,
            parser_test: BINANCE_USDM_EXCHANGE_INFO_PARSER_TEST,
            request_builder_test: BINANCE_USDM_EXCHANGE_INFO_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/api/v3/ticker/24hr",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_SPOT_TICKER_24HR_CHECKED_AT,
            doc_version: BINANCE_SPOT_TICKER_24HR_DOC_VERSION,
            schema_hash: BINANCE_SPOT_TICKER_24HR_SCHEMA_HASH,
            fixture_id: BINANCE_SPOT_TICKER_24HR_FIXTURE_ID,
            parser_test: BINANCE_SPOT_TICKER_24HR_PARSER_TEST,
            request_builder_test: BINANCE_SPOT_TICKER_24HR_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/premiumIndex",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_PREMIUM_INDEX_CHECKED_AT,
            doc_version: BINANCE_USDM_PREMIUM_INDEX_DOC_VERSION,
            schema_hash: BINANCE_USDM_PREMIUM_INDEX_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_PREMIUM_INDEX_FIXTURE_ID,
            parser_test: BINANCE_USDM_PREMIUM_INDEX_PARSER_TEST,
            request_builder_test: BINANCE_USDM_PREMIUM_INDEX_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/premiumIndex",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_PREMIUM_INDEX_CHECKED_AT,
            doc_version: BINANCE_USDM_PREMIUM_INDEX_DOC_VERSION,
            schema_hash: BINANCE_USDM_PREMIUM_INDEX_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_PREMIUM_INDEX_FIXTURE_ID,
            parser_test: BINANCE_USDM_PREMIUM_INDEX_PARSER_TEST,
            request_builder_test: BINANCE_USDM_PREMIUM_INDEX_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/openInterest",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::OpenInterest,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_OPEN_INTEREST_CHECKED_AT,
            doc_version: BINANCE_USDM_OPEN_INTEREST_DOC_VERSION,
            schema_hash: BINANCE_USDM_OPEN_INTEREST_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_OPEN_INTEREST_FIXTURE_ID,
            parser_test: BINANCE_USDM_OPEN_INTEREST_PARSER_TEST,
            request_builder_test: BINANCE_USDM_OPEN_INTEREST_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/ticker/24hr",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_TICKER_24HR_CHECKED_AT,
            doc_version: BINANCE_USDM_TICKER_24HR_DOC_VERSION,
            schema_hash: BINANCE_USDM_TICKER_24HR_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_TICKER_24HR_FIXTURE_ID,
            parser_test: BINANCE_USDM_TICKER_24HR_PARSER_TEST,
            request_builder_test: BINANCE_USDM_TICKER_24HR_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/market/books",
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_MARKET_BOOKS_CHECKED_AT,
            doc_version: OKX_MARKET_BOOKS_DOC_VERSION,
            schema_hash: OKX_MARKET_BOOKS_SCHEMA_HASH,
            fixture_id: OKX_MARKET_BOOKS_FIXTURE_ID,
            parser_test: OKX_MARKET_BOOKS_PARSER_TEST,
            request_builder_test: OKX_MARKET_BOOKS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/time",
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_PUBLIC_TIME_CHECKED_AT,
            doc_version: OKX_PUBLIC_TIME_DOC_VERSION,
            schema_hash: OKX_PUBLIC_TIME_SCHEMA_HASH,
            fixture_id: OKX_PUBLIC_TIME_FIXTURE_ID,
            parser_test: OKX_PUBLIC_TIME_TEST,
            request_builder_test: OKX_PUBLIC_TIME_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/instruments",
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_PUBLIC_INSTRUMENTS_CHECKED_AT,
            doc_version: OKX_PUBLIC_INSTRUMENTS_DOC_VERSION,
            schema_hash: OKX_PUBLIC_INSTRUMENTS_SCHEMA_HASH,
            fixture_id: OKX_PUBLIC_INSTRUMENTS_FIXTURE_ID,
            parser_test: OKX_PUBLIC_INSTRUMENTS_PARSER_TEST,
            request_builder_test: OKX_PUBLIC_INSTRUMENTS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/market/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_MARKET_TICKERS_CHECKED_AT,
            doc_version: OKX_MARKET_TICKERS_DOC_VERSION,
            schema_hash: OKX_MARKET_TICKERS_SCHEMA_HASH,
            fixture_id: OKX_MARKET_TICKERS_FIXTURE_ID,
            parser_test: OKX_MARKET_TICKERS_PARSER_TEST,
            request_builder_test: OKX_MARKET_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/market/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_MARKET_TICKERS_CHECKED_AT,
            doc_version: OKX_MARKET_TICKERS_DOC_VERSION,
            schema_hash: OKX_MARKET_TICKERS_SCHEMA_HASH,
            fixture_id: OKX_MARKET_TICKERS_FIXTURE_ID,
            parser_test: OKX_MARKET_TICKERS_PARSER_TEST,
            request_builder_test: OKX_MARKET_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/funding-rate",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_FUNDING_RATE_CHECKED_AT,
            doc_version: OKX_FUNDING_RATE_DOC_VERSION,
            schema_hash: OKX_PUBLIC_BASELINE_SCHEMA_HASH,
            fixture_id: OKX_PUBLIC_BASELINE_FIXTURE_ID,
            parser_test: OKX_FUNDING_RATE_PARSER_TEST,
            request_builder_test: OKX_FUNDING_RATE_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/mark-price",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_PUBLIC_MARK_INDEX_CHECKED_AT,
            doc_version: OKX_MARK_PRICE_DOC_VERSION,
            schema_hash: OKX_PUBLIC_BASELINE_SCHEMA_HASH,
            fixture_id: OKX_PUBLIC_BASELINE_FIXTURE_ID,
            parser_test: OKX_MARK_INDEX_PARSER_TEST,
            request_builder_test: OKX_MARK_INDEX_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/market/index-tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_PUBLIC_MARK_INDEX_CHECKED_AT,
            doc_version: OKX_INDEX_TICKERS_DOC_VERSION,
            schema_hash: OKX_PUBLIC_BASELINE_SCHEMA_HASH,
            fixture_id: OKX_PUBLIC_BASELINE_FIXTURE_ID,
            parser_test: OKX_MARK_INDEX_PARSER_TEST,
            request_builder_test: OKX_MARK_INDEX_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/open-interest",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::OpenInterest,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_PUBLIC_MARK_INDEX_CHECKED_AT,
            doc_version: OKX_OPEN_INTEREST_DOC_VERSION,
            schema_hash: OKX_PUBLIC_BASELINE_SCHEMA_HASH,
            fixture_id: OKX_PUBLIC_BASELINE_FIXTURE_ID,
            parser_test: OKX_MARK_INDEX_PARSER_TEST,
            request_builder_test: OKX_MARK_INDEX_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/orderbook",
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_ORDERBOOK_CHECKED_AT,
            doc_version: BYBIT_ORDERBOOK_DOC_VERSION,
            schema_hash: BYBIT_ORDERBOOK_SCHEMA_HASH,
            fixture_id: BYBIT_ORDERBOOK_FIXTURE_ID,
            parser_test: BYBIT_ORDERBOOK_PARSER_TEST,
            request_builder_test: BYBIT_ORDERBOOK_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/time",
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_SERVER_TIME_CHECKED_AT,
            doc_version: BYBIT_SERVER_TIME_DOC_VERSION,
            schema_hash: BYBIT_SERVER_TIME_SCHEMA_HASH,
            fixture_id: BYBIT_SERVER_TIME_FIXTURE_ID,
            parser_test: BYBIT_SERVER_TIME_TEST,
            request_builder_test: BYBIT_SERVER_TIME_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/instruments-info",
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_INSTRUMENTS_CHECKED_AT,
            doc_version: BYBIT_INSTRUMENTS_DOC_VERSION,
            schema_hash: BYBIT_INSTRUMENTS_SCHEMA_HASH,
            fixture_id: BYBIT_INSTRUMENTS_FIXTURE_ID,
            parser_test: BYBIT_INSTRUMENTS_PARSER_TEST,
            request_builder_test: BYBIT_INSTRUMENTS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_MARKET_TICKERS_CHECKED_AT,
            doc_version: BYBIT_MARKET_TICKERS_DOC_VERSION,
            schema_hash: BYBIT_MARKET_TICKERS_SCHEMA_HASH,
            fixture_id: BYBIT_MARKET_TICKERS_FIXTURE_ID,
            parser_test: BYBIT_MARKET_TICKERS_PARSER_TEST,
            request_builder_test: BYBIT_MARKET_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_MARKET_TICKERS_CHECKED_AT,
            doc_version: BYBIT_MARKET_TICKERS_DOC_VERSION,
            schema_hash: BYBIT_MARKET_TICKERS_SCHEMA_HASH,
            fixture_id: BYBIT_MARKET_TICKERS_FIXTURE_ID,
            parser_test: BYBIT_MARKET_TICKERS_PARSER_TEST,
            request_builder_test: BYBIT_MARKET_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_MARKET_TICKERS_CHECKED_AT,
            doc_version: BYBIT_MARKET_TICKERS_DOC_VERSION,
            schema_hash: BYBIT_MARKET_TICKERS_SCHEMA_HASH,
            fixture_id: BYBIT_MARKET_TICKERS_FIXTURE_ID,
            parser_test: BYBIT_MARKET_TICKERS_PARSER_TEST,
            request_builder_test: BYBIT_MARKET_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v2/public/time",
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_SERVER_TIME_CHECKED_AT,
            doc_version: BITGET_SERVER_TIME_DOC_VERSION,
            schema_hash: BITGET_SERVER_TIME_SCHEMA_HASH,
            fixture_id: BITGET_SERVER_TIME_FIXTURE_ID,
            parser_test: BITGET_SERVER_TIME_TEST,
            request_builder_test: BITGET_SERVER_TIME_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/orderbook",
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_UTA_ORDERBOOK_CHECKED_AT,
            doc_version: BITGET_UTA_ORDERBOOK_DOC_VERSION,
            schema_hash: BITGET_UTA_ORDERBOOK_SCHEMA_HASH,
            fixture_id: BITGET_UTA_ORDERBOOK_FIXTURE_ID,
            parser_test: BITGET_UTA_ORDERBOOK_PARSER_TEST,
            request_builder_test: BITGET_UTA_ORDERBOOK_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/instruments",
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_UTA_INSTRUMENTS_CHECKED_AT,
            doc_version: BITGET_UTA_INSTRUMENTS_DOC_VERSION,
            schema_hash: BITGET_UTA_INSTRUMENTS_SCHEMA_HASH,
            fixture_id: BITGET_UTA_INSTRUMENTS_FIXTURE_ID,
            parser_test: BITGET_UTA_INSTRUMENTS_PARSER_TEST,
            request_builder_test: BITGET_UTA_INSTRUMENTS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/current-fund-rate",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_UTA_CURRENT_FUNDING_CHECKED_AT,
            doc_version: BITGET_UTA_CURRENT_FUNDING_DOC_VERSION,
            schema_hash: BITGET_UTA_CURRENT_FUNDING_SCHEMA_HASH,
            fixture_id: BITGET_UTA_CURRENT_FUNDING_FIXTURE_ID,
            parser_test: BITGET_UTA_CURRENT_FUNDING_PARSER_TEST,
            request_builder_test: BITGET_UTA_CURRENT_FUNDING_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_UTA_TICKERS_CHECKED_AT,
            doc_version: BITGET_UTA_TICKERS_DOC_VERSION,
            schema_hash: BITGET_UTA_TICKERS_SCHEMA_HASH,
            fixture_id: BITGET_UTA_TICKERS_FIXTURE_ID,
            parser_test: BITGET_UTA_TICKERS_PARSER_TEST,
            request_builder_test: BITGET_UTA_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_UTA_TICKERS_CHECKED_AT,
            doc_version: BITGET_UTA_TICKERS_DOC_VERSION,
            schema_hash: BITGET_UTA_TICKERS_SCHEMA_HASH,
            fixture_id: BITGET_UTA_TICKERS_FIXTURE_ID,
            parser_test: BITGET_UTA_TICKERS_PARSER_TEST,
            request_builder_test: BITGET_UTA_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/market/funding_info",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: GATE_CROSSEX_FUNDING_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/accounts",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: GATE_CROSSEX_ACCOUNT_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/accounts",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: GATE_CROSSEX_ACCOUNT_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/open_orders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: GATE_CROSSEX_OPEN_ORDERS_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/orders/{order_id}",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: GATE_CROSSEX_ORDER_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/positions",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: GATE_CROSSEX_POSITION_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/0/private/GetWebSocketsToken",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: KRAKEN_SPOT_TOKEN_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/0/private/BalanceEx",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: KRAKEN_SPOT_BALANCE_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/0/private/OpenOrders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: KRAKEN_SPOT_OPEN_ORDERS_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/0/private/QueryOrders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: KRAKEN_SPOT_QUERY_ORDERS_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/derivatives/api/v3/sendorder",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: KRAKEN_FUTURES_SEND_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/derivatives/api/v3/cancelorder",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: KRAKEN_FUTURES_CANCEL_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Get,
        path: "/derivatives/api/v3/openorders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: KRAKEN_FUTURES_OPEN_ORDERS_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/derivatives/api/v3/orders/status",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: KRAKEN_FUTURES_ORDER_STATUS_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Get,
        path: "/derivatives/api/v3/accounts",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: KRAKEN_FUTURES_ACCOUNT_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kraken,
        method: HttpMethod::Get,
        path: "/derivatives/api/v3/openpositions",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: KRAKEN_FUTURES_POSITION_EVIDENCE,
    },
    EndpointEvidenceEntry {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_L2BOOK_CHECKED_AT,
            doc_version: HYPERLIQUID_L2BOOK_DOC_VERSION,
            schema_hash: HYPERLIQUID_L2BOOK_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_L2BOOK_FIXTURE_ID,
            parser_test: HYPERLIQUID_L2BOOK_PARSER_TEST,
            request_builder_test: HYPERLIQUID_L2BOOK_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_META_CTXS_CHECKED_AT,
            doc_version: HYPERLIQUID_META_CTXS_DOC_VERSION,
            schema_hash: HYPERLIQUID_META_CTXS_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_META_CTXS_FIXTURE_ID,
            parser_test: HYPERLIQUID_META_CTXS_PARSER_TEST,
            request_builder_test: HYPERLIQUID_META_CTXS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_META_CTXS_CHECKED_AT,
            doc_version: HYPERLIQUID_META_CTXS_DOC_VERSION,
            schema_hash: HYPERLIQUID_META_CTXS_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_META_CTXS_FIXTURE_ID,
            parser_test: HYPERLIQUID_META_CTXS_PARSER_TEST,
            request_builder_test: HYPERLIQUID_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_SPOT_META_CTXS_CHECKED_AT,
            doc_version: HYPERLIQUID_SPOT_META_CTXS_DOC_VERSION,
            schema_hash: HYPERLIQUID_SPOT_META_CTXS_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_SPOT_META_CTXS_FIXTURE_ID,
            parser_test: HYPERLIQUID_SPOT_META_CTXS_PARSER_TEST,
            request_builder_test: HYPERLIQUID_SPOT_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_PREDICTED_FUNDINGS_CHECKED_AT,
            doc_version: HYPERLIQUID_PREDICTED_FUNDINGS_DOC_VERSION,
            schema_hash: HYPERLIQUID_PREDICTED_FUNDINGS_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_PREDICTED_FUNDINGS_FIXTURE_ID,
            parser_test: HYPERLIQUID_PREDICTED_FUNDINGS_PARSER_TEST,
            request_builder_test: HYPERLIQUID_PREDICTED_FUNDINGS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/level2/depth20",
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_DEPTH20_CHECKED_AT,
            doc_version: KUCOIN_DEPTH20_DOC_VERSION,
            schema_hash: KUCOIN_DEPTH20_SCHEMA_HASH,
            fixture_id: KUCOIN_DEPTH20_FIXTURE_ID,
            parser_test: KUCOIN_DEPTH20_PARSER_TEST,
            request_builder_test: KUCOIN_DEPTH20_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/spot/time",
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_SERVER_TIME_CHECKED_AT,
            doc_version: GATE_SERVER_TIME_DOC_VERSION,
            schema_hash: GATE_SERVER_TIME_SCHEMA_HASH,
            fixture_id: GATE_SERVER_TIME_FIXTURE_ID,
            parser_test: GATE_SERVER_TIME_TEST,
            request_builder_test: GATE_SERVER_TIME_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/contracts",
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_CONTRACTS_CHECKED_AT,
            doc_version: GATE_CONTRACTS_DOC_VERSION,
            schema_hash: GATE_CONTRACTS_SCHEMA_HASH,
            fixture_id: GATE_CONTRACTS_FIXTURE_ID,
            parser_test: GATE_CONTRACTS_PARSER_TEST,
            request_builder_test: GATE_CONTRACTS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/contracts",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_CONTRACTS_CHECKED_AT,
            doc_version: GATE_CONTRACTS_DOC_VERSION,
            schema_hash: GATE_CONTRACTS_SCHEMA_HASH,
            fixture_id: GATE_CONTRACTS_FIXTURE_ID,
            parser_test: GATE_CONTRACTS_PARSER_TEST,
            request_builder_test: GATE_CONTRACTS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/order_book",
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_ORDERBOOK_CHECKED_AT,
            doc_version: GATE_ORDERBOOK_DOC_VERSION,
            schema_hash: GATE_ORDERBOOK_SCHEMA_HASH,
            fixture_id: GATE_ORDERBOOK_FIXTURE_ID,
            parser_test: GATE_ORDERBOOK_PARSER_TEST,
            request_builder_test: GATE_ORDERBOOK_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_FUTURES_TICKERS_CHECKED_AT,
            doc_version: GATE_FUTURES_TICKERS_DOC_VERSION,
            schema_hash: GATE_FUTURES_TICKERS_SCHEMA_HASH,
            fixture_id: GATE_FUTURES_TICKERS_FIXTURE_ID,
            parser_test: GATE_FUTURES_TICKERS_PARSER_TEST,
            request_builder_test: GATE_FUTURES_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/spot/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_SPOT_TICKERS_CHECKED_AT,
            doc_version: GATE_SPOT_TICKERS_DOC_VERSION,
            schema_hash: GATE_SPOT_TICKERS_SCHEMA_HASH,
            fixture_id: GATE_SPOT_TICKERS_FIXTURE_ID,
            parser_test: GATE_SPOT_TICKERS_PARSER_TEST,
            request_builder_test: GATE_SPOT_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/api/v1/timestamp",
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_SERVER_TIME_CHECKED_AT,
            doc_version: HTX_SERVER_TIME_DOC_VERSION,
            schema_hash: HTX_SERVER_TIME_SCHEMA_HASH,
            fixture_id: HTX_SERVER_TIME_FIXTURE_ID,
            parser_test: HTX_SERVER_TIME_TEST,
            request_builder_test: HTX_SERVER_TIME_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-ex/market/depth",
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_MARKET_DEPTH_CHECKED_AT,
            doc_version: HTX_MARKET_DEPTH_DOC_VERSION,
            schema_hash: HTX_MARKET_DEPTH_SCHEMA_HASH,
            fixture_id: HTX_MARKET_DEPTH_FIXTURE_ID,
            parser_test: HTX_MARKET_DEPTH_PARSER_TEST,
            request_builder_test: HTX_MARKET_DEPTH_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_contract_info",
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_CONTRACT_INFO_CHECKED_AT,
            doc_version: HTX_CONTRACT_INFO_DOC_VERSION,
            schema_hash: HTX_CONTRACT_INFO_SCHEMA_HASH,
            fixture_id: HTX_CONTRACT_INFO_FIXTURE_ID,
            parser_test: HTX_CONTRACT_INFO_PARSER_TEST,
            request_builder_test: HTX_CONTRACT_INFO_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_batch_funding_rate",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_BATCH_FUNDING_CHECKED_AT,
            doc_version: HTX_BATCH_FUNDING_DOC_VERSION,
            schema_hash: HTX_BATCH_FUNDING_SCHEMA_HASH,
            fixture_id: HTX_BATCH_FUNDING_FIXTURE_ID,
            parser_test: HTX_BATCH_FUNDING_PARSER_TEST,
            request_builder_test: HTX_BATCH_FUNDING_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-ex/market/detail/merged",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_MARKET_DETAIL_MERGED_CHECKED_AT,
            doc_version: HTX_MARKET_DETAIL_MERGED_DOC_VERSION,
            schema_hash: HTX_MARKET_DETAIL_MERGED_SCHEMA_HASH,
            fixture_id: HTX_MARKET_DETAIL_MERGED_FIXTURE_ID,
            parser_test: HTX_MARKET_DETAIL_MERGED_PARSER_TEST,
            request_builder_test: HTX_MARKET_DETAIL_MERGED_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/index/market/history/linear_swap_mark_price_kline",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_MARK_PRICE_KLINE_CHECKED_AT,
            doc_version: HTX_MARK_PRICE_KLINE_DOC_VERSION,
            schema_hash: HTX_MARK_PRICE_KLINE_SCHEMA_HASH,
            fixture_id: HTX_MARK_PRICE_KLINE_FIXTURE_ID,
            parser_test: HTX_MARK_PRICE_KLINE_PARSER_TEST,
            request_builder_test: HTX_MARK_PRICE_KLINE_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_index",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_SWAP_INDEX_CHECKED_AT,
            doc_version: HTX_SWAP_INDEX_DOC_VERSION,
            schema_hash: HTX_SWAP_INDEX_SCHEMA_HASH,
            fixture_id: HTX_SWAP_INDEX_FIXTURE_ID,
            parser_test: HTX_SWAP_INDEX_PARSER_TEST,
            request_builder_test: HTX_SWAP_INDEX_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_open_interest",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::OpenInterest,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_OPEN_INTEREST_CHECKED_AT,
            doc_version: HTX_OPEN_INTEREST_DOC_VERSION,
            schema_hash: HTX_OPEN_INTEREST_SCHEMA_HASH,
            fixture_id: HTX_OPEN_INTEREST_FIXTURE_ID,
            parser_test: HTX_OPEN_INTEREST_PARSER_TEST,
            request_builder_test: HTX_OPEN_INTEREST_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/market/tickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_SPOT_MARKET_TICKERS_CHECKED_AT,
            doc_version: HTX_SPOT_MARKET_TICKERS_DOC_VERSION,
            schema_hash: HTX_SPOT_MARKET_TICKERS_SCHEMA_HASH,
            fixture_id: HTX_SPOT_MARKET_TICKERS_FIXTURE_ID,
            parser_test: HTX_SPOT_MARKET_TICKERS_PARSER_TEST,
            request_builder_test: HTX_SPOT_MARKET_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/timestamp",
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_SERVER_TIME_CHECKED_AT,
            doc_version: KUCOIN_SERVER_TIME_DOC_VERSION,
            schema_hash: KUCOIN_SERVER_TIME_SCHEMA_HASH,
            fixture_id: KUCOIN_SERVER_TIME_FIXTURE_ID,
            parser_test: KUCOIN_SERVER_TIME_TEST,
            request_builder_test: KUCOIN_SERVER_TIME_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/contracts/active",
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_CONTRACTS_NATIVE_CHECKED_AT,
            doc_version: KUCOIN_CONTRACTS_NATIVE_DOC_VERSION,
            schema_hash: KUCOIN_CONTRACTS_NATIVE_SCHEMA_HASH,
            fixture_id: KUCOIN_CONTRACTS_NATIVE_FIXTURE_ID,
            parser_test: KUCOIN_CONTRACTS_NATIVE_PARSER_TEST,
            request_builder_test: KUCOIN_CONTRACTS_ACTIVE_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/contracts/active",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_CONTRACTS_ACTIVE_CHECKED_AT,
            doc_version: KUCOIN_CONTRACTS_ACTIVE_DOC_VERSION,
            schema_hash: KUCOIN_CONTRACTS_ACTIVE_SCHEMA_HASH,
            fixture_id: KUCOIN_CONTRACTS_ACTIVE_FIXTURE_ID,
            parser_test: KUCOIN_CONTRACTS_ACTIVE_PARSER_TEST,
            request_builder_test: KUCOIN_CONTRACTS_ACTIVE_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/allTickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_FUTURES_ALL_TICKERS_CHECKED_AT,
            doc_version: KUCOIN_FUTURES_ALL_TICKERS_DOC_VERSION,
            schema_hash: KUCOIN_FUTURES_ALL_TICKERS_SCHEMA_HASH,
            fixture_id: KUCOIN_FUTURES_ALL_TICKERS_FIXTURE_ID,
            parser_test: KUCOIN_FUTURES_ALL_TICKERS_PARSER_TEST,
            request_builder_test: KUCOIN_FUTURES_ALL_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/market/allTickers",
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_SPOT_MARKET_ALL_TICKERS_CHECKED_AT,
            doc_version: KUCOIN_SPOT_MARKET_ALL_TICKERS_DOC_VERSION,
            schema_hash: KUCOIN_SPOT_MARKET_ALL_TICKERS_SCHEMA_HASH,
            fixture_id: KUCOIN_SPOT_MARKET_ALL_TICKERS_FIXTURE_ID,
            parser_test: KUCOIN_SPOT_MARKET_ALL_TICKERS_PARSER_TEST,
            request_builder_test: KUCOIN_SPOT_MARKET_ALL_TICKERS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Post,
        path: "/fapi/v1/order",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_PLACE_ORDER_CHECKED_AT,
            doc_version: BINANCE_USDM_PLACE_ORDER_DOC_VERSION,
            schema_hash: BINANCE_USDM_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_PLACE_ORDER_FIXTURE_ID,
            parser_test: BINANCE_USDM_PLACE_ORDER_PARSER_TEST,
            request_builder_test: BINANCE_USDM_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Post,
        path: "/fapi/v1/order/test",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_ORDER_TEST_CHECKED_AT,
            doc_version: BINANCE_USDM_ORDER_TEST_DOC_VERSION,
            schema_hash: UNRECORDED_EVIDENCE_MARKER,
            fixture_id: UNRECORDED_EVIDENCE_MARKER,
            parser_test: UNRECORDED_EVIDENCE_MARKER,
            request_builder_test: BINANCE_USDM_ORDER_TEST_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Delete,
        path: "/fapi/v1/order",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_CANCEL_ORDER_CHECKED_AT,
            doc_version: BINANCE_USDM_CANCEL_ORDER_DOC_VERSION,
            schema_hash: BINANCE_USDM_CANCEL_ORDER_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_CANCEL_ORDER_FIXTURE_ID,
            parser_test: BINANCE_USDM_CANCEL_ORDER_PARSER_TEST,
            request_builder_test: BINANCE_USDM_CANCEL_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Post,
        path: "/api/v5/trade/order",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_PLACE_ORDER_CHECKED_AT,
            doc_version: OKX_PLACE_ORDER_DOC_VERSION,
            schema_hash: OKX_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: OKX_PLACE_ORDER_FIXTURE_ID,
            parser_test: OKX_PLACE_ORDER_PARSER_TEST,
            request_builder_test: OKX_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Post,
        path: "/api/v5/trade/order-precheck",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_ORDER_PRECHECK_CHECKED_AT,
            doc_version: OKX_ORDER_PRECHECK_DOC_VERSION,
            schema_hash: UNRECORDED_EVIDENCE_MARKER,
            fixture_id: UNRECORDED_EVIDENCE_MARKER,
            parser_test: UNRECORDED_EVIDENCE_MARKER,
            request_builder_test: OKX_ORDER_PRECHECK_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Post,
        path: "/api/v5/trade/cancel-order",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_CANCEL_ORDER_CHECKED_AT,
            doc_version: OKX_CANCEL_ORDER_DOC_VERSION,
            schema_hash: OKX_CANCEL_ORDER_SCHEMA_HASH,
            fixture_id: OKX_CANCEL_ORDER_FIXTURE_ID,
            parser_test: OKX_CANCEL_ORDER_PARSER_TEST,
            request_builder_test: OKX_CANCEL_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/trade/order",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_GET_ORDER_CHECKED_AT,
            doc_version: OKX_GET_ORDER_DOC_VERSION,
            schema_hash: OKX_GET_ORDER_SCHEMA_HASH,
            fixture_id: OKX_GET_ORDER_FIXTURE_ID,
            parser_test: OKX_GET_ORDER_PARSER_TEST,
            request_builder_test: OKX_GET_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/trade/orders-pending",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_OPEN_ORDERS_CHECKED_AT,
            doc_version: OKX_OPEN_ORDERS_DOC_VERSION,
            schema_hash: OKX_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: OKX_OPEN_ORDERS_FIXTURE_ID,
            parser_test: OKX_OPEN_ORDERS_PARSER_TEST,
            request_builder_test: OKX_OPEN_ORDERS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/account/config",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_ACCOUNT_CONFIG_CHECKED_AT,
            doc_version: OKX_ACCOUNT_CONFIG_DOC_VERSION,
            schema_hash: OKX_ACCOUNT_CONFIG_SCHEMA_HASH,
            fixture_id: OKX_ACCOUNT_CONFIG_FIXTURE_ID,
            parser_test: OKX_ACCOUNT_CONFIG_PARSER_TEST,
            request_builder_test: OKX_ACCOUNT_CONFIG_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/account/balance",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_ACCOUNT_BALANCE_CHECKED_AT,
            doc_version: OKX_ACCOUNT_BALANCE_DOC_VERSION,
            schema_hash: OKX_ACCOUNT_BALANCE_SCHEMA_HASH,
            fixture_id: OKX_ACCOUNT_BALANCE_FIXTURE_ID,
            parser_test: OKX_ACCOUNT_BALANCE_PARSER_TEST,
            request_builder_test: OKX_ACCOUNT_BALANCE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/account/bills",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
        meta: EndpointEvidenceMeta {
            checked_at: "2026-07-02",
            doc_version: "okx-v5-account-bills-funding-fee-2026-07-02",
            schema_hash: "sha256:b3a76a0b0cf782a3f2a599b865d801f44b07c3549bc6027329125a21e6133c8a",
            fixture_id: "crates/exchange/fixtures/okx/account_bills_funding_fee_btc_usdt_swap.json",
            parser_test: "okx_funding_payment_parses_official_bills_fixture",
            request_builder_test: "okx_funding_payments_request_uses_bills_subtype_filter",
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/account/positions",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: OKX_ACCOUNT_POSITIONS_CHECKED_AT,
            doc_version: OKX_ACCOUNT_POSITIONS_DOC_VERSION,
            schema_hash: OKX_ACCOUNT_POSITIONS_SCHEMA_HASH,
            fixture_id: OKX_ACCOUNT_POSITIONS_FIXTURE_ID,
            parser_test: OKX_ACCOUNT_POSITIONS_PARSER_TEST,
            request_builder_test: OKX_ACCOUNT_POSITIONS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Post,
        path: "/v5/order/create",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_PLACE_ORDER_CHECKED_AT,
            doc_version: BYBIT_PLACE_ORDER_DOC_VERSION,
            schema_hash: BYBIT_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: BYBIT_PLACE_ORDER_FIXTURE_ID,
            parser_test: BYBIT_PLACE_ORDER_PARSER_TEST,
            request_builder_test: BYBIT_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Post,
        path: "/v5/order/pre-check",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_ORDER_PRECHECK_CHECKED_AT,
            doc_version: BYBIT_ORDER_PRECHECK_DOC_VERSION,
            schema_hash: UNRECORDED_EVIDENCE_MARKER,
            fixture_id: UNRECORDED_EVIDENCE_MARKER,
            parser_test: UNRECORDED_EVIDENCE_MARKER,
            request_builder_test: BYBIT_ORDER_PRECHECK_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Post,
        path: "/v5/order/cancel",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_CANCEL_ORDER_CHECKED_AT,
            doc_version: BYBIT_CANCEL_ORDER_DOC_VERSION,
            schema_hash: BYBIT_CANCEL_ORDER_SCHEMA_HASH,
            fixture_id: BYBIT_CANCEL_ORDER_FIXTURE_ID,
            parser_test: BYBIT_CANCEL_ORDER_PARSER_TEST,
            request_builder_test: BYBIT_CANCEL_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/order/realtime",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_GET_ORDER_CHECKED_AT,
            doc_version: BYBIT_GET_ORDER_DOC_VERSION,
            schema_hash: BYBIT_GET_ORDER_SCHEMA_HASH,
            fixture_id: BYBIT_GET_ORDER_FIXTURE_ID,
            parser_test: BYBIT_GET_ORDER_PARSER_TEST,
            request_builder_test: BYBIT_GET_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/position/list",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_POSITION_MODE_CHECKED_AT,
            doc_version: BYBIT_POSITION_MODE_DOC_VERSION,
            schema_hash: BYBIT_POSITION_MODE_SCHEMA_HASH,
            fixture_id: BYBIT_POSITION_MODE_FIXTURE_ID,
            parser_test: BYBIT_POSITION_MODE_PARSER_TEST,
            request_builder_test: BYBIT_POSITION_MODE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/position/list",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_POSITIONS_CHECKED_AT,
            doc_version: BYBIT_POSITIONS_DOC_VERSION,
            schema_hash: BYBIT_POSITIONS_SCHEMA_HASH,
            fixture_id: BYBIT_POSITIONS_FIXTURE_ID,
            parser_test: BYBIT_POSITIONS_PARSER_TEST,
            request_builder_test: BYBIT_POSITIONS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Post,
        path: "/api/v3/trade/place-order",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_PLACE_ORDER_CHECKED_AT,
            doc_version: BITGET_PLACE_ORDER_DOC_VERSION,
            schema_hash: BITGET_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: BITGET_PLACE_ORDER_FIXTURE_ID,
            parser_test: BITGET_PLACE_ORDER_PARSER_TEST,
            request_builder_test: BITGET_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Post,
        path: "/api/v3/trade/cancel-order",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_CANCEL_ORDER_CHECKED_AT,
            doc_version: BITGET_CANCEL_ORDER_DOC_VERSION,
            schema_hash: BITGET_CANCEL_ORDER_SCHEMA_HASH,
            fixture_id: BITGET_CANCEL_ORDER_FIXTURE_ID,
            parser_test: BITGET_CANCEL_ORDER_PARSER_TEST,
            request_builder_test: BITGET_CANCEL_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/position/current-position",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_CURRENT_POSITION_CHECKED_AT,
            doc_version: BITGET_CURRENT_POSITION_DOC_VERSION,
            schema_hash: BITGET_CURRENT_POSITION_SCHEMA_HASH,
            fixture_id: BITGET_CURRENT_POSITION_FIXTURE_ID,
            parser_test: BITGET_CURRENT_POSITION_PARSER_TEST,
            request_builder_test: BITGET_CURRENT_POSITION_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/trade/order-info",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_GET_ORDER_CHECKED_AT,
            doc_version: BITGET_GET_ORDER_DOC_VERSION,
            schema_hash: BITGET_GET_ORDER_SCHEMA_HASH,
            fixture_id: BITGET_GET_ORDER_FIXTURE_ID,
            parser_test: BITGET_GET_ORDER_PARSER_TEST,
            request_builder_test: BITGET_GET_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/account/assets",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_ACCOUNT_ASSETS_CHECKED_AT,
            doc_version: BITGET_ACCOUNT_ASSETS_DOC_VERSION,
            schema_hash: BITGET_ACCOUNT_ASSETS_SCHEMA_HASH,
            fixture_id: BITGET_ACCOUNT_ASSETS_FIXTURE_ID,
            parser_test: BITGET_ACCOUNT_ASSETS_PARSER_TEST,
            request_builder_test: BITGET_ACCOUNT_ASSETS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/account/financial-records",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
        meta: EndpointEvidenceMeta {
            checked_at: "2026-07-02",
            doc_version: "bitget-uta-financial-records-funding-fee-2026-07-02",
            schema_hash: "sha256:3c89aa83d14642364cda661dfd098689e2941034cfa85ea270a85729fc4ae8c0",
            fixture_id:
                "crates/exchange/fixtures/bitget/uta_financial_records_funding_fee_btcusdt.json",
            parser_test: "bitget_uta_funding_payment_parses_official_financial_records_fixture",
            request_builder_test:
                "bitget_uta_funding_payments_request_uses_financial_records_type_filter",
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/trade/unfilled-orders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: BITGET_OPEN_ORDERS_CHECKED_AT,
            doc_version: BITGET_OPEN_ORDERS_DOC_VERSION,
            schema_hash: BITGET_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: BITGET_OPEN_ORDERS_FIXTURE_ID,
            parser_test: BITGET_OPEN_ORDERS_PARSER_TEST,
            request_builder_test: BITGET_OPEN_ORDERS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v3/positionRisk",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_POSITIONS_CHECKED_AT,
            doc_version: BINANCE_USDM_POSITIONS_DOC_VERSION,
            schema_hash: BINANCE_USDM_POSITIONS_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_POSITIONS_FIXTURE_ID,
            parser_test: BINANCE_USDM_POSITIONS_PARSER_TEST,
            request_builder_test: BINANCE_USDM_POSITIONS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v3/balance",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_BALANCE_CHECKED_AT,
            doc_version: BINANCE_USDM_BALANCE_DOC_VERSION,
            schema_hash: BINANCE_USDM_BALANCE_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_BALANCE_FIXTURE_ID,
            parser_test: BINANCE_USDM_BALANCE_PARSER_TEST,
            request_builder_test: BINANCE_USDM_BALANCE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/income",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
        meta: EndpointEvidenceMeta {
            checked_at: "2026-07-02",
            doc_version: "binance-usdm-income-history-funding-fee-2026-07-02",
            schema_hash: "sha256:a47478601a0fc110d0836509bdf9dbf58631659aef43587c6593845876e83b50",
            fixture_id: "crates/exchange/fixtures/binance/usdm_income_funding_fee_btcusdt.json",
            parser_test: "binance_funding_payment_parses_official_income_fixture",
            request_builder_test:
                "binance_funding_payments_request_uses_income_history_funding_fee_filter",
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/order",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_GET_ORDER_CHECKED_AT,
            doc_version: BINANCE_USDM_GET_ORDER_DOC_VERSION,
            schema_hash: BINANCE_USDM_GET_ORDER_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_GET_ORDER_FIXTURE_ID,
            parser_test: BINANCE_USDM_GET_ORDER_PARSER_TEST,
            request_builder_test: BINANCE_USDM_GET_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/openOrders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_OPEN_ORDERS_CHECKED_AT,
            doc_version: BINANCE_USDM_OPEN_ORDERS_DOC_VERSION,
            schema_hash: BINANCE_USDM_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_OPEN_ORDERS_FIXTURE_ID,
            parser_test: BINANCE_USDM_OPEN_ORDERS_PARSER_TEST,
            request_builder_test: BINANCE_USDM_OPEN_ORDERS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/positionSide/dual",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_POSITION_MODE_CHECKED_AT,
            doc_version: BINANCE_USDM_POSITION_MODE_DOC_VERSION,
            schema_hash: BINANCE_USDM_POSITION_MODE_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_POSITION_MODE_FIXTURE_ID,
            parser_test: BINANCE_USDM_POSITION_MODE_PARSER_TEST,
            request_builder_test: BINANCE_USDM_POSITION_MODE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/commissionRate",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: EndpointEvidenceMeta {
            checked_at: BINANCE_USDM_COMMISSION_RATE_CHECKED_AT,
            doc_version: BINANCE_USDM_COMMISSION_RATE_DOC_VERSION,
            schema_hash: BINANCE_USDM_COMMISSION_RATE_SCHEMA_HASH,
            fixture_id: BINANCE_USDM_COMMISSION_RATE_FIXTURE_ID,
            parser_test: BINANCE_USDM_COMMISSION_RATE_PARSER_TEST,
            request_builder_test: BINANCE_USDM_COMMISSION_RATE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/account/wallet-balance",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: BYBIT_WALLET_BALANCE_CHECKED_AT,
            doc_version: BYBIT_WALLET_BALANCE_DOC_VERSION,
            schema_hash: BYBIT_WALLET_BALANCE_SCHEMA_HASH,
            fixture_id: BYBIT_WALLET_BALANCE_FIXTURE_ID,
            parser_test: BYBIT_WALLET_BALANCE_PARSER_TEST,
            request_builder_test: BYBIT_WALLET_BALANCE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/account/transaction-log",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
        meta: EndpointEvidenceMeta {
            checked_at: "2026-07-02",
            doc_version: "bybit-v5-account-transaction-log-funding-fee-2026-07-02",
            schema_hash: "sha256:94271121cfe93ff9e916e5affb1e267dfcf175caac391bd58ff0595cf67f0595",
            fixture_id: "crates/exchange/fixtures/bybit/account_transaction_log_funding_fee.json",
            parser_test: "bybit_funding_payment_parses_official_transaction_log_fixture",
            request_builder_test:
                "bybit_funding_payments_request_uses_transaction_log_settlement_filter",
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/orders/{order_id}",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_GET_ORDER_CHECKED_AT,
            doc_version: GATE_GET_ORDER_DOC_VERSION,
            schema_hash: GATE_GET_ORDER_SCHEMA_HASH,
            fixture_id: GATE_GET_ORDER_FIXTURE_ID,
            parser_test: GATE_GET_ORDER_PARSER_TEST,
            request_builder_test: GATE_GET_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Delete,
        path: "/api/v4/futures/usdt/orders/{order_id}",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_CANCEL_ORDER_CHECKED_AT,
            doc_version: GATE_CANCEL_ORDER_DOC_VERSION,
            schema_hash: UNRECORDED_EVIDENCE_MARKER,
            fixture_id: UNRECORDED_EVIDENCE_MARKER,
            parser_test: UNRECORDED_EVIDENCE_MARKER,
            request_builder_test: GATE_CANCEL_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/my_trades",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_MY_TRADES_CHECKED_AT,
            doc_version: GATE_MY_TRADES_DOC_VERSION,
            schema_hash: GATE_MY_TRADES_SCHEMA_HASH,
            fixture_id: GATE_MY_TRADES_FIXTURE_ID,
            parser_test: GATE_MY_TRADES_PARSER_TEST,
            request_builder_test: GATE_MY_TRADES_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/fee",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_FUTURES_FEE_CHECKED_AT,
            doc_version: GATE_FUTURES_FEE_DOC_VERSION,
            schema_hash: GATE_FUTURES_FEE_SCHEMA_HASH,
            fixture_id: GATE_FUTURES_FEE_FIXTURE_ID,
            parser_test: GATE_FUTURES_FEE_PARSER_TEST,
            request_builder_test: GATE_FUTURES_FEE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/orders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_OPEN_ORDERS_CHECKED_AT,
            doc_version: GATE_OPEN_ORDERS_DOC_VERSION,
            schema_hash: GATE_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: GATE_OPEN_ORDERS_FIXTURE_ID,
            parser_test: GATE_OPEN_ORDERS_PARSER_TEST,
            request_builder_test: GATE_OPEN_ORDERS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/accounts",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_ACCOUNT_BALANCE_CHECKED_AT,
            doc_version: GATE_ACCOUNT_BALANCE_DOC_VERSION,
            schema_hash: GATE_ACCOUNT_BALANCE_SCHEMA_HASH,
            fixture_id: GATE_ACCOUNT_BALANCE_FIXTURE_ID,
            parser_test: GATE_ACCOUNT_BALANCE_PARSER_TEST,
            request_builder_test: GATE_ACCOUNT_BALANCE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/account_book",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
        meta: EndpointEvidenceMeta {
            checked_at: "2026-07-02",
            doc_version: "gate-apiv4-futures-account-book-funding-fee-2026-07-02",
            schema_hash: "sha256:9992355eff36b23d899f67961e1d1c4b9e4edc648e1828063e83de2a11706801",
            fixture_id:
                "crates/exchange/fixtures/gate/futures_usdt_account_book_fund_btc_usdt.json",
            parser_test: "gate_funding_payment_parses_official_account_book_fixture",
            request_builder_test: "gate_funding_payments_request_uses_account_book_fund_filter",
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/positions",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_POSITIONS_CHECKED_AT,
            doc_version: GATE_POSITIONS_DOC_VERSION,
            schema_hash: GATE_POSITIONS_SCHEMA_HASH,
            fixture_id: GATE_POSITIONS_FIXTURE_ID,
            parser_test: GATE_POSITIONS_PARSER_TEST,
            request_builder_test: GATE_POSITIONS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Post,
        path: "/api/v1/orders",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_PLACE_ORDER_CHECKED_AT,
            doc_version: KUCOIN_PLACE_ORDER_DOC_VERSION,
            schema_hash: KUCOIN_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: KUCOIN_PLACE_ORDER_FIXTURE_ID,
            parser_test: KUCOIN_PLACE_ORDER_PARSER_TEST,
            request_builder_test: KUCOIN_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Post,
        path: "/api/v1/orders/test",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_ORDER_TEST_CHECKED_AT,
            doc_version: KUCOIN_ORDER_TEST_DOC_VERSION,
            schema_hash: UNRECORDED_EVIDENCE_MARKER,
            fixture_id: UNRECORDED_EVIDENCE_MARKER,
            parser_test: UNRECORDED_EVIDENCE_MARKER,
            request_builder_test: KUCOIN_ORDER_TEST_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Delete,
        path: "/api/v1/orders/client-order/{clientOid}",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_CANCEL_ORDER_CHECKED_AT,
            doc_version: KUCOIN_CANCEL_ORDER_DOC_VERSION,
            schema_hash: KUCOIN_CANCEL_ORDER_SCHEMA_HASH,
            fixture_id: KUCOIN_CANCEL_ORDER_FIXTURE_ID,
            parser_test: KUCOIN_CANCEL_ORDER_PARSER_TEST,
            request_builder_test: KUCOIN_CANCEL_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Delete,
        path: "/api/v1/orders/{orderId}",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_CANCEL_BY_ORDER_ID_CHECKED_AT,
            doc_version: KUCOIN_CANCEL_BY_ORDER_ID_DOC_VERSION,
            schema_hash: KUCOIN_CANCEL_ORDER_SCHEMA_HASH,
            fixture_id: KUCOIN_CANCEL_ORDER_FIXTURE_ID,
            parser_test: KUCOIN_CANCEL_ORDER_PARSER_TEST,
            request_builder_test: KUCOIN_CANCEL_BY_ORDER_ID_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Gate,
        method: HttpMethod::Post,
        path: "/api/v4/futures/usdt/orders",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: GATE_PLACE_ORDER_CHECKED_AT,
            doc_version: GATE_PLACE_ORDER_DOC_VERSION,
            schema_hash: GATE_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: GATE_PLACE_ORDER_FIXTURE_ID,
            parser_test: GATE_PLACE_ORDER_PARSER_TEST,
            request_builder_test: GATE_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/positions",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_POSITIONS_CHECKED_AT,
            doc_version: KUCOIN_POSITIONS_DOC_VERSION,
            schema_hash: KUCOIN_POSITIONS_SCHEMA_HASH,
            fixture_id: KUCOIN_POSITIONS_FIXTURE_ID,
            parser_test: KUCOIN_POSITIONS_PARSER_TEST,
            request_builder_test: KUCOIN_POSITIONS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/orders/byClientOid",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_GET_ORDER_CHECKED_AT,
            doc_version: KUCOIN_GET_ORDER_DOC_VERSION,
            schema_hash: KUCOIN_GET_ORDER_SCHEMA_HASH,
            fixture_id: KUCOIN_GET_ORDER_FIXTURE_ID,
            parser_test: KUCOIN_GET_ORDER_PARSER_TEST,
            request_builder_test: KUCOIN_GET_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/orders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_OPEN_ORDERS_CHECKED_AT,
            doc_version: KUCOIN_OPEN_ORDERS_DOC_VERSION,
            schema_hash: KUCOIN_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: KUCOIN_OPEN_ORDERS_FIXTURE_ID,
            parser_test: KUCOIN_OPEN_ORDERS_PARSER_TEST,
            request_builder_test: KUCOIN_OPEN_ORDERS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/fills",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::TradeFill,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_FILLS_CHECKED_AT,
            doc_version: KUCOIN_FILLS_DOC_VERSION,
            schema_hash: KUCOIN_FILLS_SCHEMA_HASH,
            fixture_id: KUCOIN_FILLS_FIXTURE_ID,
            parser_test: KUCOIN_FILLS_PARSER_TEST,
            request_builder_test: KUCOIN_FILLS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/trade-fees",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountFeeRate,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_FEE_RATE_CHECKED_AT,
            doc_version: KUCOIN_FEE_RATE_DOC_VERSION,
            schema_hash: KUCOIN_FEE_RATE_SCHEMA_HASH,
            fixture_id: KUCOIN_FEE_RATE_FIXTURE_ID,
            parser_test: KUCOIN_FEE_RATE_PARSER_TEST,
            request_builder_test: KUCOIN_FEE_RATE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/account-overview",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_ACCOUNT_OVERVIEW_CHECKED_AT,
            doc_version: KUCOIN_ACCOUNT_OVERVIEW_DOC_VERSION,
            schema_hash: KUCOIN_ACCOUNT_OVERVIEW_SCHEMA_HASH,
            fixture_id: KUCOIN_ACCOUNT_OVERVIEW_FIXTURE_ID,
            parser_test: KUCOIN_ACCOUNT_OVERVIEW_PARSER_TEST,
            request_builder_test: KUCOIN_ACCOUNT_OVERVIEW_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/funding-history",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
        meta: EndpointEvidenceMeta {
            checked_at: "2026-07-02",
            doc_version: "kucoin-futures-private-funding-history-2026-07-02",
            schema_hash: "sha256:cab023fb504e9d1f630253d68527039865397ada77e840906ba6ad945a0edafe",
            fixture_id: "crates/exchange/fixtures/kucoin/funding_history_xbtusdtm.json",
            parser_test: "kucoin_funding_payment_parses_official_history_fixture",
            request_builder_test: "kucoin_funding_payments_request_uses_futures_history_path",
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v2/position/getPositionMode",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: EndpointEvidenceMeta {
            checked_at: KUCOIN_POSITION_MODE_CHECKED_AT,
            doc_version: KUCOIN_POSITION_MODE_DOC_VERSION,
            schema_hash: KUCOIN_POSITION_MODE_SCHEMA_HASH,
            fixture_id: KUCOIN_POSITION_MODE_FIXTURE_ID,
            parser_test: KUCOIN_POSITION_MODE_PARSER_TEST,
            request_builder_test: KUCOIN_POSITION_MODE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_account_position_info",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_ACCOUNT_POSITION_CHECKED_AT,
            doc_version: HTX_ACCOUNT_POSITION_DOC_VERSION,
            schema_hash: HTX_ACCOUNT_POSITION_SCHEMA_HASH,
            fixture_id: HTX_ACCOUNT_POSITION_FIXTURE_ID,
            parser_test: HTX_ACCOUNT_POSITION_PARSER_TEST,
            request_builder_test: HTX_ACCOUNT_POSITION_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_account_position_info",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_CROSS_ACCOUNT_POSITION_CHECKED_AT,
            doc_version: HTX_CROSS_ACCOUNT_POSITION_DOC_VERSION,
            schema_hash: HTX_CROSS_ACCOUNT_POSITION_SCHEMA_HASH,
            fixture_id: HTX_CROSS_ACCOUNT_POSITION_FIXTURE_ID,
            parser_test: HTX_CROSS_ACCOUNT_POSITION_PARSER_TEST,
            request_builder_test: HTX_CROSS_ACCOUNT_POSITION_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_openorders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_OPEN_ORDERS_CHECKED_AT,
            doc_version: HTX_OPEN_ORDERS_DOC_VERSION,
            schema_hash: HTX_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: HTX_OPEN_ORDERS_FIXTURE_ID,
            parser_test: HTX_OPEN_ORDERS_PARSER_TEST,
            request_builder_test: HTX_OPEN_ORDERS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_openorders",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_CROSS_OPEN_ORDERS_CHECKED_AT,
            doc_version: HTX_CROSS_OPEN_ORDERS_DOC_VERSION,
            schema_hash: HTX_CROSS_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: HTX_CROSS_OPEN_ORDERS_FIXTURE_ID,
            parser_test: HTX_CROSS_OPEN_ORDERS_PARSER_TEST,
            request_builder_test: HTX_CROSS_OPEN_ORDERS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_account_info",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_ACCOUNT_INFO_CHECKED_AT,
            doc_version: HTX_ACCOUNT_INFO_DOC_VERSION,
            schema_hash: HTX_ACCOUNT_INFO_SCHEMA_HASH,
            fixture_id: HTX_ACCOUNT_INFO_FIXTURE_ID,
            parser_test: HTX_ACCOUNT_INFO_PARSER_TEST,
            request_builder_test: HTX_ACCOUNT_INFO_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_account_info",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_CROSS_ACCOUNT_INFO_CHECKED_AT,
            doc_version: HTX_CROSS_ACCOUNT_INFO_DOC_VERSION,
            schema_hash: HTX_CROSS_ACCOUNT_INFO_SCHEMA_HASH,
            fixture_id: HTX_CROSS_ACCOUNT_INFO_FIXTURE_ID,
            parser_test: HTX_CROSS_ACCOUNT_INFO_PARSER_TEST,
            request_builder_test: HTX_CROSS_ACCOUNT_INFO_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v3/swap_financial_record_exact",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
        meta: EndpointEvidenceMeta {
            checked_at: "2026-07-02",
            doc_version: "htx-usdt-swap-financial-record-exact-funding-fee-2026-07-02",
            schema_hash: "sha256:c6d585a06a24f37a6af60aa58a1a92a5948f533ad29ddf4620aad8a1cd7c3158",
            fixture_id: "crates/exchange/fixtures/htx/swap_financial_record_funding_fee.json",
            parser_test: "htx_funding_payment_parses_official_financial_record_fixture",
            request_builder_test:
                "htx_funding_payments_request_uses_financial_record_funding_types",
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_order",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_PLACE_ORDER_CHECKED_AT,
            doc_version: HTX_PLACE_ORDER_DOC_VERSION,
            schema_hash: HTX_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: HTX_PLACE_ORDER_FIXTURE_ID,
            parser_test: HTX_PLACE_ORDER_PARSER_TEST,
            request_builder_test: HTX_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_order",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_ISOLATED_PLACE_ORDER_CHECKED_AT,
            doc_version: HTX_ISOLATED_PLACE_ORDER_DOC_VERSION,
            schema_hash: HTX_ISOLATED_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: HTX_ISOLATED_PLACE_ORDER_FIXTURE_ID,
            parser_test: HTX_ISOLATED_PLACE_ORDER_PARSER_TEST,
            request_builder_test: HTX_ISOLATED_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_cancel",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_CANCEL_ORDER_CHECKED_AT,
            doc_version: HTX_CANCEL_ORDER_DOC_VERSION,
            schema_hash: HTX_CANCEL_ORDER_SCHEMA_HASH,
            fixture_id: HTX_CANCEL_ORDER_FIXTURE_ID,
            parser_test: HTX_CANCEL_ORDER_PARSER_TEST,
            request_builder_test: HTX_CANCEL_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cancel",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_ISOLATED_CANCEL_ORDER_CHECKED_AT,
            doc_version: HTX_ISOLATED_CANCEL_ORDER_DOC_VERSION,
            schema_hash: HTX_ISOLATED_CANCEL_ORDER_SCHEMA_HASH,
            fixture_id: HTX_ISOLATED_CANCEL_ORDER_FIXTURE_ID,
            parser_test: HTX_ISOLATED_CANCEL_ORDER_PARSER_TEST,
            request_builder_test: HTX_ISOLATED_CANCEL_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_order_info",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_GET_ORDER_CHECKED_AT,
            doc_version: HTX_GET_ORDER_DOC_VERSION,
            schema_hash: HTX_GET_ORDER_SCHEMA_HASH,
            fixture_id: HTX_GET_ORDER_FIXTURE_ID,
            parser_test: HTX_GET_ORDER_PARSER_TEST,
            request_builder_test: HTX_GET_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_order_info",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_CROSS_GET_ORDER_CHECKED_AT,
            doc_version: HTX_CROSS_GET_ORDER_DOC_VERSION,
            schema_hash: HTX_CROSS_GET_ORDER_SCHEMA_HASH,
            fixture_id: HTX_CROSS_GET_ORDER_FIXTURE_ID,
            parser_test: HTX_CROSS_GET_ORDER_PARSER_TEST,
            request_builder_test: HTX_CROSS_GET_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v3/swap_unified_account_type",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_ACCOUNT_TYPE_CHECKED_AT,
            doc_version: HTX_ACCOUNT_TYPE_DOC_VERSION,
            schema_hash: HTX_ACCOUNT_TYPE_SCHEMA_HASH,
            fixture_id: HTX_ACCOUNT_TYPE_FIXTURE_ID,
            parser_test: HTX_ACCOUNT_TYPE_PARSER_TEST,
            request_builder_test: HTX_ACCOUNT_TYPE_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_api_trading_status",
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
        meta: EndpointEvidenceMeta {
            checked_at: HTX_API_TRADING_STATUS_CHECKED_AT,
            doc_version: HTX_API_TRADING_STATUS_DOC_VERSION,
            schema_hash: HTX_API_TRADING_STATUS_SCHEMA_HASH,
            fixture_id: HTX_API_TRADING_STATUS_FIXTURE_ID,
            parser_test: HTX_API_TRADING_STATUS_PARSER_TEST,
            request_builder_test: HTX_API_TRADING_STATUS_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
    EndpointEvidenceEntry {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/exchange",
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_PLACE_ORDER_CHECKED_AT,
            doc_version: HYPERLIQUID_PLACE_ORDER_DOC_VERSION,
            schema_hash: HYPERLIQUID_PLACE_ORDER_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_PLACE_ORDER_FIXTURE_ID,
            parser_test: HYPERLIQUID_PLACE_ORDER_PARSER_TEST,
            request_builder_test: HYPERLIQUID_PLACE_ORDER_REQUEST_TEST,
            auth_kind: SIGNED_AUTH_KIND,
        },
    },
];

// Hyperliquid multiplexes unrelated request/response schemas behind `/info`.
// Keep the raw operation and DEX scope beside the recorded evidence so a path-only
// lookup cannot borrow one operation's fixture, parser, or request contract.
const HYPERLIQUID_OPERATION_EVIDENCE: &[HyperliquidOperationEvidence] = &[
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "metaAndAssetCtxs",
        dex_scope: HyperliquidDexScope::OptionalPerpDex,
        doc_url: HYPERLIQUID_INFO_META_AND_ASSET_CTXS_DOC_URL,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_META_CTXS_CHECKED_AT,
            doc_version: HYPERLIQUID_META_CTXS_DOC_VERSION,
            schema_hash: HYPERLIQUID_META_CTXS_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_META_CTXS_FIXTURE_ID,
            parser_test: HYPERLIQUID_META_CTXS_PARSER_TEST,
            request_builder_test: HYPERLIQUID_META_CTXS_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "openOrders",
        dex_scope: HyperliquidDexScope::OptionalPerpDex,
        doc_url: HYPERLIQUID_INFO_OPEN_ORDERS_DOC_URL,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_INFO_OPERATION_CHECKED_AT,
            doc_version: HYPERLIQUID_OPEN_ORDERS_DOC_VERSION,
            schema_hash: HYPERLIQUID_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_OPEN_ORDERS_FIXTURE_ID,
            parser_test: HYPERLIQUID_OPERATION_FIXTURE_PARSER_TEST,
            request_builder_test: HYPERLIQUID_INFO_OPERATION_REQUEST_TEST,
            auth_kind: USER_ADDRESS_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "frontendOpenOrders",
        dex_scope: HyperliquidDexScope::OptionalPerpDex,
        doc_url: HYPERLIQUID_INFO_FRONTEND_OPEN_ORDERS_DOC_URL,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_INFO_OPERATION_CHECKED_AT,
            doc_version: HYPERLIQUID_FRONTEND_OPEN_ORDERS_DOC_VERSION,
            schema_hash: HYPERLIQUID_FRONTEND_OPEN_ORDERS_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_FRONTEND_OPEN_ORDERS_FIXTURE_ID,
            parser_test: HYPERLIQUID_OPERATION_FIXTURE_PARSER_TEST,
            request_builder_test: HYPERLIQUID_INFO_OPERATION_REQUEST_TEST,
            auth_kind: USER_ADDRESS_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "orderStatus",
        dex_scope: HyperliquidDexScope::NotDexScoped,
        doc_url: HYPERLIQUID_INFO_ORDER_STATUS_DOC_URL,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_GET_ORDER_CHECKED_AT,
            doc_version: HYPERLIQUID_GET_ORDER_DOC_VERSION,
            schema_hash: HYPERLIQUID_GET_ORDER_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_GET_ORDER_FIXTURE_ID,
            parser_test: HYPERLIQUID_GET_ORDER_PARSER_TEST,
            request_builder_test: HYPERLIQUID_GET_ORDER_REQUEST_TEST,
            auth_kind: USER_ADDRESS_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "clearinghouseState",
        dex_scope: HyperliquidDexScope::OptionalPerpDex,
        doc_url: HYPERLIQUID_INFO_CLEARINGHOUSE_STATE_DOC_URL,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_ACCOUNT_BALANCE_CHECKED_AT,
            doc_version: HYPERLIQUID_ACCOUNT_BALANCE_DOC_VERSION,
            schema_hash: HYPERLIQUID_ACCOUNT_BALANCE_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_ACCOUNT_BALANCE_FIXTURE_ID,
            parser_test: HYPERLIQUID_ACCOUNT_BALANCE_PARSER_TEST,
            request_builder_test: HYPERLIQUID_INFO_OPERATION_REQUEST_TEST,
            auth_kind: USER_ADDRESS_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "clearinghouseState",
        dex_scope: HyperliquidDexScope::OptionalPerpDex,
        doc_url: HYPERLIQUID_INFO_CLEARINGHOUSE_STATE_DOC_URL,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_ACCOUNT_POSITION_CHECKED_AT,
            doc_version: HYPERLIQUID_ACCOUNT_POSITION_DOC_VERSION,
            schema_hash: HYPERLIQUID_ACCOUNT_BALANCE_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_ACCOUNT_BALANCE_FIXTURE_ID,
            parser_test: HYPERLIQUID_ACCOUNT_POSITION_PARSER_TEST,
            request_builder_test: HYPERLIQUID_INFO_OPERATION_REQUEST_TEST,
            auth_kind: USER_ADDRESS_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::Info,
        operation: "spotClearinghouseState",
        dex_scope: HyperliquidDexScope::Spot,
        doc_url: HYPERLIQUID_INFO_SPOT_CLEARINGHOUSE_STATE_DOC_URL,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_INFO_OPERATION_CHECKED_AT,
            doc_version: HYPERLIQUID_SPOT_CLEARINGHOUSE_STATE_DOC_VERSION,
            schema_hash: HYPERLIQUID_SPOT_CLEARINGHOUSE_STATE_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_SPOT_CLEARINGHOUSE_STATE_FIXTURE_ID,
            parser_test: HYPERLIQUID_ACCOUNT_BALANCE_PARSER_TEST,
            request_builder_test: HYPERLIQUID_INFO_OPERATION_REQUEST_TEST,
            auth_kind: USER_ADDRESS_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::WebSocket,
        operation: "allDexsAssetCtxs",
        dex_scope: HyperliquidDexScope::AllPerpDexes,
        doc_url: HYPERLIQUID_WS_SUBSCRIPTIONS_DOC_URL,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_INFO_OPERATION_CHECKED_AT,
            doc_version: HYPERLIQUID_WS_ALL_DEXS_ASSET_CTXS_DOC_VERSION,
            schema_hash: HYPERLIQUID_WS_ALL_DEXS_ASSET_CTXS_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_WS_ALL_DEXS_ASSET_CTXS_FIXTURE_ID,
            parser_test: HYPERLIQUID_OPERATION_FIXTURE_PARSER_TEST,
            request_builder_test: HYPERLIQUID_WS_OPERATION_REQUEST_TEST,
            auth_kind: PUBLIC_AUTH_KIND,
        },
    },
    HyperliquidOperationEvidence {
        transport: HyperliquidOperationTransport::WebSocket,
        operation: "allDexsClearinghouseState",
        dex_scope: HyperliquidDexScope::AllPerpDexes,
        doc_url: HYPERLIQUID_WS_SUBSCRIPTIONS_DOC_URL,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
        meta: EndpointEvidenceMeta {
            checked_at: HYPERLIQUID_INFO_OPERATION_CHECKED_AT,
            doc_version: HYPERLIQUID_WS_ALL_DEXS_CLEARINGHOUSE_DOC_VERSION,
            schema_hash: HYPERLIQUID_WS_ALL_DEXS_CLEARINGHOUSE_SCHEMA_HASH,
            fixture_id: HYPERLIQUID_WS_ALL_DEXS_CLEARINGHOUSE_FIXTURE_ID,
            parser_test: HYPERLIQUID_OPERATION_FIXTURE_PARSER_TEST,
            request_builder_test: HYPERLIQUID_WS_OPERATION_REQUEST_TEST,
            auth_kind: USER_ADDRESS_AUTH_KIND,
        },
    },
];

const UNRECORDED_ENDPOINT_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: UNRECORDED_EVIDENCE_MARKER,
    doc_version: UNRECORDED_EVIDENCE_MARKER,
    schema_hash: UNRECORDED_EVIDENCE_MARKER,
    fixture_id: UNRECORDED_EVIDENCE_MARKER,
    parser_test: UNRECORDED_EVIDENCE_MARKER,
    request_builder_test: UNRECORDED_EVIDENCE_MARKER,
    auth_kind: UNRECORDED_EVIDENCE_MARKER,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VenueSpec {
    pub venue: VenueId,
    pub rest_base: &'static str,
    pub ws_market: Option<&'static str>,
    pub ws_trade: Option<&'static str>,
    pub docs_url: &'static str,
}

pub fn endpoint_weight(exchange: &str, method: HttpMethod, path: &str) -> Option<u32> {
    let venue = VenueId::from_exchange_name(exchange)?;
    ENDPOINT_SPECS
        .iter()
        .filter(|spec| spec.venue == venue && spec.method == method && spec.path == path)
        .map(|spec| spec.weight)
        .max()
}

pub fn endpoint_evidence(
    exchange: &str,
    method: HttpMethod,
    path: &str,
) -> Option<EndpointEvidenceSnapshot> {
    let venue = VenueId::from_exchange_name(exchange)?;
    if venue == VenueId::Hyperliquid && method == HttpMethod::Post && path == "/info" {
        return None;
    }
    let specs = ENDPOINT_SPECS
        .iter()
        .filter(|spec| spec.venue == venue && spec.method == method && spec.path == path)
        .collect::<Vec<_>>();
    let first = specs.first()?;
    let evidence_meta = endpoint_evidence_meta(&specs);
    let weight = specs
        .iter()
        .map(|spec| spec.weight)
        .max()
        .unwrap_or(first.weight);
    Some(EndpointEvidenceSnapshot {
        method: method.as_str().to_owned(),
        path: path.to_owned(),
        checked_at: evidence_meta.checked_at.to_owned(),
        doc_version: evidence_meta.doc_version.to_owned(),
        schema_hash: evidence_meta.schema_hash.to_owned(),
        fixture_id: evidence_meta.fixture_id.to_owned(),
        parser_test: evidence_meta.parser_test.to_owned(),
        request_builder_test: evidence_meta.request_builder_test.to_owned(),
        auth_kind: evidence_meta.auth_kind.to_owned(),
        doc_urls: unique_strings(specs.iter().map(|spec| spec.doc_url)),
        use_cases: unique_strings(specs.iter().map(|spec| spec.use_case.as_str())),
        data_kinds: unique_strings(specs.iter().map(|spec| spec.data_kind.as_str())),
        rate_scopes: unique_strings(specs.iter().map(|spec| spec.rate_scope.as_str())),
        weight,
    })
}

pub(crate) fn endpoint_evidence_for_spec(spec: &EndpointSpec) -> EndpointEvidenceSnapshot {
    let evidence_meta = endpoint_evidence_meta_for_spec(spec);
    EndpointEvidenceSnapshot {
        method: spec.method.as_str().to_owned(),
        path: spec.path.to_owned(),
        checked_at: evidence_meta.checked_at.to_owned(),
        doc_version: evidence_meta.doc_version.to_owned(),
        schema_hash: evidence_meta.schema_hash.to_owned(),
        fixture_id: evidence_meta.fixture_id.to_owned(),
        parser_test: evidence_meta.parser_test.to_owned(),
        request_builder_test: evidence_meta.request_builder_test.to_owned(),
        auth_kind: evidence_meta.auth_kind.to_owned(),
        doc_urls: vec![spec.doc_url.to_owned()],
        use_cases: vec![spec.use_case.as_str().to_owned()],
        data_kinds: vec![spec.data_kind.as_str().to_owned()],
        rate_scopes: vec![spec.rate_scope.as_str().to_owned()],
        weight: spec.weight,
    }
}

fn endpoint_evidence_meta_for_spec(spec: &EndpointSpec) -> EndpointEvidenceMeta {
    HYPERLIQUID_OPERATION_EVIDENCE
        .iter()
        .find(|entry| entry.matches_endpoint_spec(spec))
        .map(|entry| entry.meta)
        .or_else(|| {
            RECORDED_ENDPOINT_EVIDENCE
                .iter()
                .find(|entry| entry.matches(spec))
                .map(|entry| entry.meta)
        })
        .unwrap_or(UNRECORDED_ENDPOINT_EVIDENCE)
}

fn endpoint_evidence_meta(specs: &[&EndpointSpec]) -> EndpointEvidenceMeta {
    specs
        .iter()
        .find_map(|spec| {
            RECORDED_ENDPOINT_EVIDENCE
                .iter()
                .find(|entry| entry.matches(spec))
                .map(|entry| entry.meta)
        })
        .unwrap_or(UNRECORDED_ENDPOINT_EVIDENCE)
}

fn unique_strings<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        if !out.iter().any(|seen| seen == value) {
            out.push(value.to_owned());
        }
    }
    out
}

pub const VENUE_SPECS: &[VenueSpec] = &[
    VenueSpec {
        venue: VenueId::Binance,
        rest_base: "https://fapi.binance.com",
        ws_market: Some("wss://fstream.binance.com/public/ws"),
        ws_trade: Some("wss://ws-fapi.binance.com/ws-fapi/v1"),
        docs_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures",
    },
    VenueSpec {
        venue: VenueId::Okx,
        rest_base: "https://openapi.okx.com",
        ws_market: Some("wss://ws.okx.com:8443/ws/v5/public"),
        ws_trade: Some("wss://ws.okx.com:8443/ws/v5/private"),
        docs_url: "https://www.okx.com/docs-v5/en/",
    },
    VenueSpec {
        venue: VenueId::Bybit,
        rest_base: "https://api.bybit.com",
        ws_market: Some("wss://stream.bybit.com/v5/public/linear"),
        ws_trade: Some("wss://stream.bybit.com/v5/trade"),
        docs_url: "https://bybit-exchange.github.io/docs/v5/intro",
    },
    VenueSpec {
        venue: VenueId::Bitget,
        rest_base: "https://api.bitget.com",
        ws_market: Some("wss://ws.bitget.com/v3/ws/public"),
        ws_trade: Some("wss://ws.bitget.com/v3/ws/private"),
        docs_url: "https://www.bitget.com/api-doc/uta/guide",
    },
    VenueSpec {
        venue: VenueId::Gate,
        rest_base: "https://api.gateio.ws",
        ws_market: Some("wss://fx-ws.gateio.ws/v4/ws/usdt"),
        ws_trade: Some("wss://fx-ws.gateio.ws/v4/ws/usdt"),
        docs_url: "https://www.gate.com/docs/developers/futures/",
    },
    VenueSpec {
        venue: VenueId::GateCrossEx,
        rest_base: "https://api.gateio.ws/api/v4",
        ws_market: Some("wss://api.gateio.ws/ws/crossex/public"),
        ws_trade: Some("wss://api.gateio.ws/ws/crossex"),
        docs_url: "https://www.gate.com/docs/developers/crossex/en/",
    },
    VenueSpec {
        venue: VenueId::Kucoin,
        rest_base: "https://api-futures.kucoin.com",
        ws_market: None,
        ws_trade: None,
        docs_url: "https://www.kucoin.com/docs-new",
    },
    VenueSpec {
        venue: VenueId::Hyperliquid,
        rest_base: "https://api.hyperliquid.xyz",
        ws_market: Some("wss://api.hyperliquid.xyz/ws"),
        ws_trade: Some("wss://api.hyperliquid.xyz/ws"),
        docs_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api",
    },
    VenueSpec {
        venue: VenueId::Kraken,
        rest_base: "https://api.kraken.com",
        ws_market: Some("wss://ws.kraken.com/v2"),
        ws_trade: Some("wss://ws-auth.kraken.com/v2"),
        docs_url: "https://docs.kraken.com/api/",
    },
];

const BINANCE_USDM_PLACE_ORDER_CHECKED_AT: &str = "2026-06-29";
const BINANCE_USDM_PLACE_ORDER_DOC_VERSION: &str = "binance-usdm-futures-new-order-2026-06-29";
const BINANCE_USDM_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:5f25d9d42028c74f8141427ae560db29132f8e6d7b5c27b706806a7f0c734cf0";
const BINANCE_USDM_PLACE_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_place_order_ack.json";
const BINANCE_USDM_PLACE_ORDER_PARSER_TEST: &str =
    "binance_place_order_ack_parses_official_fixture";
const BINANCE_USDM_PLACE_ORDER_REQUEST_TEST: &str = "rest_limit_order_params_match_binance_schema";

const BINANCE_USDM_ORDER_TEST_CHECKED_AT: &str = "2026-07-07";
const BINANCE_USDM_ORDER_TEST_DOC_VERSION: &str = "binance-usdm-futures-test-new-order-2026-07-07";
const BINANCE_USDM_ORDER_TEST_REQUEST_TEST: &str =
    "crates/exchange/src/adapters/binance_private_rest.rs::test_order_uses_official_non_matching_endpoint";

const BINANCE_USDM_CANCEL_ORDER_CHECKED_AT: &str = "2026-07-02";
const BINANCE_USDM_CANCEL_ORDER_DOC_VERSION: &str = "binance-usdm-futures-cancel-order-2026-07-02";
const BINANCE_USDM_CANCEL_ORDER_SCHEMA_HASH: &str =
    "sha256:2af0ef4d430ce0c6a1bd424933d396d89c779afb17046eb7f9aac14033bb0d3e";
const BINANCE_USDM_CANCEL_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_cancel_order_ack.json";
const BINANCE_USDM_CANCEL_ORDER_PARSER_TEST: &str =
    "binance_cancel_order_ack_parses_official_fixture";
const BINANCE_USDM_CANCEL_ORDER_REQUEST_TEST: &str = "cancel_params_use_orig_client_order_id";

const BINANCE_USDM_POSITION_MODE_CHECKED_AT: &str = "2026-07-02";
const BINANCE_USDM_POSITION_MODE_DOC_VERSION: &str =
    "binance-usdm-futures-get-current-position-mode-2026-07-02";
const BINANCE_USDM_POSITION_MODE_SCHEMA_HASH: &str =
    "sha256:3b2d52fc2030034b04019a7cbb5f371fe8d89aa764c4a1de5e7e761dba0c5a7f";
const BINANCE_USDM_POSITION_MODE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_position_side_dual.json";
const BINANCE_USDM_POSITION_MODE_PARSER_TEST: &str =
    "binance_position_mode_parses_official_fixture";
const BINANCE_USDM_POSITION_MODE_REQUEST_TEST: &str =
    "exchange_account_mode_reads_official_position_side_dual_endpoint";

const OKX_PLACE_ORDER_CHECKED_AT: &str = "2026-06-29";
const OKX_PLACE_ORDER_DOC_VERSION: &str = "okx-v5-trade-place-order-2026-06-29";
const OKX_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:60311d35248b9fc2385a2181f695c6d0d82d6739a1587ea6ad0e02406e0be4ef";
const OKX_PLACE_ORDER_FIXTURE_ID: &str = "crates/exchange/fixtures/okx/trade_place_order_ack.json";
const OKX_PLACE_ORDER_PARSER_TEST: &str = "okx_place_order_ack_parses_official_fixture";
const OKX_PLACE_ORDER_REQUEST_TEST: &str = "place_order_arg_uses_configured_td_mode";

const OKX_ORDER_PRECHECK_CHECKED_AT: &str = "2026-07-07";
const OKX_ORDER_PRECHECK_DOC_VERSION: &str = "okx-v5-trade-order-precheck-2026-07-07";
const OKX_ORDER_PRECHECK_REQUEST_TEST: &str =
    "pre_check_order_uses_official_pre_check_endpoint_and_body";

const OKX_CANCEL_ORDER_CHECKED_AT: &str = "2026-07-02";
const OKX_CANCEL_ORDER_DOC_VERSION: &str = "okx-v5-trade-cancel-order-2026-07-02";
const OKX_CANCEL_ORDER_SCHEMA_HASH: &str =
    "sha256:89dfd90d94b74305ab38077f311d50c7adf51f07ffd64a740398829b4f839dad";
const OKX_CANCEL_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/okx/trade_cancel_order_ack.json";
const OKX_CANCEL_ORDER_PARSER_TEST: &str = "okx_cancel_order_ack_parses_official_fixture";
const OKX_CANCEL_ORDER_REQUEST_TEST: &str =
    "live_cancel_order_uses_official_cancel_order_path_and_client_order_id";

const OKX_GET_ORDER_CHECKED_AT: &str = "2026-06-30";
const OKX_GET_ORDER_DOC_VERSION: &str = "okx-v5-trade-get-order-details-2026-06-30";
const OKX_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:ab130bf25d597425e16841477e61fac0b5d4a345eb305d340bcec40bd82392b1";
const OKX_GET_ORDER_FIXTURE_ID: &str = "crates/exchange/fixtures/okx/trade_get_order_filled.json";
const OKX_GET_ORDER_PARSER_TEST: &str = "okx_get_order_parses_official_fixture";
const OKX_GET_ORDER_REQUEST_TEST: &str = "live_get_order_empty_success_is_none";
const OKX_OPEN_ORDERS_CHECKED_AT: &str = "2026-07-02";
const OKX_OPEN_ORDERS_DOC_VERSION: &str = "okx-v5-trade-get-order-list-2026-07-02";
const OKX_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:b3c992124a74c65a35d662bc64713dc412ed1b4c08592cdb4f5255424a6f9cbf";
const OKX_OPEN_ORDERS_FIXTURE_ID: &str = "crates/exchange/fixtures/okx/trade_orders_pending.json";
const OKX_OPEN_ORDERS_PARSER_TEST: &str = "okx_open_orders_parses_official_fixture";
const OKX_OPEN_ORDERS_REQUEST_TEST: &str =
    "live_open_orders_queries_orders_pending_with_signed_headers";

const OKX_ACCOUNT_CONFIG_CHECKED_AT: &str = "2026-07-02";
const OKX_ACCOUNT_CONFIG_DOC_VERSION: &str =
    "okx-v5-trading-account-get-account-configuration-2026-07-02";
const OKX_ACCOUNT_CONFIG_SCHEMA_HASH: &str =
    "sha256:d8967eefd2b0a5a88960be4c41768d392b866c572e027021797eb16fd3a01aa5";
const OKX_ACCOUNT_CONFIG_FIXTURE_ID: &str =
    "crates/exchange/fixtures/okx/account_config_long_short.json";
const OKX_ACCOUNT_CONFIG_PARSER_TEST: &str =
    "okx_account_config_parses_official_fixture_position_mode";
const OKX_ACCOUNT_CONFIG_REQUEST_TEST: &str =
    "okx_account_mode_reads_signed_account_config_without_demo_header";
const OKX_ACCOUNT_BALANCE_CHECKED_AT: &str = "2026-07-02";
const OKX_ACCOUNT_BALANCE_DOC_VERSION: &str = "okx-v5-trading-account-get-balance-2026-07-02";
const OKX_ACCOUNT_BALANCE_SCHEMA_HASH: &str =
    "sha256:5b845155b110daa670b43b945d0302f0c9fea4a54e373187d15f4c2559a8bb7f";
const OKX_ACCOUNT_BALANCE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/okx/account_balance_usdt.json";
const OKX_ACCOUNT_BALANCE_PARSER_TEST: &str = "okx_account_balance_parses_official_fixture";
const OKX_ACCOUNT_BALANCE_REQUEST_TEST: &str = "get_balance_sends_all_signed_headers";
const OKX_ACCOUNT_POSITIONS_CHECKED_AT: &str = "2026-07-02";
const OKX_ACCOUNT_POSITIONS_DOC_VERSION: &str = "okx-v5-trading-account-get-positions-2026-07-02";
const OKX_ACCOUNT_POSITIONS_SCHEMA_HASH: &str =
    "sha256:1e41e1e84e43736189c5614466e955494acae539e11dcc0fbafe33199b3ef03e";
const OKX_ACCOUNT_POSITIONS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/okx/account_positions_swap.json";
const OKX_ACCOUNT_POSITIONS_PARSER_TEST: &str = "okx_positions_parse_official_fixture";
const OKX_ACCOUNT_POSITIONS_REQUEST_TEST: &str =
    "live_positions_queries_account_positions_with_signed_headers";

const BYBIT_PLACE_ORDER_CHECKED_AT: &str = "2026-06-29";
const BYBIT_PLACE_ORDER_DOC_VERSION: &str = "bybit-v5-order-create-2026-06-29";
const BYBIT_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:d9d397e431457b054abd260a76b5a2261c4bc21abb673b70826a20165ffff65e";
const BYBIT_PLACE_ORDER_FIXTURE_ID: &str = "crates/exchange/fixtures/bybit/order_create_ack.json";
const BYBIT_PLACE_ORDER_PARSER_TEST: &str = "bybit_place_order_ack_parses_official_fixture";
const BYBIT_PLACE_ORDER_REQUEST_TEST: &str = "limit_order_uses_gtc_and_price";

const BYBIT_ORDER_PRECHECK_CHECKED_AT: &str = "2026-07-07";
const BYBIT_ORDER_PRECHECK_DOC_VERSION: &str = "bybit-v5-order-pre-check-2026-07-07";
const BYBIT_ORDER_PRECHECK_REQUEST_TEST: &str = "pre_check_order_uses_official_pre_check_endpoint";

const BYBIT_CANCEL_ORDER_CHECKED_AT: &str = "2026-07-02";
const BYBIT_CANCEL_ORDER_DOC_VERSION: &str = "bybit-v5-order-cancel-2026-07-02";
const BYBIT_CANCEL_ORDER_SCHEMA_HASH: &str =
    "sha256:9e66839ea77befc4089a173c25a707a41c95d636e671d02eb134876be9e3b32e";
const BYBIT_CANCEL_ORDER_FIXTURE_ID: &str = "crates/exchange/fixtures/bybit/order_cancel_ack.json";
const BYBIT_CANCEL_ORDER_PARSER_TEST: &str = "bybit_cancel_order_ack_parses_official_fixture";
const BYBIT_CANCEL_ORDER_REQUEST_TEST: &str =
    "crates/exchange/tests/bybit_test.rs::live_cancel_order_returns_cancel_requested";

const BYBIT_GET_ORDER_CHECKED_AT: &str = "2026-06-30";
const BYBIT_GET_ORDER_DOC_VERSION: &str = "bybit-v5-get-open-closed-orders-2026-06-30";
const BYBIT_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:62fb1814eb2c0fd419439dc5d123cca08da410f6c2151215ed10ab22f83b650a";
const BYBIT_GET_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bybit/order_realtime_linear_open.json";
const BYBIT_GET_ORDER_PARSER_TEST: &str = "bybit_get_order_parses_official_fixture";
const BYBIT_GET_ORDER_REQUEST_TEST: &str = "live_get_order_queries_realtime_by_order_link_id";

const BYBIT_POSITION_MODE_CHECKED_AT: &str = "2026-07-02";
const BYBIT_POSITION_MODE_DOC_VERSION: &str = "bybit-v5-get-position-info-2026-07-02";
const BYBIT_POSITION_MODE_SCHEMA_HASH: &str =
    "sha256:3acee570e8c4c95192898d7a9da073a93f08394e860d0f912155b6a88b571de1";
const BYBIT_POSITION_MODE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bybit/position_list_one_way.json";
const BYBIT_POSITION_MODE_PARSER_TEST: &str = "bybit_position_mode_parses_official_fixture";
const BYBIT_POSITION_MODE_REQUEST_TEST: &str = "live_account_mode_reads_one_way_position_idx";
const BYBIT_POSITIONS_CHECKED_AT: &str = "2026-07-02";
const BYBIT_POSITIONS_DOC_VERSION: &str = "bybit-v5-get-position-info-account-position-2026-07-02";
const BYBIT_POSITIONS_SCHEMA_HASH: &str =
    "sha256:1d9b41c78476221951fae10cb30559e9e5e2d848d6b64bb1999c4580a9c35b1e";
const BYBIT_POSITIONS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bybit/position_list_linear_open.json";
const BYBIT_POSITIONS_PARSER_TEST: &str = "bybit_positions_parse_official_fixture";
const BYBIT_POSITIONS_REQUEST_TEST: &str = "private_account_reads_fan_out_usdt_and_usdc_settles";

const BITGET_PLACE_ORDER_CHECKED_AT: &str = "2026-06-29";
const BITGET_PLACE_ORDER_DOC_VERSION: &str = "bitget-uta-trade-place-order-2026-06-29";
const BITGET_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:22dc071f603168741b8d5e8f393f5838a5286a992286bc0369c0b7e4a1b33932";
const BITGET_PLACE_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_place_order_ack.json";
const BITGET_PLACE_ORDER_PARSER_TEST: &str = "bitget_place_order_ack_parses_official_fixture";
const BITGET_PLACE_ORDER_REQUEST_TEST: &str = "place_order_body_uses_time_in_force_and_v3_category";

const BITGET_CANCEL_ORDER_CHECKED_AT: &str = "2026-07-02";
const BITGET_CANCEL_ORDER_DOC_VERSION: &str = "bitget-uta-trade-cancel-order-2026-07-02";
const BITGET_CANCEL_ORDER_SCHEMA_HASH: &str =
    "sha256:bd083965f8d4e03c3fa9cf64d2aacdc8d6337ed763d2d7f050c9cb7a087db348";
const BITGET_CANCEL_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_cancel_order_ack.json";
const BITGET_CANCEL_ORDER_PARSER_TEST: &str = "bitget_cancel_order_ack_parses_official_fixture";
const BITGET_CANCEL_ORDER_REQUEST_TEST: &str =
    "crates/exchange/tests/bitget_test.rs::live_cancel_order_returns_cancel_requested";

const BITGET_GET_ORDER_CHECKED_AT: &str = "2026-06-30";
const BITGET_GET_ORDER_DOC_VERSION: &str = "bitget-uta-trade-get-order-details-2026-06-30";
const BITGET_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:79653caccf87a5bab7cd986acd4046a6fd2dccee61d346a636e5249de51f6ddd";
const BITGET_GET_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_order_info_filled.json";
const BITGET_GET_ORDER_PARSER_TEST: &str = "bitget_get_order_parses_official_fixture";
const BITGET_GET_ORDER_REQUEST_TEST: &str = "live_get_order_queries_detail_by_client_oid";

const BITGET_ACCOUNT_ASSETS_CHECKED_AT: &str = "2026-07-02";
const BITGET_ACCOUNT_ASSETS_DOC_VERSION: &str = "bitget-uta-account-get-account-assets-2026-07-02";
const BITGET_ACCOUNT_ASSETS_SCHEMA_HASH: &str =
    "sha256:60f1410d2d8d70b4a1e8a603250d6f60b515bfda4bff218bc1274b25291e86c2";
const BITGET_ACCOUNT_ASSETS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_account_assets.json";
const BITGET_ACCOUNT_ASSETS_PARSER_TEST: &str = "bitget_account_assets_parses_official_fixture";
const BITGET_ACCOUNT_ASSETS_REQUEST_TEST: &str =
    "crates/exchange/tests/bitget_test.rs::balance_signed_headers";
const BITGET_CURRENT_POSITION_CHECKED_AT: &str = "2026-07-02";
const BITGET_CURRENT_POSITION_DOC_VERSION: &str =
    "bitget-uta-trade-get-current-position-2026-07-02";
const BITGET_CURRENT_POSITION_SCHEMA_HASH: &str =
    "sha256:22ead2a4c3fe6a979be913b9db5e75bedb8a9e6c8170b9c437d30b0d5731f0fb";
const BITGET_CURRENT_POSITION_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_current_position_btcusdt.json";
const BITGET_CURRENT_POSITION_PARSER_TEST: &str = "bitget_current_position_parses_official_fixture";
const BITGET_CURRENT_POSITION_REQUEST_TEST: &str =
    "live_positions_queries_current_position_with_signed_headers";
const BITGET_OPEN_ORDERS_CHECKED_AT: &str = "2026-07-02";
const BITGET_OPEN_ORDERS_DOC_VERSION: &str = "bitget-uta-trade-get-open-orders-2026-07-02";
const BITGET_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:db28d856fe017bc4cb9b8609a8cae12fbef7c85367db6aef90ce7bfe0eae8d13";
const BITGET_OPEN_ORDERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bitget/uta_unfilled_orders_open.json";
const BITGET_OPEN_ORDERS_PARSER_TEST: &str = "bitget_unfilled_orders_parses_official_fixture";
const BITGET_OPEN_ORDERS_REQUEST_TEST: &str =
    "live_open_orders_queries_unfilled_orders_with_v3_category";

const BINANCE_USDM_BALANCE_CHECKED_AT: &str = "2026-07-30";
const BINANCE_USDM_BALANCE_DOC_VERSION: &str = "binance-usdm-futures-account-balance-v3-2026-07-30";
const BINANCE_USDM_BALANCE_SCHEMA_HASH: &str =
    "sha256:8308340728d96036dd921af43a27acbaeef83df4b990109f762e29143bbe9b01";
const BINANCE_USDM_BALANCE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_balance_v3.json";
const BINANCE_USDM_BALANCE_PARSER_TEST: &str = "binance_balance_parses_official_fixture";
const BINANCE_USDM_BALANCE_REQUEST_TEST: &str = "balance_signed_request_with_api_key_header";
const BINANCE_USDM_POSITIONS_CHECKED_AT: &str = "2026-07-30";
const BINANCE_USDM_POSITIONS_DOC_VERSION: &str =
    "binance-usdm-futures-position-information-v3-2026-07-30";
const BINANCE_USDM_POSITIONS_SCHEMA_HASH: &str =
    "sha256:bb7a83b3f7839bc2a6fb38413b6df9a17a31ebdb56e61216e2e1805c34d1c177";
const BINANCE_USDM_POSITIONS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_position_risk_btcusdt.json";
const BINANCE_USDM_POSITIONS_PARSER_TEST: &str = "binance_positions_parse_official_fixture";
const BINANCE_USDM_POSITIONS_REQUEST_TEST: &str = "live_positions_queries_signed_position_risk";

const BINANCE_USDM_GET_ORDER_CHECKED_AT: &str = "2026-06-29";
const BINANCE_USDM_GET_ORDER_DOC_VERSION: &str = "binance-usdm-futures-query-order-2026-06-29";
const BINANCE_USDM_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:f6f5edac0707319bbd976905f6d86cc20d291a667379f4abf68555dcd682999f";
const BINANCE_USDM_GET_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_get_order_filled.json";
const BINANCE_USDM_GET_ORDER_PARSER_TEST: &str = "binance_get_order_parses_official_fixture";
const BINANCE_USDM_GET_ORDER_REQUEST_TEST: &str = "live_get_order_returns_order_info";
const BINANCE_USDM_OPEN_ORDERS_CHECKED_AT: &str = "2026-07-02";
const BINANCE_USDM_OPEN_ORDERS_DOC_VERSION: &str =
    "binance-usdm-futures-current-all-open-orders-2026-07-02";
const BINANCE_USDM_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:3e60dbbbcdd7950be4f1c2d61c5f5031b6344806bbd7cfa81ca1b244a2b1680b";
const BINANCE_USDM_OPEN_ORDERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/binance/usdm_open_orders.json";
const BINANCE_USDM_OPEN_ORDERS_PARSER_TEST: &str = "binance_open_orders_parses_official_fixture";
const BINANCE_USDM_OPEN_ORDERS_REQUEST_TEST: &str =
    "live_open_orders_queries_signed_current_open_orders";

const BYBIT_WALLET_BALANCE_CHECKED_AT: &str = "2026-07-13";
const BYBIT_WALLET_BALANCE_DOC_VERSION: &str = "bybit-v5-wallet-balance-2026-07-13";
const BYBIT_WALLET_BALANCE_SCHEMA_HASH: &str =
    "sha256:39775d9331b6ba06d3ff435d78ecb62acbf544c13474e498545b19f414a9751f";
const BYBIT_WALLET_BALANCE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/bybit/wallet_balance_unified_account_metrics.json";
const BYBIT_WALLET_BALANCE_PARSER_TEST: &str =
    "bybit_account_summary_parses_equity_margin_rates_and_source";
const BYBIT_WALLET_BALANCE_REQUEST_TEST: &str =
    "crates/exchange/tests/bybit_test.rs::balance_signed_headers";

const GATE_GET_ORDER_CHECKED_AT: &str = "2026-06-30";
const GATE_GET_ORDER_DOC_VERSION: &str = "gate-apiv4-get-a-single-order-2026-06-30";
const GATE_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:10ea597b450b7e1d2e19c44021869c5df63f7b322dd969225232c685c2c62f69";
const GATE_GET_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/futures_usdt_get_order_filled.json";
const GATE_GET_ORDER_PARSER_TEST: &str = "gate_get_order_parses_official_fixture";
const GATE_GET_ORDER_REQUEST_TEST: &str = "get_order_uses_official_order_detail_by_gate_text";
const GATE_MY_TRADES_CHECKED_AT: &str = "2026-07-11";
const GATE_MY_TRADES_DOC_VERSION: &str =
    "gate-apiv4-v4.105.32-query-personal-trading-records-2026-07-11";
const GATE_MY_TRADES_SCHEMA_HASH: &str =
    "sha256:5fc511e1c98816438ef07b39f8c1e9bdb1f3d911d8244605aa07a49e8b1412b9";
const GATE_MY_TRADES_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/futures_usdt_my_trades_order.json";
const GATE_MY_TRADES_PARSER_TEST: &str =
    "parses_official_my_trades_fixture_without_combining_fee_units";
const GATE_MY_TRADES_REQUEST_TEST: &str = "my_trades_uses_signed_order_query_and_official_fixture";
const GATE_FUTURES_FEE_CHECKED_AT: &str = "2026-07-11";
const GATE_FUTURES_FEE_DOC_VERSION: &str =
    "gate-apiv4-v4.105.32-query-futures-market-trading-fee-rates-2026-07-11";
const GATE_FUTURES_FEE_SCHEMA_HASH: &str =
    "sha256:8ad7680d9bbf1abf952dfb6158ab84a169a04c3fd580da1197613c504bdc5bcb";
const GATE_FUTURES_FEE_FIXTURE_ID: &str = "crates/exchange/fixtures/gate/futures_usdt_fee.json";
const GATE_FUTURES_FEE_PARSER_TEST: &str = "parses_official_fee_fixture_and_preserves_maker_rebate";
const GATE_FUTURES_FEE_REQUEST_TEST: &str = "fee_read_uses_signed_endpoint_and_preserves_rebate";

const GATE_CANCEL_ORDER_CHECKED_AT: &str = "2026-07-07";
const GATE_CANCEL_ORDER_DOC_VERSION: &str = "gate-apiv4-cancel-a-single-order-2026-07-07";
const GATE_CANCEL_ORDER_REQUEST_TEST: &str =
    "safe_cancel_no_match_probe_uses_official_single_cancel_without_live_writes";

const GATE_OPEN_ORDERS_CHECKED_AT: &str = "2026-07-02";
const GATE_OPEN_ORDERS_DOC_VERSION: &str = "gate-apiv4-list-futures-orders-2026-07-02";
const GATE_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:b84fd51489451c265e0789b8d4416db9ac8a9f7d462b2ec2280f5c835b0e0707";
const GATE_OPEN_ORDERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/futures_usdt_orders_open.json";
const GATE_OPEN_ORDERS_PARSER_TEST: &str = "gate_open_orders_parses_official_fixture";
const GATE_OPEN_ORDERS_REQUEST_TEST: &str = "open_orders_rejects_malformed_order_row";
const GATE_ACCOUNT_BALANCE_CHECKED_AT: &str = "2026-07-02";
const GATE_ACCOUNT_BALANCE_DOC_VERSION: &str = "gate-apiv4-get-futures-account-2026-07-02";
const GATE_ACCOUNT_BALANCE_SCHEMA_HASH: &str =
    "sha256:aae3ae7417405ed39b1c0aefef27736dd0d04be06c03959c0df7aeed99777fd2";
const GATE_ACCOUNT_BALANCE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/futures_usdt_account.json";
const GATE_ACCOUNT_BALANCE_PARSER_TEST: &str =
    "gate_futures_account_balance_parses_official_fixture";
const GATE_ACCOUNT_BALANCE_REQUEST_TEST: &str =
    "crates/exchange/tests/gate_test.rs::balance_signed_headers";
const GATE_POSITIONS_CHECKED_AT: &str = "2026-07-02";
const GATE_POSITIONS_DOC_VERSION: &str = "gate-apiv4-list-futures-positions-2026-07-02";
const GATE_POSITIONS_SCHEMA_HASH: &str =
    "sha256:8570896ffc84c2af96e35e62f4311f70f8876504aa8c6a52be3c13e8589e06cd";
const GATE_POSITIONS_FIXTURE_ID: &str = "crates/exchange/fixtures/gate/futures_usdt_positions.json";
const GATE_POSITIONS_PARSER_TEST: &str = "gate_positions_parse_official_fixture";
const GATE_POSITIONS_REQUEST_TEST: &str = "positions_signed_size_to_long_short";

const KUCOIN_PLACE_ORDER_CHECKED_AT: &str = "2026-06-29";
const KUCOIN_PLACE_ORDER_DOC_VERSION: &str = "kucoin-futures-place-order-2026-06-29";
const KUCOIN_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:28326340f23fda8ba48e0ab15bbdd23f32bfe5e4467b525e923bdeb3b3cdf090";
const KUCOIN_PLACE_ORDER_FIXTURE_ID: &str = "crates/exchange/fixtures/kucoin/place_order_ack.json";
const KUCOIN_PLACE_ORDER_PARSER_TEST: &str = "kucoin_place_order_ack_parses_official_fixture";
const KUCOIN_PLACE_ORDER_REQUEST_TEST: &str =
    "crates/exchange/src/adapters/kucoin_trade_data_tests.rs::limit_order_matches_official_body_shape";

const KUCOIN_ORDER_TEST_CHECKED_AT: &str = "2026-07-07";
const KUCOIN_ORDER_TEST_DOC_VERSION: &str = "kucoin-futures-add-order-test-2026-07-07";
const KUCOIN_ORDER_TEST_REQUEST_TEST: &str =
    "crates/exchange/src/adapters/kucoin_private_rest.rs::test_order_uses_official_non_matching_endpoint";

const KUCOIN_CANCEL_ORDER_CHECKED_AT: &str = "2026-07-02";
const KUCOIN_CANCEL_ORDER_DOC_VERSION: &str = "kucoin-futures-cancel-order-by-clientoid-2026-07-02";
const KUCOIN_CANCEL_ORDER_SCHEMA_HASH: &str =
    "sha256:b239aa651f0d267fe9ee6de16ec0f77200763a86fb8a157bfc31351a9de7db5e";
const KUCOIN_CANCEL_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/cancel_order_by_client_oid_ack.json";
const KUCOIN_CANCEL_ORDER_PARSER_TEST: &str = "kucoin_cancel_order_ack_parses_official_fixture";
const KUCOIN_CANCEL_ORDER_REQUEST_TEST: &str = "live_cancel_order_by_client_oid";
const KUCOIN_CANCEL_BY_ORDER_ID_CHECKED_AT: &str = "2026-07-11";
const KUCOIN_CANCEL_BY_ORDER_ID_DOC_VERSION: &str =
    "kucoin-futures-cancel-order-by-order-id-2026-07-11";
const KUCOIN_CANCEL_BY_ORDER_ID_REQUEST_TEST: &str =
    "crates/exchange/src/adapters/kucoin_private_rest.rs::cancel_target_prefers_exchange_order_id_then_client_oid_fallback";

const KUCOIN_FILLS_CHECKED_AT: &str = "2026-07-11";
const KUCOIN_FILLS_DOC_VERSION: &str = "kucoin-futures-get-trade-history-2026-07-11";
const KUCOIN_FILLS_SCHEMA_HASH: &str =
    "sha256:0d9814f361e3636aaa92f17c525dbc75081a65f952cf8e888b9e45d8dd7ce985";
const KUCOIN_FILLS_FIXTURE_ID: &str = "crates/exchange/fixtures/kucoin/fills_by_order_id.json";
const KUCOIN_FILLS_PARSER_TEST: &str = "kucoin_fills_parse_official_fixture_without_defaulting_fee";
const KUCOIN_FILLS_REQUEST_TEST: &str =
    "crates/exchange/src/adapters/kucoin_private_rest.rs::fills_read_uses_signed_order_query_and_official_fixture";

const KUCOIN_FEE_RATE_CHECKED_AT: &str = "2026-07-11";
const KUCOIN_FEE_RATE_DOC_VERSION: &str = "kucoin-futures-get-actual-fee-2026-07-11";
const KUCOIN_FEE_RATE_SCHEMA_HASH: &str =
    "sha256:118317e838e199c08f627b38c4ba95a1870ef5807379d5c4b21afef2f856047d";
const KUCOIN_FEE_RATE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/futures_actual_fee_xbtusdtm.json";
const KUCOIN_FEE_RATE_PARSER_TEST: &str = "kucoin_fee_rate_parses_official_fixture_and_source_url";
const KUCOIN_FEE_RATE_REQUEST_TEST: &str =
    "crates/exchange/src/adapters/kucoin_private_rest.rs::fee_rate_read_uses_signed_symbol_query_and_official_fixture";

const GATE_PLACE_ORDER_CHECKED_AT: &str = "2026-06-30";
const GATE_PLACE_ORDER_DOC_VERSION: &str = "gate-apiv4-create-futures-order-2026-06-30";
const GATE_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:e803326821f11c9e143477a1bbd2d1d65ccb86c07405dea79cd05cce75cb3f89";
const GATE_PLACE_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/gate/futures_place_order_ack.json";
const GATE_PLACE_ORDER_PARSER_TEST: &str = "gate_place_order_ack_parses_official_fixture";
const GATE_PLACE_ORDER_REQUEST_TEST: &str = "place_order_converts_base_qty_to_contracts";

const KUCOIN_GET_ORDER_CHECKED_AT: &str = "2026-06-30";
const KUCOIN_GET_ORDER_DOC_VERSION: &str = "kucoin-futures-get-order-by-clientoid-2026-06-30";
const KUCOIN_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:43f0847e8fa858abc15901801e2a811376d18f74b61c9dd4794cf378bd3ecf1a";
const KUCOIN_GET_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/get_order_by_client_oid_open.json";
const KUCOIN_GET_ORDER_PARSER_TEST: &str = "kucoin_get_order_by_client_oid_parses_official_fixture";
const KUCOIN_GET_ORDER_REQUEST_TEST: &str = "live_get_order_uses_official_by_client_oid_query";

const KUCOIN_OPEN_ORDERS_CHECKED_AT: &str = "2026-07-02";
const KUCOIN_OPEN_ORDERS_DOC_VERSION: &str = "kucoin-futures-get-order-list-2026-07-02";
const KUCOIN_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:e8a3c1e65c542c75b10b6881072b5547bc8bc3513f127fc8bbd98cfff5b3c679";
const KUCOIN_OPEN_ORDERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/get_order_list_active.json";
const KUCOIN_OPEN_ORDERS_PARSER_TEST: &str = "kucoin_open_orders_parses_official_fixture";
const KUCOIN_OPEN_ORDERS_REQUEST_TEST: &str = "live_open_orders_rejects_malformed_order_row";
const KUCOIN_ACCOUNT_OVERVIEW_CHECKED_AT: &str = "2026-07-02";
const KUCOIN_ACCOUNT_OVERVIEW_DOC_VERSION: &str = "kucoin-futures-get-account-futures-2026-07-02";
const KUCOIN_ACCOUNT_OVERVIEW_SCHEMA_HASH: &str =
    "sha256:22eb8516645041f936a6881790de0e4b24bc5758a0c6ebcb14dceb729698f01d";
const KUCOIN_ACCOUNT_OVERVIEW_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/account_overview_usdt.json";
const KUCOIN_ACCOUNT_OVERVIEW_PARSER_TEST: &str = "kucoin_account_overview_parses_official_fixture";
const KUCOIN_ACCOUNT_OVERVIEW_REQUEST_TEST: &str = "balance_signed_with_encrypted_passphrase";
const KUCOIN_POSITIONS_CHECKED_AT: &str = "2026-07-02";
const KUCOIN_POSITIONS_DOC_VERSION: &str = "kucoin-futures-get-position-list-2026-07-02";
const KUCOIN_POSITIONS_SCHEMA_HASH: &str =
    "sha256:2f64271367f6b18fe62f98cb3c6ce0e001c49420fd60912dbe88dcb2b35cb767";
const KUCOIN_POSITIONS_FIXTURE_ID: &str = "crates/exchange/fixtures/kucoin/positions_open.json";
const KUCOIN_POSITIONS_PARSER_TEST: &str = "kucoin_positions_parse_official_fixture";
const KUCOIN_POSITIONS_REQUEST_TEST: &str = "positions_filter_open_and_signed_qty";

const KUCOIN_POSITION_MODE_CHECKED_AT: &str = "2026-07-02";
const KUCOIN_POSITION_MODE_DOC_VERSION: &str = "kucoin-futures-get-position-mode-2026-07-02";
const KUCOIN_POSITION_MODE_SCHEMA_HASH: &str =
    "sha256:a78c28a8f3d69d3e11dede61335b864c4e45f19a574507ef20798f04efe2b02a";
const KUCOIN_POSITION_MODE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/kucoin/position_mode_hedge.json";
const KUCOIN_POSITION_MODE_PARSER_TEST: &str = "kucoin_position_mode_parses_official_fixture";
const KUCOIN_POSITION_MODE_REQUEST_TEST: &str = "account_mode_read_does_not_require_live_writes";

const HTX_PLACE_ORDER_CHECKED_AT: &str = "2026-06-29";
const HTX_PLACE_ORDER_DOC_VERSION: &str = "htx-usdt-swap-cross-order-2026-06-29";
const HTX_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:43a7aaaf25cbd963330d465138dc5dfcbf768ace19db0da9c005ee9d3ef876fd";
const HTX_PLACE_ORDER_FIXTURE_ID: &str = "crates/exchange/fixtures/htx/swap_cross_order_ack.json";
const HTX_PLACE_ORDER_PARSER_TEST: &str = "htx_place_order_ack_parses_official_fixture";
const HTX_PLACE_ORDER_REQUEST_TEST: &str =
    "crates/exchange/src/adapters/htx_trade_data_tests.rs::limit_order_matches_official_body_shape";
const HTX_ISOLATED_PLACE_ORDER_CHECKED_AT: &str = "2026-07-02";
const HTX_ISOLATED_PLACE_ORDER_DOC_VERSION: &str = "htx-usdt-swap-isolated-order-2026-07-02";
const HTX_ISOLATED_PLACE_ORDER_SCHEMA_HASH: &str =
    "sha256:2d96140355b23fd2e5384fff9ac74e1c003cc5c216c6f7184b25fc99385764c0";
const HTX_ISOLATED_PLACE_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_order_ack.json";
const HTX_ISOLATED_PLACE_ORDER_PARSER_TEST: &str =
    "htx_isolated_place_order_ack_parses_official_fixture";
const HTX_ISOLATED_PLACE_ORDER_REQUEST_TEST: &str = "live_place_order_sends_signed_swap_order";
const HTX_CANCEL_ORDER_CHECKED_AT: &str = "2026-07-02";
const HTX_CANCEL_ORDER_DOC_VERSION: &str = "htx-usdt-swap-cross-cancel-order-2026-07-02";
const HTX_CANCEL_ORDER_SCHEMA_HASH: &str =
    "sha256:f270177dcbe17e7bc67402d56ee16bce0051d559be519c34907b9e1bedc76b25";
const HTX_CANCEL_ORDER_FIXTURE_ID: &str = "crates/exchange/fixtures/htx/swap_cross_cancel_ack.json";
const HTX_CANCEL_ORDER_PARSER_TEST: &str = "htx_cancel_order_ack_parses_official_fixture";
const HTX_CANCEL_ORDER_REQUEST_TEST: &str =
    "live_cancel_order_uses_official_cross_cancel_path_and_order_id";
const HTX_ISOLATED_CANCEL_ORDER_CHECKED_AT: &str = "2026-07-02";
const HTX_ISOLATED_CANCEL_ORDER_DOC_VERSION: &str =
    "htx-usdt-swap-isolated-cancel-order-2026-07-02";
const HTX_ISOLATED_CANCEL_ORDER_SCHEMA_HASH: &str =
    "sha256:3022981d88458c81859a69267aec7d37308265b0002a78120e52798b1848698b";
const HTX_ISOLATED_CANCEL_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_cancel_ack.json";
const HTX_ISOLATED_CANCEL_ORDER_PARSER_TEST: &str =
    "htx_isolated_cancel_order_ack_parses_official_fixture";
const HTX_ISOLATED_CANCEL_ORDER_REQUEST_TEST: &str = "live_cancel_order_sends_signed_cancel";
const HTX_GET_ORDER_CHECKED_AT: &str = "2026-06-30";
const HTX_GET_ORDER_DOC_VERSION: &str = "htx-usdt-swap-get-information-of-an-order-2026-06-30";
const HTX_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:7c0578a431a5bca0c78361c0ab4f044cdddc384a33aaea2dbe96265a98aa409f";
const HTX_GET_ORDER_FIXTURE_ID: &str = "crates/exchange/fixtures/htx/swap_order_info_filled.json";
const HTX_GET_ORDER_PARSER_TEST: &str =
    "crates/exchange/src/adapters/htx_private_data_tests.rs::get_order_official_envelope_parses_filled_order";
const HTX_GET_ORDER_REQUEST_TEST: &str = "live_get_order_parses_strict_order_info";
const HTX_CROSS_GET_ORDER_CHECKED_AT: &str = "2026-07-08";
const HTX_CROSS_GET_ORDER_DOC_VERSION: &str =
    "htx-usdt-swap-cross-get-information-of-order-2026-07-08";
const HTX_CROSS_GET_ORDER_SCHEMA_HASH: &str =
    "sha256:c639d3b390988fc6682e92217dcddd3a81795c65f30405969bba2984d4f3f58a";
const HTX_CROSS_GET_ORDER_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_cross_order_info_filled.json";
const HTX_CROSS_GET_ORDER_PARSER_TEST: &str = "htx_cross_order_info_parses_official_fixture";
const HTX_CROSS_GET_ORDER_REQUEST_TEST: &str = "htx_cross_get_order_parses_object_order_info";
const HTX_OPEN_ORDERS_CHECKED_AT: &str = "2026-07-02";
const HTX_OPEN_ORDERS_DOC_VERSION: &str = "htx-usdt-swap-isolated-openorders-2026-07-02";
const HTX_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:5fc1d5ddf498aabdcebd6b5887ba5b2fa4f0bd09b7ec8c3cbcd821ee3f10dd1c";
const HTX_OPEN_ORDERS_FIXTURE_ID: &str = "crates/exchange/fixtures/htx/swap_openorders_limit.json";
const HTX_OPEN_ORDERS_PARSER_TEST: &str = "htx_open_orders_parses_official_fixture";
const HTX_OPEN_ORDERS_REQUEST_TEST: &str = "live_open_orders_rejects_malformed_status";
const HTX_CROSS_OPEN_ORDERS_CHECKED_AT: &str = "2026-07-08";
const HTX_CROSS_OPEN_ORDERS_DOC_VERSION: &str = "htx-usdt-swap-cross-openorders-2026-07-08";
const HTX_CROSS_OPEN_ORDERS_SCHEMA_HASH: &str =
    "sha256:7f63f31bd75482d0d0e76eda7f2e5984a990448da7d324892596931760dc007f";
const HTX_CROSS_OPEN_ORDERS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_cross_openorders_limit.json";
const HTX_CROSS_OPEN_ORDERS_PARSER_TEST: &str = "htx_cross_open_orders_parses_official_fixture";
const HTX_CROSS_OPEN_ORDERS_REQUEST_TEST: &str = "htx_cross_open_orders_queries_cross_openorders";
const HTX_ACCOUNT_INFO_CHECKED_AT: &str = "2026-07-02";
const HTX_ACCOUNT_INFO_DOC_VERSION: &str = "htx-usdt-swap-isolated-account-info-2026-07-02";
const HTX_ACCOUNT_INFO_SCHEMA_HASH: &str =
    "sha256:da6336090c8520e5ee8f5d7bbd168c05209a8b632512aaf76607e8d5e46cf59e";
const HTX_ACCOUNT_INFO_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_account_info_usdt.json";
const HTX_ACCOUNT_INFO_PARSER_TEST: &str = "htx_account_info_parses_official_fixture";
const HTX_ACCOUNT_INFO_REQUEST_TEST: &str = "balance_signed_via_query";
const HTX_CROSS_ACCOUNT_INFO_CHECKED_AT: &str = "2026-07-08";
const HTX_CROSS_ACCOUNT_INFO_DOC_VERSION: &str = "htx-usdt-swap-cross-account-info-2026-07-08";
const HTX_CROSS_ACCOUNT_INFO_SCHEMA_HASH: &str =
    "sha256:342020e437ba562d0a439c140c2911162c9abcf925a4720a5abe3c25a5ae3df2";
const HTX_CROSS_ACCOUNT_INFO_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_cross_account_info_usdt.json";
const HTX_CROSS_ACCOUNT_INFO_PARSER_TEST: &str = "htx_cross_account_info_parses_official_fixture";
const HTX_CROSS_ACCOUNT_INFO_REQUEST_TEST: &str = "htx_cross_balance_signed_via_query";
const HTX_ACCOUNT_POSITION_CHECKED_AT: &str = "2026-07-02";
const HTX_ACCOUNT_POSITION_DOC_VERSION: &str =
    "htx-usdt-swap-isolated-account-position-info-2026-07-02";
const HTX_ACCOUNT_POSITION_SCHEMA_HASH: &str =
    "sha256:b90b4fbdb41028008b0e7c57d158f1f7481af5636b0c29a035a0e66a06ae0d25";
const HTX_ACCOUNT_POSITION_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_account_position_info_btc_usdt.json";
const HTX_ACCOUNT_POSITION_PARSER_TEST: &str = "htx_positions_parse_official_fixture";
const HTX_ACCOUNT_POSITION_REQUEST_TEST: &str =
    "live_positions_queries_signed_account_position_info";
const HTX_CROSS_ACCOUNT_POSITION_CHECKED_AT: &str = "2026-07-08";
const HTX_CROSS_ACCOUNT_POSITION_DOC_VERSION: &str =
    "htx-usdt-swap-cross-account-position-info-2026-07-08";
const HTX_CROSS_ACCOUNT_POSITION_SCHEMA_HASH: &str =
    "sha256:8491ecafc4fc1500a8c03efa86931bfbac7c38da660bbc3d14eb9b259ef0cfe3";
const HTX_CROSS_ACCOUNT_POSITION_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_cross_account_position_info_btc_usdt.json";
const HTX_CROSS_ACCOUNT_POSITION_PARSER_TEST: &str = "htx_cross_positions_parse_official_fixture";
const HTX_CROSS_ACCOUNT_POSITION_REQUEST_TEST: &str =
    "htx_cross_positions_queries_signed_account_position_info";
const HTX_ACCOUNT_TYPE_CHECKED_AT: &str = "2026-07-02";
const HTX_ACCOUNT_TYPE_DOC_VERSION: &str = "htx-usdt-swap-account-type-query-2026-07-02";
const HTX_ACCOUNT_TYPE_SCHEMA_HASH: &str =
    "sha256:1d735b53ef982fa98504c37ce5d5a0989068f84e86ebe899439d9627925822bc";
const HTX_ACCOUNT_TYPE_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_unified_account_type_non_unified.json";
const HTX_ACCOUNT_TYPE_PARSER_TEST: &str = "htx_account_type_parses_official_fixture";
const HTX_ACCOUNT_TYPE_REQUEST_TEST: &str = "live_account_mode_reads_non_unified_account_type";
const HTX_API_TRADING_STATUS_CHECKED_AT: &str = "2026-07-02";
const HTX_API_TRADING_STATUS_DOC_VERSION: &str = "htx-usdt-swap-api-trading-status-2026-07-02";
const HTX_API_TRADING_STATUS_SCHEMA_HASH: &str =
    "sha256:d830565ac920258e034ce247f7abcbe0fb9957d0d41e8d4ea7b8d92820f7ae59";
const HTX_API_TRADING_STATUS_FIXTURE_ID: &str =
    "crates/exchange/fixtures/htx/swap_api_trading_status_disabled.json";
const HTX_API_TRADING_STATUS_PARSER_TEST: &str = "htx_api_trading_status_parses_official_fixture";
const HTX_API_TRADING_STATUS_REQUEST_TEST: &str =
    "live_preflight_order_checks_api_trading_status_without_submit";

const KRAKEN_SPOT_TOKEN_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-spot-rest-get-websockets-token-2026-08-06",
    schema_hash: "sha256:de1a048ae3fb97d52533e7a1a4e7f585c9fd4f03b12a0f7b568668e2d3f88746",
    fixture_id: "crates/exchange/fixtures/kraken/spot_ws_token.json",
    parser_test: "official_token_fixture_and_private_read_requests_match_spot_contracts",
    request_builder_test: "official_token_fixture_and_private_read_requests_match_spot_contracts",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_SPOT_BALANCE_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-spot-rest-extended-balance-2026-08-06",
    schema_hash: "sha256:9c973150d855fb824fa13fe58e77c2c2e7dc7c41ea7349b57a22c42133994525",
    fixture_id: "crates/exchange/fixtures/kraken/spot_balance_ex.json",
    parser_test: "balance_ex_uses_official_available_formula",
    request_builder_test: "signed_request_uses_official_headers_and_body_contract",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_SPOT_OPEN_ORDERS_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-spot-rest-open-orders-2026-08-06",
    schema_hash: "sha256:1e6ca36c5632885892b04f44bf7bced54a29e6831b4e9274ea1f0f4fbaa1a5d1",
    fixture_id: "crates/exchange/fixtures/kraken/spot_open_orders.json",
    parser_test: "rest_order_fixtures_keep_open_and_terminal_state",
    request_builder_test: "official_token_fixture_and_private_read_requests_match_spot_contracts",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_SPOT_QUERY_ORDERS_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-spot-rest-query-orders-2026-08-06",
    schema_hash: "sha256:2de01b2037908770cc948630aec4c44d2052217860fc3913309dad3b064a3afb",
    fixture_id: "crates/exchange/fixtures/kraken/spot_query_orders.json",
    parser_test: "rest_order_fixtures_keep_open_and_terminal_state",
    request_builder_test: "official_token_fixture_and_private_read_requests_match_spot_contracts",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_FUTURES_SEND_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-derivatives-rest-send-order-2026-08-06",
    schema_hash: "sha256:e3121498a6423ec9ee3c7519b156c4d98b0866f7202d7b5fcfccaceac4fb1169",
    fixture_id: "crates/exchange/fixtures/kraken/futures_send_order_ack.json",
    parser_test: "parses_official_trade_write_and_order_status_responses",
    request_builder_test: "signed_requests_bind_official_futures_paths_headers_and_payloads",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_FUTURES_CANCEL_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-derivatives-rest-cancel-order-2026-08-06",
    schema_hash: "sha256:55676659980435fcd1ba33f392cb0e55360f97d578c6bab55af75de0265cbae0",
    fixture_id: "crates/exchange/fixtures/kraken/futures_cancel_order_ack.json",
    parser_test: "parses_official_trade_write_and_order_status_responses",
    request_builder_test: "signed_requests_bind_official_futures_paths_headers_and_payloads",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_FUTURES_OPEN_ORDERS_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-derivatives-rest-open-orders-2026-08-06",
    schema_hash: "sha256:df8cb6a43089b3340b16206f916e902321d98fdbc9ad1465736f9ac6104f9101",
    fixture_id: "crates/exchange/fixtures/kraken/futures_rest_open_orders.json",
    parser_test: "parses_official_rest_accounts_and_open_rows",
    request_builder_test: "signed_requests_bind_official_futures_paths_headers_and_payloads",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_FUTURES_ORDER_STATUS_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-derivatives-rest-specific-order-status-2026-08-06",
    schema_hash: "sha256:03519461734113877bed17606e416daa9d06870c45237751efdf184d89b1ccca",
    fixture_id: "crates/exchange/fixtures/kraken/futures_order_status.json",
    parser_test: "parses_official_trade_write_and_order_status_responses",
    request_builder_test: "signed_requests_bind_official_futures_paths_headers_and_payloads",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_FUTURES_ACCOUNT_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-derivatives-rest-wallets-2026-08-06",
    schema_hash: "sha256:696f464b0811801595f00ca90504c9c5f5d5e020996aa8635f901f1e9bad5b41",
    fixture_id: "crates/exchange/fixtures/kraken/futures_accounts.json",
    parser_test: "parses_official_rest_accounts_and_open_rows",
    request_builder_test: "signed_requests_bind_official_futures_paths_headers_and_payloads",
    auth_kind: SIGNED_AUTH_KIND,
};
const KRAKEN_FUTURES_POSITION_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "kraken-derivatives-rest-open-positions-2026-08-06",
    schema_hash: "sha256:c7f885e4c2117efdd0244f7e4e97deebfd91d370437a998883d03a16c001d4e2",
    fixture_id: "crates/exchange/fixtures/kraken/futures_open_positions.json",
    parser_test: "parses_official_rest_accounts_and_open_rows",
    request_builder_test: "signed_requests_bind_official_futures_paths_headers_and_payloads",
    auth_kind: SIGNED_AUTH_KIND,
};
const GATE_CROSSEX_FUNDING_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "gate-crossex-v1.0.1-funding-info-2026-08-06",
    schema_hash: "sha256:3341339b9d8ea8df5996a51656dd90c1efbd53f7c03539d66c8f6e083ef91490",
    fixture_id: "crates/exchange/fixtures/gate_crossex/funding_info_intervals.json",
    parser_test: "funding_metadata_keeps_native_event_intervals",
    request_builder_test: "signed_private_reads_bind_canonical_crossex_paths_and_queries",
    auth_kind: SIGNED_AUTH_KIND,
};
const GATE_CROSSEX_ACCOUNT_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "gate-crossex-v1.0.1-account-assets-2026-08-06",
    schema_hash: "sha256:2af431f2c94d5de41ef04da4e15162d9a58ad60680af083f51db1ce3da270252",
    fixture_id: "crates/exchange/fixtures/gate_crossex/account.json",
    parser_test: "parses_bounded_rest_bootstrap_rows",
    request_builder_test: "signed_private_reads_bind_canonical_crossex_paths_and_queries",
    auth_kind: SIGNED_AUTH_KIND,
};
const GATE_CROSSEX_OPEN_ORDERS_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "gate-crossex-v1.0.1-open-orders-2026-08-06",
    schema_hash: "sha256:d5982036259dce34d2879404f2bfacc8481741e221e334b77b65ce210ded9b2c",
    fixture_id: "crates/exchange/fixtures/gate_crossex/open_orders.json",
    parser_test: "parses_bounded_rest_bootstrap_rows",
    request_builder_test: "signed_private_reads_bind_canonical_crossex_paths_and_queries",
    auth_kind: SIGNED_AUTH_KIND,
};
const GATE_CROSSEX_ORDER_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "gate-crossex-v1.0.1-order-details-2026-08-06",
    schema_hash: "sha256:cfae949acb63f815882f8e29973f9ae15a351ab282cf5c545f3863d14bf4c567",
    fixture_id: "crates/exchange/fixtures/gate_crossex/order_detail.json",
    parser_test: "parses_bounded_rest_bootstrap_rows",
    request_builder_test: "signed_private_reads_bind_canonical_crossex_paths_and_queries",
    auth_kind: SIGNED_AUTH_KIND,
};
const GATE_CROSSEX_POSITION_EVIDENCE: EndpointEvidenceMeta = EndpointEvidenceMeta {
    checked_at: "2026-08-06",
    doc_version: "gate-crossex-v1.0.1-contract-positions-2026-08-06",
    schema_hash: "sha256:fcfa6c34f193e4adfcd5a46ae72d86cebfca9c5345a6f56fec30c091341e2532",
    fixture_id: "crates/exchange/fixtures/gate_crossex/positions.json",
    parser_test: "parses_bounded_rest_bootstrap_rows",
    request_builder_test: "signed_private_reads_bind_canonical_crossex_paths_and_queries",
    auth_kind: SIGNED_AUTH_KIND,
};

pub const ENDPOINT_SPECS: &[EndpointSpec] = &[
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/depth",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Order-Book",
        weight: 2,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/market/books",
        doc_url: "https://www.okx.com/docs-v5/en/#order-book-trading-market-data-get-order-book",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/orderbook",
        doc_url: "https://bybit-exchange.github.io/docs/v5/market/orderbook",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/orderbook",
        doc_url: "https://www.bitget.com/api-doc/uta/public/OrderBook",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/order_book",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/futures/#query-futures-market-depth-information",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-ex/market/depth",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#general-get-market-depth",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/level2/depth20",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-part-orderbook",
        weight: 5,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#l2-book-snapshot",
        weight: 2,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::HotPathFallback,
        data_kind: EndpointDataKind::OrderBook,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/time",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Check-Server-Time",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/time",
        doc_url: "https://www.okx.com/docs-v5/en/#public-data-rest-api-get-system-time",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/time",
        doc_url: "https://bybit-exchange.github.io/docs/v5/market/time",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v2/public/time",
        doc_url: "https://www.bitget.com/api-doc/common/public/Get-Server-Time",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/spot/time",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/#get-server-current-time",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/api/v1/timestamp",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#get-current-system-timestamp",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/timestamp",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-server-time",
        weight: 2,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Calibration,
        data_kind: EndpointDataKind::ServerTime,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/exchangeInfo",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Exchange-Information",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/instruments",
        doc_url: "https://www.okx.com/docs-v5/en/#public-data-rest-api-get-instruments",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/instruments-info",
        doc_url: "https://bybit-exchange.github.io/docs/v5/market/instrument",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/instruments",
        doc_url: "https://www.bitget.com/api-doc/uta/public/Instruments",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/contracts",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/#list-futures-contracts",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_contract_info",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#general-query-swap-info",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/contracts/active",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-symbols",
        weight: 3,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-perpetuals-metadata-universe-and-margin-tables",
        weight: 20,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Metadata,
        data_kind: EndpointDataKind::InstrumentMetadata,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/premiumIndex",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Mark-Price",
        weight: 10,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/premiumIndex",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Mark-Price",
        weight: 10,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/openInterest",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Open-Interest",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::OpenInterest,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/ticker/24hr",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/24hr-Ticker-Price-Change-Statistics",
        weight: 40,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/api/v3/ticker/24hr",
        doc_url: "https://developers.binance.com/docs/binance-spot-api-docs/rest-api/market-data-endpoints#24hr-ticker-price-change-statistics",
        weight: 80,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/market/tickers",
        doc_url: "https://www.okx.com/docs-v5/en/#order-book-trading-market-data-get-tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/market/tickers",
        doc_url: "https://www.okx.com/docs-v5/en/#order-book-trading-market-data-get-tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/funding-rate",
        doc_url: "https://www.okx.com/docs-v5/en/#public-data-rest-api-get-funding-rate",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/mark-price",
        doc_url: "https://www.okx.com/docs-v5/en/#public-data-rest-api-get-mark-price",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/market/index-tickers",
        doc_url: "https://www.okx.com/docs-v5/en/#public-data-rest-api-get-index-tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/public/open-interest",
        doc_url: "https://www.okx.com/docs-v5/en/#public-data-rest-api-get-open-interest",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::OpenInterest,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/tickers",
        doc_url: "https://bybit-exchange.github.io/docs/v5/market/tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/tickers",
        doc_url: "https://bybit-exchange.github.io/docs/v5/market/tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/market/tickers",
        doc_url: "https://bybit-exchange.github.io/docs/v5/market/tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/current-fund-rate",
        doc_url: "https://www.bitget.com/api-doc/uta/public/Get-Current-Funding-Rate",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/tickers",
        doc_url: "https://www.bitget.com/api-doc/uta/public/Tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/market/tickers",
        doc_url: "https://www.bitget.com/api-doc/uta/public/Tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/tickers",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/#list-futures-tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/spot/tickers",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/#list-spot-tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/contracts",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/#list-futures-contracts",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_batch_funding_rate",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#general-query-a-batch-of-funding-rate",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-ex/market/detail/merged",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#general-get-market-data-overview",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/index/market/history/linear_swap_mark_price_kline",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#general-get-kline-data-of-mark-price",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_index",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#general-query-swap-index-price-information",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::MarkIndex,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_open_interest",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#general-get-swap-open-interest-information",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::OpenInterest,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/market/tickers",
        doc_url: "https://huobiapi.github.io/docs/spot/v1/en/#get-market-tickers",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/allTickers",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-tickers",
        weight: 5,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/contracts/active",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-symbols",
        weight: 3,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/market/allTickers",
        doc_url: "https://www.kucoin.com/docs-new/rest/spot-trading/market-data/get-all-tickers",
        weight: 15,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-perpetuals-asset-contexts-includes-mark-price-current-funding-open-interest-etc",
        weight: 20,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::PerpTicker,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/spot#retrieve-spot-asset-contexts",
        weight: 20,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::SpotTicker,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-predicted-fundings",
        weight: 20,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Post,
        path: "/fapi/v1/order",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/New-Order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Post,
        path: "/fapi/v1/order/test",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/New-Order-Test",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Delete,
        path: "/fapi/v1/order",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/Cancel-Order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Post,
        path: "/api/v5/trade/order",
        doc_url: "https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-place-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Post,
        path: "/api/v5/trade/order-precheck",
        doc_url: "https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-order-precheck",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Post,
        path: "/api/v5/trade/cancel-order",
        doc_url: "https://www.okx.com/docs-v5/en/#order-book-trading-trade-post-cancel-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/trade/order",
        doc_url: "https://www.okx.com/docs-v5/en/#order-book-trading-trade-get-order-details",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/trade/orders-pending",
        doc_url: "https://www.okx.com/docs-v5/en/#order-book-trading-trade-get-order-list",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/account/config",
        doc_url: "https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-account-configuration",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/account/balance",
        doc_url: "https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-balance",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/account/bills",
        doc_url: "https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-bills-details-last-7-days",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
    },
    EndpointSpec {
        venue: VenueId::Okx,
        method: HttpMethod::Get,
        path: "/api/v5/account/positions",
        doc_url: "https://www.okx.com/docs-v5/en/#trading-account-rest-api-get-positions",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Post,
        path: "/v5/order/create",
        doc_url: "https://bybit-exchange.github.io/docs/v5/order/create-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Post,
        path: "/v5/order/pre-check",
        doc_url: "https://bybit-exchange.github.io/docs/v5/order/pre-check-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Post,
        path: "/v5/order/cancel",
        doc_url: "https://bybit-exchange.github.io/docs/v5/order/cancel-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/order/realtime",
        doc_url: "https://bybit-exchange.github.io/docs/v5/order/open-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/position/list",
        doc_url: "https://bybit-exchange.github.io/docs/v5/position",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/position/list",
        doc_url: "https://bybit-exchange.github.io/docs/v5/position",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Post,
        path: "/api/v3/trade/place-order",
        doc_url: "https://www.bitget.com/api-doc/uta/trade/Place-Order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Post,
        path: "/api/v3/trade/cancel-order",
        doc_url: "https://www.bitget.com/api-doc/uta/trade/Cancel-Order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/trade/order-info",
        doc_url: "https://www.bitget.com/api-doc/uta/trade/Get-Order-Details",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/account/assets",
        doc_url: "https://www.bitget.com/api-doc/uta/account/Get-Account",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/account/financial-records",
        doc_url: "https://www.bitget.com/api-doc/uta/account/Get-Financial-Records",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/position/current-position",
        doc_url: "https://www.bitget.com/api-doc/uta/trade/Get-Position",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Bitget,
        method: HttpMethod::Get,
        path: "/api/v3/trade/unfilled-orders",
        doc_url: "https://www.bitget.com/api-doc/uta/trade/Get-Order-Pending",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v3/balance",
        doc_url: "https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/rest-api/account#futures-account-balance-v3",
        weight: 5,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/income",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/account/rest-api/Get-Income-History",
        weight: 30,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v3/positionRisk",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/Position-Information-V3",
        weight: 5,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/account/wallet-balance",
        doc_url: "https://bybit-exchange.github.io/docs/v5/account/wallet-balance",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Bybit,
        method: HttpMethod::Get,
        path: "/v5/account/transaction-log",
        doc_url: "https://bybit-exchange.github.io/docs/v5/account/transaction-log",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/order",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/Query-Order",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/openOrders",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/Current-All-Open-Orders",
        weight: 1,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/positionSide/dual",
        doc_url: "https://developers.binance.com/docs/derivatives/usds-margined-futures/account/rest-api/Get-Current-Position-Mode",
        weight: 30,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Binance,
        method: HttpMethod::Get,
        path: "/fapi/v1/commissionRate",
        doc_url: "https://developers.binance.com/en/docs/catalog/core-trading-derivatives-trading-usd-s-m-futures/api/rest-api/account#user-commission-rate",
        weight: 20,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/orders/{order_id}",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/futures/#get-a-single-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Delete,
        path: "/api/v4/futures/usdt/orders/{order_id}",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/futures/#cancel-a-single-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/my_trades",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/futures/#query-personal-trading-records",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/fee",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/futures/#query-futures-market-trading-fee-rates",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/orders",
        doc_url: "https://www.gate.com/docs/futures/api/index.html#list-futures-orders",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/accounts",
        doc_url: "https://www.gate.com/docs/futures/api/index.html#query-futures-account",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/account_book",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/futures/#query-futures-account-change-history",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Get,
        path: "/api/v4/futures/usdt/positions",
        doc_url: "https://www.gate.com/docs/developers/apiv4/en/#list-positions",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Post,
        path: "/api/v1/orders",
        doc_url: "https://www.kucoin.com/docs/rest/futures-trading/orders/place-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Post,
        path: "/api/v1/orders/test",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/orders/add-order-test",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Delete,
        path: "/api/v1/orders/client-order/{clientOid}",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/orders/cancel-order-by-clientoid",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Delete,
        path: "/api/v1/orders/{orderId}",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/orders/cancel-order-by-orderld",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Gate,
        method: HttpMethod::Post,
        path: "/api/v4/futures/usdt/orders",
        doc_url: "https://www.gate.com/docs/futures/api/index.html#create-a-futures-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/orders/byClientOid",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/get-stop-order-by-clientoid",
        weight: 5,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/orders",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/orders/get-order-list",
        weight: 2,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/fills",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/orders/get-trade-history",
        weight: 5,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::TradeFill,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/trade-fees",
        doc_url: "https://www.kucoin.com/docs-new/rest/account-info/trade-fee/get-actual-fee-futures",
        weight: 3,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountFeeRate,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/account-overview",
        doc_url: "https://www.kucoin.com/docs-new/rest/account-info/account-funding/get-account-futures",
        weight: 5,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/funding-history",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/funding-fees/get-private-funding-history",
        weight: 5,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v1/positions",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/positions/get-position-list",
        weight: 2,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Kucoin,
        method: HttpMethod::Get,
        path: "/api/v2/position/getPositionMode",
        doc_url: "https://www.kucoin.com/docs-new/rest/futures-trading/positions/get-position-mode",
        weight: 2,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_order",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#place-an-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_order",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#isolated-place-an-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_cancel",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#cross-cancel-an-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cancel",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#isolated-cancel-an-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_order_info",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#isolated-get-information-of-an-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_order_info",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#cross-get-information-of-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_openorders",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#isolated-current-unfilled-order-acquisition",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_openorders",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#cross-current-unfilled-order-acquisition",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_account_info",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#isolated-query-user-s-account-information",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_account_info",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#cross-query-user-39-s-account-information",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v3/swap_financial_record_exact",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#general-query-account-financial-records-via-multiple-fields-new",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::FundingPayment,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_account_position_info",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#isolated-query-assets-and-positions",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Post,
        path: "/linear-swap-api/v1/swap_cross_account_position_info",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#cross-query-assets-and-positions",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v3/swap_unified_account_type",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#account-type-query",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Htx,
        method: HttpMethod::Get,
        path: "/linear-swap-api/v1/swap_api_trading_status",
        doc_url: "https://huobiapi.github.io/docs/usdt_swap/v1/en/#query-api-trading-status",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/exchange",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint#place-an-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#query-order-status-by-oid-or-cloid",
        weight: 2,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#retrieve-a-users-open-orders",
        weight: 20,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint#retrieve-a-users-open-orders-with-additional-frontend-info",
        weight: 20,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-users-perpetuals-account-summary",
        weight: 2,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/spot#retrieve-a-users-token-balances",
        weight: 2,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Hyperliquid,
        method: HttpMethod::Post,
        path: "/info",
        doc_url: "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals#retrieve-users-perpetuals-account-summary",
        weight: 2,
        rate_scope: RateScope::Ip,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/market/funding_info",
        doc_url: "https://www.gate.com/docs/developers/crossex/en/",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::Baseline,
        data_kind: EndpointDataKind::FundingRate,
    },
    EndpointSpec {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/accounts",
        doc_url: "https://www.gate.com/docs/developers/crossex/en/",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/accounts",
        doc_url: "https://www.gate.com/docs/developers/crossex/en/",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/open_orders",
        doc_url: "https://www.gate.com/docs/developers/crossex/en/",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/orders/{order_id}",
        doc_url: "https://www.gate.com/docs/developers/crossex/en/",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::GateCrossEx,
        method: HttpMethod::Get,
        path: "/api/v4/crossex/positions",
        doc_url: "https://www.gate.com/docs/developers/crossex/en/",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/0/private/GetWebSocketsToken",
        doc_url: "https://docs.kraken.com/api-reference/trading/get-websockets-token",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountConfig,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/0/private/BalanceEx",
        doc_url: "https://docs.kraken.com/api-reference/account-data/get-extended-balance",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/0/private/OpenOrders",
        doc_url: "https://docs.kraken.com/api-reference/account-data/get-open-orders",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/0/private/QueryOrders",
        doc_url: "https://docs.kraken.com/api-reference/account-data/query-orders-info",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/derivatives/api/v3/sendorder",
        doc_url: "https://docs.kraken.com/api-reference/order-management/send-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/derivatives/api/v3/cancelorder",
        doc_url: "https://docs.kraken.com/api-reference/order-management/cancel-order",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::TradeWrite,
        data_kind: EndpointDataKind::OrderAck,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Get,
        path: "/derivatives/api/v3/openorders",
        doc_url: "https://docs.kraken.com/api-reference/order-management/get-open-orders",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Post,
        path: "/derivatives/api/v3/orders/status",
        doc_url: "https://docs.kraken.com/api-reference/order-management/get-specific-orders-status",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::OrderStatus,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Get,
        path: "/derivatives/api/v3/accounts",
        doc_url: "https://docs.kraken.com/api-reference/account-information/get-wallets",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountBalance,
    },
    EndpointSpec {
        venue: VenueId::Kraken,
        method: HttpMethod::Get,
        path: "/derivatives/api/v3/openpositions",
        doc_url: "https://docs.kraken.com/api-reference/account-information/get-open-positions",
        weight: 1,
        rate_scope: RateScope::Account,
        use_case: EndpointUseCase::PrivateRead,
        data_kind: EndpointDataKind::AccountPosition,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_endpoint_has_official_doc_url() {
        for spec in ENDPOINT_SPECS {
            assert!(spec.doc_url.starts_with("https://"));
            assert!(spec.weight > 0);
        }
    }

    #[test]
    fn server_time_calibration_specs_cover_signed_venues() {
        let venues = calibration_venues();
        for venue in venues {
            assert!(
                ENDPOINT_SPECS.iter().any(|spec| {
                    spec.venue == venue
                        && spec.use_case == EndpointUseCase::Calibration
                        && spec.data_kind == EndpointDataKind::ServerTime
                }),
                "missing calibration endpoint for {venue:?}"
            );
        }
    }

    #[test]
    fn endpoint_weight_maps_family_and_path() {
        assert_eq!(
            endpoint_weight("binance", HttpMethod::Get, "/api/v3/ticker/24hr"),
            Some(80)
        );
        assert_eq!(
            endpoint_weight("hyperliquid:xyz", HttpMethod::Post, "/info"),
            Some(20)
        );
        assert_eq!(
            endpoint_weight("unknown", HttpMethod::Get, "/api/v3/ticker/24hr"),
            None
        );
    }

    #[test]
    fn hyperliquid_info_path_only_evidence_is_unavailable() {
        assert!(endpoint_evidence("hyperliquid:xyz", HttpMethod::Post, "/info").is_none());

        for spec in ENDPOINT_SPECS.iter().filter(|spec| {
            spec.venue == VenueId::Hyperliquid
                && spec.method == HttpMethod::Post
                && spec.path == "/info"
        }) {
            let evidence = endpoint_evidence_for_spec(spec);
            assert_eq!(evidence.doc_urls, vec![spec.doc_url.to_owned()]);
            assert_eq!(evidence.use_cases, vec![spec.use_case.as_str().to_owned()]);
            assert_eq!(
                evidence.data_kinds,
                vec![spec.data_kind.as_str().to_owned()]
            );
        }
    }

    #[test]
    fn hyperliquid_spot_tickers_evidence_registry_has_recorded_fixture_metadata() {
        let entry = find_recorded_entry(
            VenueId::Hyperliquid,
            HttpMethod::Post,
            "/info",
            EndpointUseCase::Baseline,
            EndpointDataKind::SpotTicker,
        );
        let spec = find_endpoint_spec(
            VenueId::Hyperliquid,
            HttpMethod::Post,
            "/info",
            EndpointUseCase::Baseline,
            EndpointDataKind::SpotTicker,
        );

        assert_eq!(entry.meta.checked_at, HYPERLIQUID_SPOT_META_CTXS_CHECKED_AT);
        assert_eq!(
            entry.meta.doc_version,
            HYPERLIQUID_SPOT_META_CTXS_DOC_VERSION
        );
        assert_eq!(
            entry.meta.schema_hash,
            HYPERLIQUID_SPOT_META_CTXS_SCHEMA_HASH
        );
        assert_eq!(entry.meta.fixture_id, HYPERLIQUID_SPOT_META_CTXS_FIXTURE_ID);
        assert_eq!(
            entry.meta.parser_test,
            HYPERLIQUID_SPOT_META_CTXS_PARSER_TEST
        );
        assert_eq!(
            entry.meta.request_builder_test,
            HYPERLIQUID_SPOT_TICKERS_REQUEST_TEST
        );
        assert_eq!(entry.meta.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(spec.doc_url, "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/spot#retrieve-spot-asset-contexts");
        assert_eq!(spec.weight, 20);
        assert_eq!(spec.rate_scope, RateScope::Ip);
    }

    #[derive(Clone, Copy)]
    struct ExpectedEvidence<'a> {
        method: &'a str,
        path: &'a str,
        checked_at: &'a str,
        doc_version: &'a str,
        schema_hash: &'a str,
        fixture_id: &'a str,
        parser_test: &'a str,
        request_builder_test: &'a str,
        auth_kind: &'a str,
        weight: u32,
    }

    fn assert_recorded_evidence(evidence: &EndpointEvidenceSnapshot, expected: ExpectedEvidence) {
        assert_eq!(evidence.method, expected.method);
        assert_eq!(evidence.path, expected.path);
        assert_eq!(evidence.checked_at, expected.checked_at);
        assert_eq!(evidence.doc_version, expected.doc_version);
        assert_eq!(evidence.schema_hash, expected.schema_hash);
        assert_eq!(evidence.fixture_id, expected.fixture_id);
        assert_eq!(evidence.parser_test, expected.parser_test);
        assert_eq!(evidence.request_builder_test, expected.request_builder_test);
        assert_eq!(evidence.auth_kind, expected.auth_kind);
        assert_eq!(evidence.weight, expected.weight);
    }

    fn assert_list_contains(values: &[String], expected: &str) {
        assert!(values.iter().any(|value| value == expected));
    }

    fn assert_url_matches(evidence: &EndpointEvidenceSnapshot, pattern: &str) {
        assert!(evidence.doc_urls.iter().any(|url| url.contains(pattern)));
    }

    fn find_recorded_entry(
        venue: VenueId,
        method: HttpMethod,
        path: &str,
        use_case: EndpointUseCase,
        data_kind: EndpointDataKind,
    ) -> &'static EndpointEvidenceEntry {
        RECORDED_ENDPOINT_EVIDENCE
            .iter()
            .find(|entry| {
                entry.venue == venue
                    && entry.method == method
                    && entry.path == path
                    && entry.use_case == use_case
                    && entry.data_kind == data_kind
            })
            .expect("recorded endpoint evidence")
    }

    fn find_endpoint_spec(
        venue: VenueId,
        method: HttpMethod,
        path: &str,
        use_case: EndpointUseCase,
        data_kind: EndpointDataKind,
    ) -> &'static EndpointSpec {
        ENDPOINT_SPECS
            .iter()
            .find(|spec| {
                spec.venue == venue
                    && spec.method == method
                    && spec.path == path
                    && spec.use_case == use_case
                    && spec.data_kind == data_kind
            })
            .expect("endpoint spec")
    }

    type EndpointIdentity = (HttpMethod, &'static str, EndpointUseCase, EndpointDataKind);

    const HTX_PR_EP_CORE_ENDPOINTS: &[EndpointIdentity] = &[
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_order",
            EndpointUseCase::TradeWrite,
            EndpointDataKind::OrderAck,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_order",
            EndpointUseCase::TradeWrite,
            EndpointDataKind::OrderAck,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_cancel",
            EndpointUseCase::TradeWrite,
            EndpointDataKind::OrderAck,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cancel",
            EndpointUseCase::TradeWrite,
            EndpointDataKind::OrderAck,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_order_info",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::OrderStatus,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_order_info",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::OrderStatus,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_openorders",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::OrderStatus,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_openorders",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::OrderStatus,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_account_info",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::AccountBalance,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_account_info",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::AccountBalance,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v3/swap_financial_record_exact",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::FundingPayment,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_account_position_info",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::AccountPosition,
        ),
        (
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_account_position_info",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::AccountPosition,
        ),
        (
            HttpMethod::Get,
            "/linear-swap-api/v3/swap_unified_account_type",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::AccountConfig,
        ),
        (
            HttpMethod::Get,
            "/linear-swap-api/v1/swap_api_trading_status",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::AccountConfig,
        ),
    ];

    #[test]
    fn htx_pr_ep_private_trade_rest_registry_contains_recorded_core() {
        let actual_count = ENDPOINT_SPECS
            .iter()
            .filter(|spec| {
                spec.venue == VenueId::Htx
                    && matches!(
                        spec.use_case,
                        EndpointUseCase::PrivateRead | EndpointUseCase::TradeWrite
                    )
            })
            .count();
        assert!(actual_count >= HTX_PR_EP_CORE_ENDPOINTS.len());

        for &(method, path, use_case, data_kind) in HTX_PR_EP_CORE_ENDPOINTS {
            let spec = find_endpoint_spec(VenueId::Htx, method, path, use_case, data_kind);
            let evidence = find_recorded_entry(VenueId::Htx, method, path, use_case, data_kind);

            assert!(spec.doc_url.starts_with("https://huobiapi.github.io/"));
            assert_eq!(spec.rate_scope, RateScope::Account);
            assert_eq!(evidence.meta.auth_kind, SIGNED_AUTH_KIND);
            assert!(evidence
                .meta
                .fixture_id
                .starts_with("crates/exchange/fixtures/htx/"));
            assert_ne!(evidence.meta.parser_test, UNRECORDED_EVIDENCE_MARKER);
            assert_ne!(
                evidence.meta.request_builder_test,
                UNRECORDED_EVIDENCE_MARKER
            );
        }
    }

    #[test]
    fn htx_trade_ws_schema_endpoint_is_recorded() {
        let spec = VENUE_SPECS
            .iter()
            .find(|spec| spec.venue == VenueId::Htx)
            .expect("htx venue spec");

        assert_eq!(spec.ws_trade, Some("wss://api.hbdm.com/linear-swap-trade"));
    }

    struct ExpectedSafeProbeEvidence<'a> {
        venue_name: &'a str,
        venue: VenueId,
        method: HttpMethod,
        path: &'a str,
        checked_at: &'a str,
        doc_version: &'a str,
        request_builder_test: &'a str,
        doc_url_fragment: &'a str,
    }

    #[test]
    fn safe_order_permission_probe_endpoints_have_registry_evidence() {
        let cases = [
            ExpectedSafeProbeEvidence {
                venue_name: "binance",
                venue: VenueId::Binance,
                method: HttpMethod::Post,
                path: "/fapi/v1/order/test",
                checked_at: BINANCE_USDM_ORDER_TEST_CHECKED_AT,
                doc_version: BINANCE_USDM_ORDER_TEST_DOC_VERSION,
                request_builder_test: BINANCE_USDM_ORDER_TEST_REQUEST_TEST,
                doc_url_fragment: "New-Order-Test",
            },
            ExpectedSafeProbeEvidence {
                venue_name: "okx",
                venue: VenueId::Okx,
                method: HttpMethod::Post,
                path: "/api/v5/trade/order-precheck",
                checked_at: OKX_ORDER_PRECHECK_CHECKED_AT,
                doc_version: OKX_ORDER_PRECHECK_DOC_VERSION,
                request_builder_test: OKX_ORDER_PRECHECK_REQUEST_TEST,
                doc_url_fragment: "order-precheck",
            },
            ExpectedSafeProbeEvidence {
                venue_name: "bybit",
                venue: VenueId::Bybit,
                method: HttpMethod::Post,
                path: "/v5/order/pre-check",
                checked_at: BYBIT_ORDER_PRECHECK_CHECKED_AT,
                doc_version: BYBIT_ORDER_PRECHECK_DOC_VERSION,
                request_builder_test: BYBIT_ORDER_PRECHECK_REQUEST_TEST,
                doc_url_fragment: "pre-check-order",
            },
            ExpectedSafeProbeEvidence {
                venue_name: "gate",
                venue: VenueId::Gate,
                method: HttpMethod::Delete,
                path: "/api/v4/futures/usdt/orders/{order_id}",
                checked_at: GATE_CANCEL_ORDER_CHECKED_AT,
                doc_version: GATE_CANCEL_ORDER_DOC_VERSION,
                request_builder_test: GATE_CANCEL_ORDER_REQUEST_TEST,
                doc_url_fragment: "cancel-a-single-order",
            },
            ExpectedSafeProbeEvidence {
                venue_name: "kucoin",
                venue: VenueId::Kucoin,
                method: HttpMethod::Post,
                path: "/api/v1/orders/test",
                checked_at: KUCOIN_ORDER_TEST_CHECKED_AT,
                doc_version: KUCOIN_ORDER_TEST_DOC_VERSION,
                request_builder_test: KUCOIN_ORDER_TEST_REQUEST_TEST,
                doc_url_fragment: "add-order-test",
            },
        ];

        for case in cases {
            find_recorded_entry(
                case.venue,
                case.method,
                case.path,
                EndpointUseCase::TradeWrite,
                EndpointDataKind::OrderAck,
            );
            find_endpoint_spec(
                case.venue,
                case.method,
                case.path,
                EndpointUseCase::TradeWrite,
                EndpointDataKind::OrderAck,
            );

            let evidence =
                endpoint_evidence(case.venue_name, case.method, case.path).expect("evidence");
            assert_eq!(evidence.checked_at, case.checked_at);
            assert_eq!(evidence.doc_version, case.doc_version);
            assert_eq!(evidence.schema_hash, UNRECORDED_EVIDENCE_MARKER);
            assert_eq!(evidence.fixture_id, UNRECORDED_EVIDENCE_MARKER);
            assert_eq!(evidence.parser_test, UNRECORDED_EVIDENCE_MARKER);
            assert_eq!(evidence.request_builder_test, case.request_builder_test);
            assert_eq!(evidence.auth_kind, SIGNED_AUTH_KIND);
            assert_eq!(evidence.weight, 1);
            assert_list_contains(&evidence.use_cases, "trade_write");
            assert_list_contains(&evidence.data_kinds, "order_ack");
            assert_list_contains(&evidence.rate_scopes, "account");
            assert_url_matches(&evidence, case.doc_url_fragment);
        }
    }

    #[test]
    fn binance_depth_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/depth").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/fapi/v1/depth");
        assert_eq!(evidence.checked_at, BINANCE_USDM_DEPTH_CHECKED_AT);
        assert_eq!(evidence.doc_version, BINANCE_USDM_DEPTH_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BINANCE_USDM_DEPTH_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BINANCE_USDM_DEPTH_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BINANCE_USDM_DEPTH_TEST);
        assert_eq!(evidence.request_builder_test, BINANCE_USDM_DEPTH_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 2);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/Order-Book")));
        assert!(evidence.use_cases.contains(&"hot_path_fallback".to_owned()));
        assert!(evidence.data_kinds.contains(&"order_book".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn binance_server_time_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/time").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/fapi/v1/time");
        assert_eq!(evidence.checked_at, BINANCE_USDM_SERVER_TIME_CHECKED_AT);
        assert_eq!(evidence.doc_version, BINANCE_USDM_SERVER_TIME_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BINANCE_USDM_SERVER_TIME_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BINANCE_USDM_SERVER_TIME_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BINANCE_USDM_SERVER_TIME_TEST);
        assert_eq!(evidence.request_builder_test, BINANCE_USDM_SERVER_TIME_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/Check-Server-Time")));
        assert!(evidence.use_cases.contains(&"calibration".to_owned()));
        assert!(evidence.data_kinds.contains(&"server_time".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn binance_exchange_info_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/exchangeInfo")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/fapi/v1/exchangeInfo");
        assert_eq!(evidence.checked_at, BINANCE_USDM_EXCHANGE_INFO_CHECKED_AT);
        assert_eq!(evidence.doc_version, BINANCE_USDM_EXCHANGE_INFO_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BINANCE_USDM_EXCHANGE_INFO_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BINANCE_USDM_EXCHANGE_INFO_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BINANCE_USDM_EXCHANGE_INFO_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            BINANCE_USDM_EXCHANGE_INFO_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/Exchange-Information")));
        assert!(evidence.use_cases.contains(&"metadata".to_owned()));
        assert!(evidence
            .data_kinds
            .contains(&"instrument_metadata".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn okx_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("okx", HttpMethod::Post, "/api/v5/trade/order").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/api/v5/trade/order",
                checked_at: OKX_PLACE_ORDER_CHECKED_AT,
                doc_version: OKX_PLACE_ORDER_DOC_VERSION,
                schema_hash: OKX_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: OKX_PLACE_ORDER_FIXTURE_ID,
                parser_test: OKX_PLACE_ORDER_PARSER_TEST,
                request_builder_test: OKX_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "place-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn okx_cancel_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("okx", HttpMethod::Post, "/api/v5/trade/cancel-order")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/api/v5/trade/cancel-order",
                checked_at: OKX_CANCEL_ORDER_CHECKED_AT,
                doc_version: OKX_CANCEL_ORDER_DOC_VERSION,
                schema_hash: OKX_CANCEL_ORDER_SCHEMA_HASH,
                fixture_id: OKX_CANCEL_ORDER_FIXTURE_ID,
                parser_test: OKX_CANCEL_ORDER_PARSER_TEST,
                request_builder_test: OKX_CANCEL_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "cancel-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn okx_get_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("okx", HttpMethod::Get, "/api/v5/trade/order").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v5/trade/order",
                checked_at: OKX_GET_ORDER_CHECKED_AT,
                doc_version: OKX_GET_ORDER_DOC_VERSION,
                schema_hash: OKX_GET_ORDER_SCHEMA_HASH,
                fixture_id: OKX_GET_ORDER_FIXTURE_ID,
                parser_test: OKX_GET_ORDER_PARSER_TEST,
                request_builder_test: OKX_GET_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "get-order-details");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn okx_open_orders_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("okx", HttpMethod::Get, "/api/v5/trade/orders-pending")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v5/trade/orders-pending",
                checked_at: OKX_OPEN_ORDERS_CHECKED_AT,
                doc_version: OKX_OPEN_ORDERS_DOC_VERSION,
                schema_hash: OKX_OPEN_ORDERS_SCHEMA_HASH,
                fixture_id: OKX_OPEN_ORDERS_FIXTURE_ID,
                parser_test: OKX_OPEN_ORDERS_PARSER_TEST,
                request_builder_test: OKX_OPEN_ORDERS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "get-order-list");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn okx_account_config_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("okx", HttpMethod::Get, "/api/v5/account/config").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v5/account/config",
                checked_at: OKX_ACCOUNT_CONFIG_CHECKED_AT,
                doc_version: OKX_ACCOUNT_CONFIG_DOC_VERSION,
                schema_hash: OKX_ACCOUNT_CONFIG_SCHEMA_HASH,
                fixture_id: OKX_ACCOUNT_CONFIG_FIXTURE_ID,
                parser_test: OKX_ACCOUNT_CONFIG_PARSER_TEST,
                request_builder_test: OKX_ACCOUNT_CONFIG_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "get-account-configuration");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_config");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn okx_account_balance_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("okx", HttpMethod::Get, "/api/v5/account/balance").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v5/account/balance",
                checked_at: OKX_ACCOUNT_BALANCE_CHECKED_AT,
                doc_version: OKX_ACCOUNT_BALANCE_DOC_VERSION,
                schema_hash: OKX_ACCOUNT_BALANCE_SCHEMA_HASH,
                fixture_id: OKX_ACCOUNT_BALANCE_FIXTURE_ID,
                parser_test: OKX_ACCOUNT_BALANCE_PARSER_TEST,
                request_builder_test: OKX_ACCOUNT_BALANCE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "get-balance");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_balance");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bybit_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bybit", HttpMethod::Post, "/v5/order/create").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/v5/order/create",
                checked_at: BYBIT_PLACE_ORDER_CHECKED_AT,
                doc_version: BYBIT_PLACE_ORDER_DOC_VERSION,
                schema_hash: BYBIT_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: BYBIT_PLACE_ORDER_FIXTURE_ID,
                parser_test: BYBIT_PLACE_ORDER_PARSER_TEST,
                request_builder_test: BYBIT_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "create-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bybit_cancel_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bybit", HttpMethod::Post, "/v5/order/cancel").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/v5/order/cancel",
                checked_at: BYBIT_CANCEL_ORDER_CHECKED_AT,
                doc_version: BYBIT_CANCEL_ORDER_DOC_VERSION,
                schema_hash: BYBIT_CANCEL_ORDER_SCHEMA_HASH,
                fixture_id: BYBIT_CANCEL_ORDER_FIXTURE_ID,
                parser_test: BYBIT_CANCEL_ORDER_PARSER_TEST,
                request_builder_test: BYBIT_CANCEL_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "cancel-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bybit_get_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bybit", HttpMethod::Get, "/v5/order/realtime").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/v5/order/realtime",
                checked_at: BYBIT_GET_ORDER_CHECKED_AT,
                doc_version: BYBIT_GET_ORDER_DOC_VERSION,
                schema_hash: BYBIT_GET_ORDER_SCHEMA_HASH,
                fixture_id: BYBIT_GET_ORDER_FIXTURE_ID,
                parser_test: BYBIT_GET_ORDER_PARSER_TEST,
                request_builder_test: BYBIT_GET_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "open-order");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bybit_position_mode_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bybit", HttpMethod::Get, "/v5/position/list").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/v5/position/list",
                checked_at: BYBIT_POSITION_MODE_CHECKED_AT,
                doc_version: BYBIT_POSITION_MODE_DOC_VERSION,
                schema_hash: BYBIT_POSITION_MODE_SCHEMA_HASH,
                fixture_id: BYBIT_POSITION_MODE_FIXTURE_ID,
                parser_test: BYBIT_POSITION_MODE_PARSER_TEST,
                request_builder_test: BYBIT_POSITION_MODE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "position");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_config");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bitget_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bitget", HttpMethod::Post, "/api/v3/trade/place-order")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/api/v3/trade/place-order",
                checked_at: BITGET_PLACE_ORDER_CHECKED_AT,
                doc_version: BITGET_PLACE_ORDER_DOC_VERSION,
                schema_hash: BITGET_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: BITGET_PLACE_ORDER_FIXTURE_ID,
                parser_test: BITGET_PLACE_ORDER_PARSER_TEST,
                request_builder_test: BITGET_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "Place-Order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bitget_cancel_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bitget", HttpMethod::Post, "/api/v3/trade/cancel-order")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/api/v3/trade/cancel-order",
                checked_at: BITGET_CANCEL_ORDER_CHECKED_AT,
                doc_version: BITGET_CANCEL_ORDER_DOC_VERSION,
                schema_hash: BITGET_CANCEL_ORDER_SCHEMA_HASH,
                fixture_id: BITGET_CANCEL_ORDER_FIXTURE_ID,
                parser_test: BITGET_CANCEL_ORDER_PARSER_TEST,
                request_builder_test: BITGET_CANCEL_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "Cancel-Order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bitget_get_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bitget", HttpMethod::Get, "/api/v3/trade/order-info")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v3/trade/order-info",
                checked_at: BITGET_GET_ORDER_CHECKED_AT,
                doc_version: BITGET_GET_ORDER_DOC_VERSION,
                schema_hash: BITGET_GET_ORDER_SCHEMA_HASH,
                fixture_id: BITGET_GET_ORDER_FIXTURE_ID,
                parser_test: BITGET_GET_ORDER_PARSER_TEST,
                request_builder_test: BITGET_GET_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "Get-Order-Details");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bitget_account_assets_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bitget", HttpMethod::Get, "/api/v3/account/assets")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v3/account/assets",
                checked_at: BITGET_ACCOUNT_ASSETS_CHECKED_AT,
                doc_version: BITGET_ACCOUNT_ASSETS_DOC_VERSION,
                schema_hash: BITGET_ACCOUNT_ASSETS_SCHEMA_HASH,
                fixture_id: BITGET_ACCOUNT_ASSETS_FIXTURE_ID,
                parser_test: BITGET_ACCOUNT_ASSETS_PARSER_TEST,
                request_builder_test: BITGET_ACCOUNT_ASSETS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "Get-Account");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_balance");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bitget_open_orders_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bitget", HttpMethod::Get, "/api/v3/trade/unfilled-orders")
                .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v3/trade/unfilled-orders",
                checked_at: BITGET_OPEN_ORDERS_CHECKED_AT,
                doc_version: BITGET_OPEN_ORDERS_DOC_VERSION,
                schema_hash: BITGET_OPEN_ORDERS_SCHEMA_HASH,
                fixture_id: BITGET_OPEN_ORDERS_FIXTURE_ID,
                parser_test: BITGET_OPEN_ORDERS_PARSER_TEST,
                request_builder_test: BITGET_OPEN_ORDERS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "Get-Order-Pending");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn binance_balance_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("binance", HttpMethod::Get, "/fapi/v3/balance").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/fapi/v3/balance",
                checked_at: BINANCE_USDM_BALANCE_CHECKED_AT,
                doc_version: BINANCE_USDM_BALANCE_DOC_VERSION,
                schema_hash: BINANCE_USDM_BALANCE_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_BALANCE_FIXTURE_ID,
                parser_test: BINANCE_USDM_BALANCE_PARSER_TEST,
                request_builder_test: BINANCE_USDM_BALANCE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 5,
            },
        );
        assert_url_matches(&evidence, "futures-account-balance-v3");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_balance");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn bybit_wallet_balance_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bybit", HttpMethod::Get, "/v5/account/wallet-balance")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/v5/account/wallet-balance",
                checked_at: BYBIT_WALLET_BALANCE_CHECKED_AT,
                doc_version: BYBIT_WALLET_BALANCE_DOC_VERSION,
                schema_hash: BYBIT_WALLET_BALANCE_SCHEMA_HASH,
                fixture_id: BYBIT_WALLET_BALANCE_FIXTURE_ID,
                parser_test: BYBIT_WALLET_BALANCE_PARSER_TEST,
                request_builder_test: BYBIT_WALLET_BALANCE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "wallet-balance");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_balance");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn binance_get_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/order").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/fapi/v1/order",
                checked_at: BINANCE_USDM_GET_ORDER_CHECKED_AT,
                doc_version: BINANCE_USDM_GET_ORDER_DOC_VERSION,
                schema_hash: BINANCE_USDM_GET_ORDER_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_GET_ORDER_FIXTURE_ID,
                parser_test: BINANCE_USDM_GET_ORDER_PARSER_TEST,
                request_builder_test: BINANCE_USDM_GET_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "Query-Order");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn binance_open_orders_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/openOrders").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/fapi/v1/openOrders",
                checked_at: BINANCE_USDM_OPEN_ORDERS_CHECKED_AT,
                doc_version: BINANCE_USDM_OPEN_ORDERS_DOC_VERSION,
                schema_hash: BINANCE_USDM_OPEN_ORDERS_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_OPEN_ORDERS_FIXTURE_ID,
                parser_test: BINANCE_USDM_OPEN_ORDERS_PARSER_TEST,
                request_builder_test: BINANCE_USDM_OPEN_ORDERS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "Current-All-Open-Orders");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn binance_position_mode_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/positionSide/dual")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/fapi/v1/positionSide/dual",
                checked_at: BINANCE_USDM_POSITION_MODE_CHECKED_AT,
                doc_version: BINANCE_USDM_POSITION_MODE_DOC_VERSION,
                schema_hash: BINANCE_USDM_POSITION_MODE_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_POSITION_MODE_FIXTURE_ID,
                parser_test: BINANCE_USDM_POSITION_MODE_PARSER_TEST,
                request_builder_test: BINANCE_USDM_POSITION_MODE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 30,
            },
        );
        assert_url_matches(&evidence, "Get-Current-Position-Mode");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_config");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn binance_commission_rate_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/commissionRate")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/fapi/v1/commissionRate",
                checked_at: BINANCE_USDM_COMMISSION_RATE_CHECKED_AT,
                doc_version: BINANCE_USDM_COMMISSION_RATE_DOC_VERSION,
                schema_hash: BINANCE_USDM_COMMISSION_RATE_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_COMMISSION_RATE_FIXTURE_ID,
                parser_test: BINANCE_USDM_COMMISSION_RATE_PARSER_TEST,
                request_builder_test: BINANCE_USDM_COMMISSION_RATE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 20,
            },
        );
        assert_url_matches(&evidence, "user-commission-rate");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_config");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn binance_pr_el_private_rest_registry_is_recorded() {
        let cases = [
            ("/fapi/v3/balance", EndpointDataKind::AccountBalance),
            ("/fapi/v3/positionRisk", EndpointDataKind::AccountPosition),
            ("/fapi/v1/openOrders", EndpointDataKind::OrderStatus),
            ("/fapi/v1/order", EndpointDataKind::OrderStatus),
        ];

        for (path, data_kind) in cases {
            let entry = find_recorded_entry(
                VenueId::Binance,
                HttpMethod::Get,
                path,
                EndpointUseCase::PrivateRead,
                data_kind,
            );
            assert_ne!(entry.meta.fixture_id, UNRECORDED_EVIDENCE_MARKER);
            assert_ne!(entry.meta.parser_test, UNRECORDED_EVIDENCE_MARKER);
            assert_ne!(entry.meta.request_builder_test, UNRECORDED_EVIDENCE_MARKER);
            assert_eq!(entry.meta.auth_kind, SIGNED_AUTH_KIND);
        }
    }

    #[test]
    fn gate_get_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "gate",
            HttpMethod::Get,
            "/api/v4/futures/usdt/orders/{order_id}",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v4/futures/usdt/orders/{order_id}",
                checked_at: GATE_GET_ORDER_CHECKED_AT,
                doc_version: GATE_GET_ORDER_DOC_VERSION,
                schema_hash: GATE_GET_ORDER_SCHEMA_HASH,
                fixture_id: GATE_GET_ORDER_FIXTURE_ID,
                parser_test: GATE_GET_ORDER_PARSER_TEST,
                request_builder_test: GATE_GET_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "get-a-single-order");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn gate_my_trades_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("gate", HttpMethod::Get, "/api/v4/futures/usdt/my_trades")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v4/futures/usdt/my_trades",
                checked_at: GATE_MY_TRADES_CHECKED_AT,
                doc_version: GATE_MY_TRADES_DOC_VERSION,
                schema_hash: GATE_MY_TRADES_SCHEMA_HASH,
                fixture_id: GATE_MY_TRADES_FIXTURE_ID,
                parser_test: GATE_MY_TRADES_PARSER_TEST,
                request_builder_test: GATE_MY_TRADES_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "query-personal-trading-records");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn gate_futures_fee_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("gate", HttpMethod::Get, "/api/v4/futures/usdt/fee")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v4/futures/usdt/fee",
                checked_at: GATE_FUTURES_FEE_CHECKED_AT,
                doc_version: GATE_FUTURES_FEE_DOC_VERSION,
                schema_hash: GATE_FUTURES_FEE_SCHEMA_HASH,
                fixture_id: GATE_FUTURES_FEE_FIXTURE_ID,
                parser_test: GATE_FUTURES_FEE_PARSER_TEST,
                request_builder_test: GATE_FUTURES_FEE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "query-futures-market-trading-fee-rates");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_config");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn gate_open_orders_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("gate", HttpMethod::Get, "/api/v4/futures/usdt/orders")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v4/futures/usdt/orders",
                checked_at: GATE_OPEN_ORDERS_CHECKED_AT,
                doc_version: GATE_OPEN_ORDERS_DOC_VERSION,
                schema_hash: GATE_OPEN_ORDERS_SCHEMA_HASH,
                fixture_id: GATE_OPEN_ORDERS_FIXTURE_ID,
                parser_test: GATE_OPEN_ORDERS_PARSER_TEST,
                request_builder_test: GATE_OPEN_ORDERS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "list-futures-orders");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn gate_account_balance_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("gate", HttpMethod::Get, "/api/v4/futures/usdt/accounts")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v4/futures/usdt/accounts",
                checked_at: GATE_ACCOUNT_BALANCE_CHECKED_AT,
                doc_version: GATE_ACCOUNT_BALANCE_DOC_VERSION,
                schema_hash: GATE_ACCOUNT_BALANCE_SCHEMA_HASH,
                fixture_id: GATE_ACCOUNT_BALANCE_FIXTURE_ID,
                parser_test: GATE_ACCOUNT_BALANCE_PARSER_TEST,
                request_builder_test: GATE_ACCOUNT_BALANCE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "query-futures-account");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_balance");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn kucoin_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("kucoin", HttpMethod::Post, "/api/v1/orders").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/api/v1/orders",
                checked_at: KUCOIN_PLACE_ORDER_CHECKED_AT,
                doc_version: KUCOIN_PLACE_ORDER_DOC_VERSION,
                schema_hash: KUCOIN_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: KUCOIN_PLACE_ORDER_FIXTURE_ID,
                parser_test: KUCOIN_PLACE_ORDER_PARSER_TEST,
                request_builder_test: KUCOIN_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "place-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn kucoin_cancel_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "kucoin",
            HttpMethod::Delete,
            "/api/v1/orders/client-order/{clientOid}",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "DELETE",
                path: "/api/v1/orders/client-order/{clientOid}",
                checked_at: KUCOIN_CANCEL_ORDER_CHECKED_AT,
                doc_version: KUCOIN_CANCEL_ORDER_DOC_VERSION,
                schema_hash: KUCOIN_CANCEL_ORDER_SCHEMA_HASH,
                fixture_id: KUCOIN_CANCEL_ORDER_FIXTURE_ID,
                parser_test: KUCOIN_CANCEL_ORDER_PARSER_TEST,
                request_builder_test: KUCOIN_CANCEL_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "cancel-order-by-clientoid");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn kucoin_pr_eo_private_evidence_is_recorded() {
        let cancel = endpoint_evidence("kucoin", HttpMethod::Delete, "/api/v1/orders/{orderId}")
            .expect("cancel by order id evidence");
        assert_eq!(cancel.checked_at, KUCOIN_CANCEL_BY_ORDER_ID_CHECKED_AT);
        assert_eq!(cancel.fixture_id, KUCOIN_CANCEL_ORDER_FIXTURE_ID);
        assert_eq!(
            cancel.request_builder_test,
            KUCOIN_CANCEL_BY_ORDER_ID_REQUEST_TEST
        );

        let fills =
            endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/fills").expect("fills evidence");
        assert_eq!(fills.schema_hash, KUCOIN_FILLS_SCHEMA_HASH);
        assert_eq!(fills.fixture_id, KUCOIN_FILLS_FIXTURE_ID);
        assert_list_contains(&fills.data_kinds, "trade_fill");

        let fees = endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/trade-fees")
            .expect("fee evidence");
        assert_eq!(fees.schema_hash, KUCOIN_FEE_RATE_SCHEMA_HASH);
        assert_eq!(fees.fixture_id, KUCOIN_FEE_RATE_FIXTURE_ID);
        assert_list_contains(&fees.data_kinds, "account_fee_rate");
    }

    #[test]
    fn gate_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("gate", HttpMethod::Post, "/api/v4/futures/usdt/orders")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/api/v4/futures/usdt/orders",
                checked_at: GATE_PLACE_ORDER_CHECKED_AT,
                doc_version: GATE_PLACE_ORDER_DOC_VERSION,
                schema_hash: GATE_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: GATE_PLACE_ORDER_FIXTURE_ID,
                parser_test: GATE_PLACE_ORDER_PARSER_TEST,
                request_builder_test: GATE_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "create-a-futures-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn kucoin_get_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/orders/byClientOid")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v1/orders/byClientOid",
                checked_at: KUCOIN_GET_ORDER_CHECKED_AT,
                doc_version: KUCOIN_GET_ORDER_DOC_VERSION,
                schema_hash: KUCOIN_GET_ORDER_SCHEMA_HASH,
                fixture_id: KUCOIN_GET_ORDER_FIXTURE_ID,
                parser_test: KUCOIN_GET_ORDER_PARSER_TEST,
                request_builder_test: KUCOIN_GET_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 5,
            },
        );
        assert_url_matches(&evidence, "get-stop-order-by-clientoid");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn kucoin_open_orders_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/orders").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v1/orders",
                checked_at: KUCOIN_OPEN_ORDERS_CHECKED_AT,
                doc_version: KUCOIN_OPEN_ORDERS_DOC_VERSION,
                schema_hash: KUCOIN_OPEN_ORDERS_SCHEMA_HASH,
                fixture_id: KUCOIN_OPEN_ORDERS_FIXTURE_ID,
                parser_test: KUCOIN_OPEN_ORDERS_PARSER_TEST,
                request_builder_test: KUCOIN_OPEN_ORDERS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 2,
            },
        );
        assert_url_matches(&evidence, "get-order-list");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn kucoin_account_overview_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/account-overview")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v1/account-overview",
                checked_at: KUCOIN_ACCOUNT_OVERVIEW_CHECKED_AT,
                doc_version: KUCOIN_ACCOUNT_OVERVIEW_DOC_VERSION,
                schema_hash: KUCOIN_ACCOUNT_OVERVIEW_SCHEMA_HASH,
                fixture_id: KUCOIN_ACCOUNT_OVERVIEW_FIXTURE_ID,
                parser_test: KUCOIN_ACCOUNT_OVERVIEW_PARSER_TEST,
                request_builder_test: KUCOIN_ACCOUNT_OVERVIEW_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 5,
            },
        );
        assert_url_matches(&evidence, "get-account-futures");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_balance");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn kucoin_position_mode_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "kucoin",
            HttpMethod::Get,
            "/api/v2/position/getPositionMode",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v2/position/getPositionMode",
                checked_at: KUCOIN_POSITION_MODE_CHECKED_AT,
                doc_version: KUCOIN_POSITION_MODE_DOC_VERSION,
                schema_hash: KUCOIN_POSITION_MODE_SCHEMA_HASH,
                fixture_id: KUCOIN_POSITION_MODE_FIXTURE_ID,
                parser_test: KUCOIN_POSITION_MODE_PARSER_TEST,
                request_builder_test: KUCOIN_POSITION_MODE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 2,
            },
        );
        assert_url_matches(&evidence, "get-position-mode");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_config");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_order",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_cross_order",
                checked_at: HTX_PLACE_ORDER_CHECKED_AT,
                doc_version: HTX_PLACE_ORDER_DOC_VERSION,
                schema_hash: HTX_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: HTX_PLACE_ORDER_FIXTURE_ID,
                parser_test: HTX_PLACE_ORDER_PARSER_TEST,
                request_builder_test: HTX_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "place-an-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_isolated_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("htx", HttpMethod::Post, "/linear-swap-api/v1/swap_order")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_order",
                checked_at: HTX_ISOLATED_PLACE_ORDER_CHECKED_AT,
                doc_version: HTX_ISOLATED_PLACE_ORDER_DOC_VERSION,
                schema_hash: HTX_ISOLATED_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: HTX_ISOLATED_PLACE_ORDER_FIXTURE_ID,
                parser_test: HTX_ISOLATED_PLACE_ORDER_PARSER_TEST,
                request_builder_test: HTX_ISOLATED_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "isolated-place-an-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_cancel_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_cancel",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_cross_cancel",
                checked_at: HTX_CANCEL_ORDER_CHECKED_AT,
                doc_version: HTX_CANCEL_ORDER_DOC_VERSION,
                schema_hash: HTX_CANCEL_ORDER_SCHEMA_HASH,
                fixture_id: HTX_CANCEL_ORDER_FIXTURE_ID,
                parser_test: HTX_CANCEL_ORDER_PARSER_TEST,
                request_builder_test: HTX_CANCEL_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "cross-cancel-an-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_isolated_cancel_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("htx", HttpMethod::Post, "/linear-swap-api/v1/swap_cancel")
                .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_cancel",
                checked_at: HTX_ISOLATED_CANCEL_ORDER_CHECKED_AT,
                doc_version: HTX_ISOLATED_CANCEL_ORDER_DOC_VERSION,
                schema_hash: HTX_ISOLATED_CANCEL_ORDER_SCHEMA_HASH,
                fixture_id: HTX_ISOLATED_CANCEL_ORDER_FIXTURE_ID,
                parser_test: HTX_ISOLATED_CANCEL_ORDER_PARSER_TEST,
                request_builder_test: HTX_ISOLATED_CANCEL_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "isolated-cancel-an-order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_get_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_order_info",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_order_info",
                checked_at: HTX_GET_ORDER_CHECKED_AT,
                doc_version: HTX_GET_ORDER_DOC_VERSION,
                schema_hash: HTX_GET_ORDER_SCHEMA_HASH,
                fixture_id: HTX_GET_ORDER_FIXTURE_ID,
                parser_test: HTX_GET_ORDER_PARSER_TEST,
                request_builder_test: HTX_GET_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "get-information-of-an-order");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_cross_get_order_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_order_info",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_cross_order_info",
                checked_at: HTX_CROSS_GET_ORDER_CHECKED_AT,
                doc_version: HTX_CROSS_GET_ORDER_DOC_VERSION,
                schema_hash: HTX_CROSS_GET_ORDER_SCHEMA_HASH,
                fixture_id: HTX_CROSS_GET_ORDER_FIXTURE_ID,
                parser_test: HTX_CROSS_GET_ORDER_PARSER_TEST,
                request_builder_test: HTX_CROSS_GET_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "cross-get-information-of-order");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_open_orders_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_openorders",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_openorders",
                checked_at: HTX_OPEN_ORDERS_CHECKED_AT,
                doc_version: HTX_OPEN_ORDERS_DOC_VERSION,
                schema_hash: HTX_OPEN_ORDERS_SCHEMA_HASH,
                fixture_id: HTX_OPEN_ORDERS_FIXTURE_ID,
                parser_test: HTX_OPEN_ORDERS_PARSER_TEST,
                request_builder_test: HTX_OPEN_ORDERS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "isolated-current-unfilled-order-acquisition");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_cross_open_orders_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_openorders",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_cross_openorders",
                checked_at: HTX_CROSS_OPEN_ORDERS_CHECKED_AT,
                doc_version: HTX_CROSS_OPEN_ORDERS_DOC_VERSION,
                schema_hash: HTX_CROSS_OPEN_ORDERS_SCHEMA_HASH,
                fixture_id: HTX_CROSS_OPEN_ORDERS_FIXTURE_ID,
                parser_test: HTX_CROSS_OPEN_ORDERS_PARSER_TEST,
                request_builder_test: HTX_CROSS_OPEN_ORDERS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "cross-current-unfilled-order-acquisition");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "order_status");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_account_info_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_account_info",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_account_info",
                checked_at: HTX_ACCOUNT_INFO_CHECKED_AT,
                doc_version: HTX_ACCOUNT_INFO_DOC_VERSION,
                schema_hash: HTX_ACCOUNT_INFO_SCHEMA_HASH,
                fixture_id: HTX_ACCOUNT_INFO_FIXTURE_ID,
                parser_test: HTX_ACCOUNT_INFO_PARSER_TEST,
                request_builder_test: HTX_ACCOUNT_INFO_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "isolated-query-user-s-account-information");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_balance");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_cross_account_info_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_account_info",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_cross_account_info",
                checked_at: HTX_CROSS_ACCOUNT_INFO_CHECKED_AT,
                doc_version: HTX_CROSS_ACCOUNT_INFO_DOC_VERSION,
                schema_hash: HTX_CROSS_ACCOUNT_INFO_SCHEMA_HASH,
                fixture_id: HTX_CROSS_ACCOUNT_INFO_FIXTURE_ID,
                parser_test: HTX_CROSS_ACCOUNT_INFO_PARSER_TEST,
                request_builder_test: HTX_CROSS_ACCOUNT_INFO_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "cross-query-user-39-s-account-information");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_balance");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_account_type_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Get,
            "/linear-swap-api/v3/swap_unified_account_type",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/linear-swap-api/v3/swap_unified_account_type",
                checked_at: HTX_ACCOUNT_TYPE_CHECKED_AT,
                doc_version: HTX_ACCOUNT_TYPE_DOC_VERSION,
                schema_hash: HTX_ACCOUNT_TYPE_SCHEMA_HASH,
                fixture_id: HTX_ACCOUNT_TYPE_FIXTURE_ID,
                parser_test: HTX_ACCOUNT_TYPE_PARSER_TEST,
                request_builder_test: HTX_ACCOUNT_TYPE_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "account-type-query");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_config");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_api_trading_status_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Get,
            "/linear-swap-api/v1/swap_api_trading_status",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/linear-swap-api/v1/swap_api_trading_status",
                checked_at: HTX_API_TRADING_STATUS_CHECKED_AT,
                doc_version: HTX_API_TRADING_STATUS_DOC_VERSION,
                schema_hash: HTX_API_TRADING_STATUS_SCHEMA_HASH,
                fixture_id: HTX_API_TRADING_STATUS_FIXTURE_ID,
                parser_test: HTX_API_TRADING_STATUS_PARSER_TEST,
                request_builder_test: HTX_API_TRADING_STATUS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "query-api-trading-status");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_config");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn hyperliquid_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("hyperliquid", HttpMethod::Post, "/exchange").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/exchange",
                checked_at: HYPERLIQUID_PLACE_ORDER_CHECKED_AT,
                doc_version: HYPERLIQUID_PLACE_ORDER_DOC_VERSION,
                schema_hash: HYPERLIQUID_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: HYPERLIQUID_PLACE_ORDER_FIXTURE_ID,
                parser_test: HYPERLIQUID_PLACE_ORDER_PARSER_TEST,
                request_builder_test: HYPERLIQUID_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "exchange-endpoint");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn hyperliquid_operation_evidence_is_exact_and_transport_scoped() {
        let actual = HYPERLIQUID_OPERATION_EVIDENCE
            .iter()
            .map(|entry| {
                (
                    entry.transport,
                    entry.operation,
                    entry.dex_scope,
                    entry.use_case,
                    entry.data_kind,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                (
                    HyperliquidOperationTransport::Info,
                    "metaAndAssetCtxs",
                    HyperliquidDexScope::OptionalPerpDex,
                    EndpointUseCase::Baseline,
                    EndpointDataKind::PerpTicker,
                ),
                (
                    HyperliquidOperationTransport::Info,
                    "openOrders",
                    HyperliquidDexScope::OptionalPerpDex,
                    EndpointUseCase::PrivateRead,
                    EndpointDataKind::OrderStatus,
                ),
                (
                    HyperliquidOperationTransport::Info,
                    "frontendOpenOrders",
                    HyperliquidDexScope::OptionalPerpDex,
                    EndpointUseCase::PrivateRead,
                    EndpointDataKind::OrderStatus,
                ),
                (
                    HyperliquidOperationTransport::Info,
                    "orderStatus",
                    HyperliquidDexScope::NotDexScoped,
                    EndpointUseCase::PrivateRead,
                    EndpointDataKind::OrderStatus,
                ),
                (
                    HyperliquidOperationTransport::Info,
                    "clearinghouseState",
                    HyperliquidDexScope::OptionalPerpDex,
                    EndpointUseCase::PrivateRead,
                    EndpointDataKind::AccountBalance,
                ),
                (
                    HyperliquidOperationTransport::Info,
                    "clearinghouseState",
                    HyperliquidDexScope::OptionalPerpDex,
                    EndpointUseCase::PrivateRead,
                    EndpointDataKind::AccountPosition,
                ),
                (
                    HyperliquidOperationTransport::Info,
                    "spotClearinghouseState",
                    HyperliquidDexScope::Spot,
                    EndpointUseCase::PrivateRead,
                    EndpointDataKind::AccountBalance,
                ),
                (
                    HyperliquidOperationTransport::WebSocket,
                    "allDexsAssetCtxs",
                    HyperliquidDexScope::AllPerpDexes,
                    EndpointUseCase::Baseline,
                    EndpointDataKind::PerpTicker,
                ),
                (
                    HyperliquidOperationTransport::WebSocket,
                    "allDexsClearinghouseState",
                    HyperliquidDexScope::AllPerpDexes,
                    EndpointUseCase::PrivateRead,
                    EndpointDataKind::AccountBalance,
                ),
            ]
        );

        for entry in HYPERLIQUID_OPERATION_EVIDENCE {
            assert!(entry.doc_url.starts_with("https://"));
            for value in [
                entry.meta.schema_hash,
                entry.meta.fixture_id,
                entry.meta.parser_test,
                entry.meta.request_builder_test,
                entry.meta.auth_kind,
            ] {
                assert_ne!(value, UNRECORDED_EVIDENCE_MARKER, "{}", entry.operation);
            }

            match entry.transport {
                HyperliquidOperationTransport::Info => {
                    let specs = ENDPOINT_SPECS
                        .iter()
                        .filter(|spec| entry.matches_endpoint_spec(spec))
                        .collect::<Vec<_>>();
                    assert_eq!(specs.len(), 1, "{}", entry.operation);
                    assert_eq!(specs[0].rate_scope, RateScope::Ip);
                    let evidence = endpoint_evidence_for_spec(specs[0]);
                    assert_eq!(evidence.doc_urls, vec![entry.doc_url.to_owned()]);
                    assert_eq!(evidence.fixture_id, entry.meta.fixture_id);
                }
                HyperliquidOperationTransport::WebSocket => assert!(
                    !ENDPOINT_SPECS.iter().any(|spec| {
                        spec.venue == VenueId::Hyperliquid
                            && spec.method == HttpMethod::Post
                            && spec.path == "/info"
                            && spec.doc_url == entry.doc_url
                    }),
                    "{} must remain WebSocket-only",
                    entry.operation
                ),
            }
        }
    }

    #[test]
    fn hyperliquid_info_operation_request_contracts_are_distinct() {
        let user = "0x0123456789abcdef0123456789abcdef01234567";
        let requests = vec![
            (
                "/info",
                "metaAndAssetCtxs",
                HyperliquidDexScope::OptionalPerpDex,
                serde_json::json!({"type": "metaAndAssetCtxs", "dex": "xyz"}),
            ),
            (
                "/info",
                "openOrders",
                HyperliquidDexScope::OptionalPerpDex,
                serde_json::json!({"type": "openOrders", "user": user, "dex": "xyz"}),
            ),
            (
                "/info",
                "frontendOpenOrders",
                HyperliquidDexScope::OptionalPerpDex,
                serde_json::json!({"type": "frontendOpenOrders", "user": user, "dex": "xyz"}),
            ),
            (
                "/info",
                "orderStatus",
                HyperliquidDexScope::NotDexScoped,
                serde_json::json!({"type": "orderStatus", "user": user, "oid": 77}),
            ),
            (
                "/info",
                "clearinghouseState",
                HyperliquidDexScope::OptionalPerpDex,
                serde_json::json!({"type": "clearinghouseState", "user": user, "dex": "xyz"}),
            ),
            (
                "/info",
                "spotClearinghouseState",
                HyperliquidDexScope::Spot,
                serde_json::json!({"type": "spotClearinghouseState", "user": user}),
            ),
        ];

        for (path, operation, dex_scope, request) in requests {
            assert_eq!(path, "/info");
            assert_eq!(
                request.get("type").and_then(serde_json::Value::as_str),
                Some(operation)
            );
            match dex_scope {
                HyperliquidDexScope::OptionalPerpDex => assert_eq!(
                    request.get("dex").and_then(serde_json::Value::as_str),
                    Some("xyz")
                ),
                HyperliquidDexScope::NotDexScoped | HyperliquidDexScope::Spot => {
                    assert!(request.get("dex").is_none())
                }
                HyperliquidDexScope::AllPerpDexes => unreachable!("WebSocket operation"),
            }
        }
    }

    #[test]
    fn hyperliquid_ws_operation_request_contracts_remain_ws_only() {
        let user = "0x0123456789abcdef0123456789abcdef01234567";
        let subscriptions = vec![
            (
                "allDexsAssetCtxs",
                serde_json::json!({
                    "method": "subscribe",
                    "subscription": {"type": "allDexsAssetCtxs"},
                }),
            ),
            (
                "allDexsClearinghouseState",
                serde_json::json!({
                    "method": "subscribe",
                    "subscription": {"type": "allDexsClearinghouseState", "user": user},
                }),
            ),
        ];

        for (operation, request) in subscriptions {
            let subscription = request
                .get("subscription")
                .and_then(serde_json::Value::as_object)
                .expect("subscription payload");
            assert_eq!(
                request.get("method").and_then(serde_json::Value::as_str),
                Some("subscribe")
            );
            assert_eq!(
                subscription.get("type").and_then(serde_json::Value::as_str),
                Some(operation)
            );
            if operation == "allDexsClearinghouseState" {
                assert_eq!(
                    subscription.get("user").and_then(serde_json::Value::as_str),
                    Some(user)
                );
            } else {
                assert!(subscription.get("user").is_none());
            }

            let evidence = HYPERLIQUID_OPERATION_EVIDENCE
                .iter()
                .find(|entry| entry.operation == operation)
                .expect("WebSocket operation evidence");
            assert_eq!(evidence.transport, HyperliquidOperationTransport::WebSocket);
            assert_eq!(evidence.dex_scope, HyperliquidDexScope::AllPerpDexes);
            assert_eq!(
                evidence.meta.request_builder_test,
                HYPERLIQUID_WS_OPERATION_REQUEST_TEST
            );
        }
    }

    #[test]
    fn hyperliquid_operation_fixtures_parse_and_keep_dex_scope() {
        assert_hyperliquid_operation_fixture(
            "metaAndAssetCtxs",
            include_str!("../fixtures/hyperliquid/meta_and_asset_ctxs_btc_eth.json"),
            assert_meta_and_asset_ctxs_fixture,
        );
        assert_hyperliquid_operation_fixture(
            "openOrders",
            include_str!("../fixtures/hyperliquid/info_open_orders_dex.json"),
            assert_open_orders_fixture,
        );
        assert_hyperliquid_operation_fixture(
            "frontendOpenOrders",
            include_str!("../fixtures/hyperliquid/info_frontend_open_orders_dex.json"),
            assert_frontend_open_orders_fixture,
        );
        assert_hyperliquid_operation_fixture(
            "orderStatus",
            include_str!("../fixtures/hyperliquid/info_order_status_filled.json"),
            assert_order_status_fixture,
        );
        assert_hyperliquid_operation_fixture(
            "clearinghouseState",
            include_str!("../fixtures/hyperliquid/info_clearinghouse_state_account_balance.json"),
            assert_clearinghouse_state_fixture,
        );
        assert_hyperliquid_operation_fixture(
            "spotClearinghouseState",
            include_str!(
                "../fixtures/hyperliquid/info_spot_clearinghouse_state_account_balance.json"
            ),
            assert_spot_clearinghouse_state_fixture,
        );
        assert_hyperliquid_operation_fixture(
            "allDexsAssetCtxs",
            include_str!("../fixtures/hyperliquid/ws_all_dexs_asset_ctxs_evidence.json"),
            assert_all_dexs_asset_ctxs_fixture,
        );
        assert_hyperliquid_operation_fixture(
            "allDexsClearinghouseState",
            include_str!("../fixtures/hyperliquid/ws_all_dexs_clearinghouse_evidence.json"),
            assert_all_dexs_clearinghouse_state_fixture,
        );
    }

    fn assert_hyperliquid_operation_fixture(
        operation: &str,
        fixture_source: &str,
        assert_shape: impl FnOnce(&serde_json::Value),
    ) {
        let fixture: serde_json::Value = serde_json::from_str(fixture_source)
            .unwrap_or_else(|error| panic!("{operation}: {error}"));
        assert_shape(&fixture);
    }

    #[test]
    fn order_status_official_envelope_parses_filled_order() {
        assert_hyperliquid_operation_fixture(
            "orderStatus",
            include_str!("../fixtures/hyperliquid/info_order_status_filled.json"),
            assert_order_status_fixture,
        );
    }

    fn assert_meta_and_asset_ctxs_fixture(fixture: &serde_json::Value) {
        assert_eq!(fixture.as_array().map(Vec::len), Some(2));
    }

    fn assert_open_orders_fixture(fixture: &serde_json::Value) {
        let order = first_fixture_order(fixture, "open order");
        assert!(order.get("origSz").is_none());
        assert!(order.get("orderType").is_none());
    }

    fn assert_frontend_open_orders_fixture(fixture: &serde_json::Value) {
        let order = first_fixture_order(fixture, "frontend open order");
        assert_eq!(
            order.get("origSz").and_then(serde_json::Value::as_str),
            Some("5.0")
        );
        assert!(order.get("orderType").is_some());
    }

    fn first_fixture_order<'a>(
        fixture: &'a serde_json::Value,
        label: &str,
    ) -> &'a serde_json::Value {
        fixture
            .as_array()
            .and_then(|orders| orders.first())
            .unwrap_or_else(|| panic!("{label}"))
    }

    fn assert_order_status_fixture(fixture: &serde_json::Value) {
        assert_eq!(
            fixture.get("status").and_then(serde_json::Value::as_str),
            Some("order")
        );
        assert!(fixture.get("order").is_some());
    }

    fn assert_clearinghouse_state_fixture(fixture: &serde_json::Value) {
        assert!(fixture.get("marginSummary").is_some());
        assert!(fixture.get("assetPositions").is_some());
    }

    fn assert_spot_clearinghouse_state_fixture(fixture: &serde_json::Value) {
        assert!(fixture.get("balances").is_some());
    }

    fn assert_all_dexs_asset_ctxs_fixture(fixture: &serde_json::Value) {
        let contexts = fixture
            .get("ctxs")
            .and_then(serde_json::Value::as_object)
            .expect("all-dex contexts");
        assert!(contexts.contains_key(""));
        assert!(contexts.contains_key("xyz"));
    }

    fn assert_all_dexs_clearinghouse_state_fixture(fixture: &serde_json::Value) {
        let states = fixture
            .get("clearinghouseStates")
            .and_then(serde_json::Value::as_object)
            .expect("all-dex clearinghouse states");
        assert!(states.contains_key(""));
        assert!(states.contains_key("xyz"));
    }

    #[test]
    fn binance_account_position_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("binance", HttpMethod::Get, "/fapi/v3/positionRisk")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/fapi/v3/positionRisk",
                checked_at: BINANCE_USDM_POSITIONS_CHECKED_AT,
                doc_version: BINANCE_USDM_POSITIONS_DOC_VERSION,
                schema_hash: BINANCE_USDM_POSITIONS_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_POSITIONS_FIXTURE_ID,
                parser_test: BINANCE_USDM_POSITIONS_PARSER_TEST,
                request_builder_test: BINANCE_USDM_POSITIONS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 5,
            },
        );
        assert_url_matches(&evidence, "Position-Information-V3");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_position");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn okx_account_position_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("okx", HttpMethod::Get, "/api/v5/account/positions")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v5/account/positions",
                checked_at: OKX_ACCOUNT_POSITIONS_CHECKED_AT,
                doc_version: OKX_ACCOUNT_POSITIONS_DOC_VERSION,
                schema_hash: OKX_ACCOUNT_POSITIONS_SCHEMA_HASH,
                fixture_id: OKX_ACCOUNT_POSITIONS_FIXTURE_ID,
                parser_test: OKX_ACCOUNT_POSITIONS_PARSER_TEST,
                request_builder_test: OKX_ACCOUNT_POSITIONS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "get-positions");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_position");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bybit_account_position_evidence_registry_has_private_read_metadata() {
        let entry = find_recorded_entry(
            VenueId::Bybit,
            HttpMethod::Get,
            "/v5/position/list",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::AccountPosition,
        );
        let spec = find_endpoint_spec(
            VenueId::Bybit,
            HttpMethod::Get,
            "/v5/position/list",
            EndpointUseCase::PrivateRead,
            EndpointDataKind::AccountPosition,
        );

        assert_eq!(entry.meta.checked_at, BYBIT_POSITIONS_CHECKED_AT);
        assert_eq!(entry.meta.doc_version, BYBIT_POSITIONS_DOC_VERSION);
        assert_eq!(entry.meta.schema_hash, BYBIT_POSITIONS_SCHEMA_HASH);
        assert_eq!(entry.meta.fixture_id, BYBIT_POSITIONS_FIXTURE_ID);
        assert_eq!(entry.meta.parser_test, BYBIT_POSITIONS_PARSER_TEST);
        assert_eq!(
            entry.meta.request_builder_test,
            BYBIT_POSITIONS_REQUEST_TEST
        );
        assert_eq!(entry.meta.auth_kind, SIGNED_AUTH_KIND);
        assert_eq!(spec.weight, 1);
        assert_eq!(spec.rate_scope, RateScope::Account);
        assert!(spec.doc_url.ends_with("/position"));

        let evidence =
            endpoint_evidence("bybit", HttpMethod::Get, "/v5/position/list").expect("aggregate");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_position");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn bitget_account_position_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "bitget",
            HttpMethod::Get,
            "/api/v3/position/current-position",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v3/position/current-position",
                checked_at: BITGET_CURRENT_POSITION_CHECKED_AT,
                doc_version: BITGET_CURRENT_POSITION_DOC_VERSION,
                schema_hash: BITGET_CURRENT_POSITION_SCHEMA_HASH,
                fixture_id: BITGET_CURRENT_POSITION_FIXTURE_ID,
                parser_test: BITGET_CURRENT_POSITION_PARSER_TEST,
                request_builder_test: BITGET_CURRENT_POSITION_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "Get-Position");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_position");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn gate_account_position_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("gate", HttpMethod::Get, "/api/v4/futures/usdt/positions")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v4/futures/usdt/positions",
                checked_at: GATE_POSITIONS_CHECKED_AT,
                doc_version: GATE_POSITIONS_DOC_VERSION,
                schema_hash: GATE_POSITIONS_SCHEMA_HASH,
                fixture_id: GATE_POSITIONS_FIXTURE_ID,
                parser_test: GATE_POSITIONS_PARSER_TEST,
                request_builder_test: GATE_POSITIONS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "list-positions");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_position");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn kucoin_account_position_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/positions").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v1/positions",
                checked_at: KUCOIN_POSITIONS_CHECKED_AT,
                doc_version: KUCOIN_POSITIONS_DOC_VERSION,
                schema_hash: KUCOIN_POSITIONS_SCHEMA_HASH,
                fixture_id: KUCOIN_POSITIONS_FIXTURE_ID,
                parser_test: KUCOIN_POSITIONS_PARSER_TEST,
                request_builder_test: KUCOIN_POSITIONS_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 2,
            },
        );
        assert_url_matches(&evidence, "get-position-list");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_position");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_account_position_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_account_position_info",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_account_position_info",
                checked_at: HTX_ACCOUNT_POSITION_CHECKED_AT,
                doc_version: HTX_ACCOUNT_POSITION_DOC_VERSION,
                schema_hash: HTX_ACCOUNT_POSITION_SCHEMA_HASH,
                fixture_id: HTX_ACCOUNT_POSITION_FIXTURE_ID,
                parser_test: HTX_ACCOUNT_POSITION_PARSER_TEST,
                request_builder_test: HTX_ACCOUNT_POSITION_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "assets-and-positions");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_position");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn htx_cross_account_position_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Post,
            "/linear-swap-api/v1/swap_cross_account_position_info",
        )
        .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/linear-swap-api/v1/swap_cross_account_position_info",
                checked_at: HTX_CROSS_ACCOUNT_POSITION_CHECKED_AT,
                doc_version: HTX_CROSS_ACCOUNT_POSITION_DOC_VERSION,
                schema_hash: HTX_CROSS_ACCOUNT_POSITION_SCHEMA_HASH,
                fixture_id: HTX_CROSS_ACCOUNT_POSITION_FIXTURE_ID,
                parser_test: HTX_CROSS_ACCOUNT_POSITION_PARSER_TEST,
                request_builder_test: HTX_CROSS_ACCOUNT_POSITION_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "cross-query-assets-and-positions");
        assert_list_contains(&evidence.use_cases, "private_read");
        assert_list_contains(&evidence.data_kinds, "account_position");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn binance_place_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("binance", HttpMethod::Post, "/fapi/v1/order").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "POST",
                path: "/fapi/v1/order",
                checked_at: BINANCE_USDM_PLACE_ORDER_CHECKED_AT,
                doc_version: BINANCE_USDM_PLACE_ORDER_DOC_VERSION,
                schema_hash: BINANCE_USDM_PLACE_ORDER_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_PLACE_ORDER_FIXTURE_ID,
                parser_test: BINANCE_USDM_PLACE_ORDER_PARSER_TEST,
                request_builder_test: BINANCE_USDM_PLACE_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "/New-Order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn binance_cancel_order_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("binance", HttpMethod::Delete, "/fapi/v1/order").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "DELETE",
                path: "/fapi/v1/order",
                checked_at: BINANCE_USDM_CANCEL_ORDER_CHECKED_AT,
                doc_version: BINANCE_USDM_CANCEL_ORDER_DOC_VERSION,
                schema_hash: BINANCE_USDM_CANCEL_ORDER_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_CANCEL_ORDER_FIXTURE_ID,
                parser_test: BINANCE_USDM_CANCEL_ORDER_PARSER_TEST,
                request_builder_test: BINANCE_USDM_CANCEL_ORDER_REQUEST_TEST,
                auth_kind: SIGNED_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "/Cancel-Order");
        assert_list_contains(&evidence.use_cases, "trade_write");
        assert_list_contains(&evidence.data_kinds, "order_ack");
        assert_list_contains(&evidence.rate_scopes, "account");
    }

    #[test]
    fn binance_spot_ticker_24hr_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("binance", HttpMethod::Get, "/api/v3/ticker/24hr").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v3/ticker/24hr");
        assert_eq!(evidence.checked_at, BINANCE_SPOT_TICKER_24HR_CHECKED_AT);
        assert_eq!(evidence.doc_version, BINANCE_SPOT_TICKER_24HR_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BINANCE_SPOT_TICKER_24HR_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BINANCE_SPOT_TICKER_24HR_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BINANCE_SPOT_TICKER_24HR_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            BINANCE_SPOT_TICKER_24HR_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 80);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#24hr-ticker-price-change-statistics")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"spot_ticker".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn binance_premium_index_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/premiumIndex")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/fapi/v1/premiumIndex",
                checked_at: BINANCE_USDM_PREMIUM_INDEX_CHECKED_AT,
                doc_version: BINANCE_USDM_PREMIUM_INDEX_DOC_VERSION,
                schema_hash: BINANCE_USDM_PREMIUM_INDEX_SCHEMA_HASH,
                fixture_id: BINANCE_USDM_PREMIUM_INDEX_FIXTURE_ID,
                parser_test: BINANCE_USDM_PREMIUM_INDEX_PARSER_TEST,
                request_builder_test: BINANCE_USDM_PREMIUM_INDEX_REQUEST_TEST,
                auth_kind: PUBLIC_AUTH_KIND,
                weight: 10,
            },
        );
        assert_url_matches(&evidence, "/Mark-Price");
        assert_list_contains(&evidence.use_cases, "baseline");
        assert_list_contains(&evidence.data_kinds, "funding_rate");
        assert_list_contains(&evidence.data_kinds, "mark_index");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn binance_open_interest_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/openInterest")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/fapi/v1/openInterest");
        assert_eq!(evidence.checked_at, BINANCE_USDM_OPEN_INTEREST_CHECKED_AT);
        assert_eq!(evidence.doc_version, BINANCE_USDM_OPEN_INTEREST_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BINANCE_USDM_OPEN_INTEREST_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BINANCE_USDM_OPEN_INTEREST_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BINANCE_USDM_OPEN_INTEREST_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            BINANCE_USDM_OPEN_INTEREST_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/Open-Interest")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"open_interest".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn binance_usdm_ticker_24hr_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("binance", HttpMethod::Get, "/fapi/v1/ticker/24hr")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/fapi/v1/ticker/24hr");
        assert_eq!(evidence.checked_at, BINANCE_USDM_TICKER_24HR_CHECKED_AT);
        assert_eq!(evidence.doc_version, BINANCE_USDM_TICKER_24HR_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BINANCE_USDM_TICKER_24HR_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BINANCE_USDM_TICKER_24HR_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BINANCE_USDM_TICKER_24HR_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            BINANCE_USDM_TICKER_24HR_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 40);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/24hr-Ticker-Price-Change-Statistics")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"perp_ticker".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn okx_market_books_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("okx", HttpMethod::Get, "/api/v5/market/books").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v5/market/books");
        assert_eq!(evidence.checked_at, OKX_MARKET_BOOKS_CHECKED_AT);
        assert_eq!(evidence.doc_version, OKX_MARKET_BOOKS_DOC_VERSION);
        assert_eq!(evidence.schema_hash, OKX_MARKET_BOOKS_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, OKX_MARKET_BOOKS_FIXTURE_ID);
        assert_eq!(evidence.parser_test, OKX_MARKET_BOOKS_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, OKX_MARKET_BOOKS_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#order-book-trading-market-data-get-order-book")));
        assert!(evidence.use_cases.contains(&"hot_path_fallback".to_owned()));
        assert!(evidence.data_kinds.contains(&"order_book".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn okx_public_time_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("okx", HttpMethod::Get, "/api/v5/public/time").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v5/public/time");
        assert_eq!(evidence.checked_at, OKX_PUBLIC_TIME_CHECKED_AT);
        assert_eq!(evidence.doc_version, OKX_PUBLIC_TIME_DOC_VERSION);
        assert_eq!(evidence.schema_hash, OKX_PUBLIC_TIME_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, OKX_PUBLIC_TIME_FIXTURE_ID);
        assert_eq!(evidence.parser_test, OKX_PUBLIC_TIME_TEST);
        assert_eq!(evidence.request_builder_test, OKX_PUBLIC_TIME_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#public-data-rest-api-get-system-time")));
        assert!(evidence.use_cases.contains(&"calibration".to_owned()));
        assert!(evidence.data_kinds.contains(&"server_time".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn okx_public_instruments_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("okx", HttpMethod::Get, "/api/v5/public/instruments")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v5/public/instruments");
        assert_eq!(evidence.checked_at, OKX_PUBLIC_INSTRUMENTS_CHECKED_AT);
        assert_eq!(evidence.doc_version, OKX_PUBLIC_INSTRUMENTS_DOC_VERSION);
        assert_eq!(evidence.schema_hash, OKX_PUBLIC_INSTRUMENTS_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, OKX_PUBLIC_INSTRUMENTS_FIXTURE_ID);
        assert_eq!(evidence.parser_test, OKX_PUBLIC_INSTRUMENTS_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            OKX_PUBLIC_INSTRUMENTS_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#public-data-rest-api-get-instruments")));
        assert!(evidence.use_cases.contains(&"metadata".to_owned()));
        assert!(evidence
            .data_kinds
            .contains(&"instrument_metadata".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn okx_market_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("okx", HttpMethod::Get, "/api/v5/market/tickers").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v5/market/tickers",
                checked_at: OKX_MARKET_TICKERS_CHECKED_AT,
                doc_version: OKX_MARKET_TICKERS_DOC_VERSION,
                schema_hash: OKX_MARKET_TICKERS_SCHEMA_HASH,
                fixture_id: OKX_MARKET_TICKERS_FIXTURE_ID,
                parser_test: OKX_MARKET_TICKERS_PARSER_TEST,
                request_builder_test: OKX_MARKET_TICKERS_REQUEST_TEST,
                auth_kind: PUBLIC_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "#order-book-trading-market-data-get-tickers");
        assert_list_contains(&evidence.use_cases, "baseline");
        assert_list_contains(&evidence.data_kinds, "perp_ticker");
        assert_list_contains(&evidence.data_kinds, "spot_ticker");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn okx_funding_rate_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("okx", HttpMethod::Get, "/api/v5/public/funding-rate")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v5/public/funding-rate");
        assert_eq!(evidence.checked_at, OKX_FUNDING_RATE_CHECKED_AT);
        assert_eq!(evidence.doc_version, OKX_FUNDING_RATE_DOC_VERSION);
        assert_eq!(evidence.schema_hash, OKX_PUBLIC_BASELINE_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, OKX_PUBLIC_BASELINE_FIXTURE_ID);
        assert_eq!(evidence.parser_test, OKX_FUNDING_RATE_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, OKX_FUNDING_RATE_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#public-data-rest-api-get-funding-rate")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"funding_rate".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn okx_mark_price_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("okx", HttpMethod::Get, "/api/v5/public/mark-price")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v5/public/mark-price");
        assert_eq!(evidence.checked_at, OKX_PUBLIC_MARK_INDEX_CHECKED_AT);
        assert_eq!(evidence.doc_version, OKX_MARK_PRICE_DOC_VERSION);
        assert_eq!(evidence.schema_hash, OKX_PUBLIC_BASELINE_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, OKX_PUBLIC_BASELINE_FIXTURE_ID);
        assert_eq!(evidence.parser_test, OKX_MARK_INDEX_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, OKX_MARK_INDEX_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#public-data-rest-api-get-mark-price")));
        assert!(evidence.data_kinds.contains(&"mark_index".to_owned()));
    }

    #[test]
    fn okx_index_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("okx", HttpMethod::Get, "/api/v5/market/index-tickers")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v5/market/index-tickers");
        assert_eq!(evidence.checked_at, OKX_PUBLIC_MARK_INDEX_CHECKED_AT);
        assert_eq!(evidence.doc_version, OKX_INDEX_TICKERS_DOC_VERSION);
        assert_eq!(evidence.schema_hash, OKX_PUBLIC_BASELINE_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, OKX_PUBLIC_BASELINE_FIXTURE_ID);
        assert_eq!(evidence.parser_test, OKX_MARK_INDEX_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, OKX_MARK_INDEX_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#public-data-rest-api-get-index-tickers")));
        assert!(evidence.data_kinds.contains(&"mark_index".to_owned()));
    }

    #[test]
    fn okx_open_interest_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("okx", HttpMethod::Get, "/api/v5/public/open-interest")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v5/public/open-interest");
        assert_eq!(evidence.checked_at, OKX_PUBLIC_MARK_INDEX_CHECKED_AT);
        assert_eq!(evidence.doc_version, OKX_OPEN_INTEREST_DOC_VERSION);
        assert_eq!(evidence.schema_hash, OKX_PUBLIC_BASELINE_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, OKX_PUBLIC_BASELINE_FIXTURE_ID);
        assert_eq!(evidence.parser_test, OKX_MARK_INDEX_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, OKX_MARK_INDEX_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#public-data-rest-api-get-open-interest")));
        assert!(evidence.data_kinds.contains(&"open_interest".to_owned()));
    }

    #[test]
    fn bybit_server_time_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bybit", HttpMethod::Get, "/v5/market/time").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/v5/market/time");
        assert_eq!(evidence.checked_at, BYBIT_SERVER_TIME_CHECKED_AT);
        assert_eq!(evidence.doc_version, BYBIT_SERVER_TIME_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BYBIT_SERVER_TIME_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BYBIT_SERVER_TIME_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BYBIT_SERVER_TIME_TEST);
        assert_eq!(evidence.request_builder_test, BYBIT_SERVER_TIME_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence.doc_urls.iter().any(|url| url.ends_with("/time")));
        assert!(evidence.use_cases.contains(&"calibration".to_owned()));
        assert!(evidence.data_kinds.contains(&"server_time".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn bybit_orderbook_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bybit", HttpMethod::Get, "/v5/market/orderbook").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/v5/market/orderbook");
        assert_eq!(evidence.checked_at, BYBIT_ORDERBOOK_CHECKED_AT);
        assert_eq!(evidence.doc_version, BYBIT_ORDERBOOK_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BYBIT_ORDERBOOK_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BYBIT_ORDERBOOK_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BYBIT_ORDERBOOK_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, BYBIT_ORDERBOOK_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/orderbook")));
        assert!(evidence.use_cases.contains(&"hot_path_fallback".to_owned()));
        assert!(evidence.data_kinds.contains(&"order_book".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn bybit_instruments_info_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bybit", HttpMethod::Get, "/v5/market/instruments-info")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/v5/market/instruments-info");
        assert_eq!(evidence.checked_at, BYBIT_INSTRUMENTS_CHECKED_AT);
        assert_eq!(evidence.doc_version, BYBIT_INSTRUMENTS_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BYBIT_INSTRUMENTS_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BYBIT_INSTRUMENTS_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BYBIT_INSTRUMENTS_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            BYBIT_INSTRUMENTS_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/instrument")));
        assert!(evidence.use_cases.contains(&"metadata".to_owned()));
        assert!(evidence
            .data_kinds
            .contains(&"instrument_metadata".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn bybit_market_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bybit", HttpMethod::Get, "/v5/market/tickers").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/v5/market/tickers",
                checked_at: BYBIT_MARKET_TICKERS_CHECKED_AT,
                doc_version: BYBIT_MARKET_TICKERS_DOC_VERSION,
                schema_hash: BYBIT_MARKET_TICKERS_SCHEMA_HASH,
                fixture_id: BYBIT_MARKET_TICKERS_FIXTURE_ID,
                parser_test: BYBIT_MARKET_TICKERS_PARSER_TEST,
                request_builder_test: BYBIT_MARKET_TICKERS_REQUEST_TEST,
                auth_kind: PUBLIC_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "/tickers");
        assert_list_contains(&evidence.use_cases, "baseline");
        assert_list_contains(&evidence.data_kinds, "perp_ticker");
        assert_list_contains(&evidence.data_kinds, "funding_rate");
        assert_list_contains(&evidence.data_kinds, "spot_ticker");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn bitget_server_time_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("bitget", HttpMethod::Get, "/api/v2/public/time").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v2/public/time");
        assert_eq!(evidence.checked_at, BITGET_SERVER_TIME_CHECKED_AT);
        assert_eq!(evidence.doc_version, BITGET_SERVER_TIME_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BITGET_SERVER_TIME_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BITGET_SERVER_TIME_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BITGET_SERVER_TIME_TEST);
        assert_eq!(evidence.request_builder_test, BITGET_SERVER_TIME_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/Get-Server-Time")));
        assert!(evidence.use_cases.contains(&"calibration".to_owned()));
        assert!(evidence.data_kinds.contains(&"server_time".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn bitget_uta_orderbook_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bitget", HttpMethod::Get, "/api/v3/market/orderbook")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v3/market/orderbook");
        assert_eq!(evidence.checked_at, BITGET_UTA_ORDERBOOK_CHECKED_AT);
        assert_eq!(evidence.doc_version, BITGET_UTA_ORDERBOOK_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BITGET_UTA_ORDERBOOK_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BITGET_UTA_ORDERBOOK_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BITGET_UTA_ORDERBOOK_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            BITGET_UTA_ORDERBOOK_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/OrderBook")));
        assert!(evidence.use_cases.contains(&"hot_path_fallback".to_owned()));
        assert!(evidence.data_kinds.contains(&"order_book".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn bitget_uta_instruments_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bitget", HttpMethod::Get, "/api/v3/market/instruments")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v3/market/instruments");
        assert_eq!(evidence.checked_at, BITGET_UTA_INSTRUMENTS_CHECKED_AT);
        assert_eq!(evidence.doc_version, BITGET_UTA_INSTRUMENTS_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BITGET_UTA_INSTRUMENTS_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BITGET_UTA_INSTRUMENTS_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BITGET_UTA_INSTRUMENTS_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            BITGET_UTA_INSTRUMENTS_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/Instruments")));
        assert!(evidence.use_cases.contains(&"metadata".to_owned()));
        assert!(evidence
            .data_kinds
            .contains(&"instrument_metadata".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn bitget_uta_current_funding_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "bitget",
            HttpMethod::Get,
            "/api/v3/market/current-fund-rate",
        )
        .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v3/market/current-fund-rate");
        assert_eq!(evidence.checked_at, BITGET_UTA_CURRENT_FUNDING_CHECKED_AT);
        assert_eq!(evidence.doc_version, BITGET_UTA_CURRENT_FUNDING_DOC_VERSION);
        assert_eq!(evidence.schema_hash, BITGET_UTA_CURRENT_FUNDING_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, BITGET_UTA_CURRENT_FUNDING_FIXTURE_ID);
        assert_eq!(evidence.parser_test, BITGET_UTA_CURRENT_FUNDING_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            BITGET_UTA_CURRENT_FUNDING_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/Get-Current-Funding-Rate")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"funding_rate".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn bitget_uta_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("bitget", HttpMethod::Get, "/api/v3/market/tickers")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v3/market/tickers",
                checked_at: BITGET_UTA_TICKERS_CHECKED_AT,
                doc_version: BITGET_UTA_TICKERS_DOC_VERSION,
                schema_hash: BITGET_UTA_TICKERS_SCHEMA_HASH,
                fixture_id: BITGET_UTA_TICKERS_FIXTURE_ID,
                parser_test: BITGET_UTA_TICKERS_PARSER_TEST,
                request_builder_test: BITGET_UTA_TICKERS_REQUEST_TEST,
                auth_kind: PUBLIC_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "/Tickers");
        assert_list_contains(&evidence.use_cases, "baseline");
        assert_list_contains(&evidence.data_kinds, "perp_ticker");
        assert_list_contains(&evidence.data_kinds, "spot_ticker");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn gate_server_time_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("gate", HttpMethod::Get, "/api/v4/spot/time").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v4/spot/time");
        assert_eq!(evidence.checked_at, GATE_SERVER_TIME_CHECKED_AT);
        assert_eq!(evidence.doc_version, GATE_SERVER_TIME_DOC_VERSION);
        assert_eq!(evidence.schema_hash, GATE_SERVER_TIME_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, GATE_SERVER_TIME_FIXTURE_ID);
        assert_eq!(evidence.parser_test, GATE_SERVER_TIME_TEST);
        assert_eq!(evidence.request_builder_test, GATE_SERVER_TIME_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#get-server-current-time")));
        assert!(evidence.use_cases.contains(&"calibration".to_owned()));
        assert!(evidence.data_kinds.contains(&"server_time".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn gate_contracts_evidence_uses_recorded_fixture_metadata_and_funding() {
        let evidence = endpoint_evidence("gate", HttpMethod::Get, "/api/v4/futures/usdt/contracts")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v4/futures/usdt/contracts",
                checked_at: GATE_CONTRACTS_CHECKED_AT,
                doc_version: GATE_CONTRACTS_DOC_VERSION,
                schema_hash: GATE_CONTRACTS_SCHEMA_HASH,
                fixture_id: GATE_CONTRACTS_FIXTURE_ID,
                parser_test: GATE_CONTRACTS_PARSER_TEST,
                request_builder_test: GATE_CONTRACTS_REQUEST_TEST,
                auth_kind: PUBLIC_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "#list-futures-contracts");
        assert_list_contains(&evidence.use_cases, "metadata");
        assert_list_contains(&evidence.use_cases, "baseline");
        assert_list_contains(&evidence.data_kinds, "instrument_metadata");
        assert_list_contains(&evidence.data_kinds, "funding_rate");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn gate_orderbook_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("gate", HttpMethod::Get, "/api/v4/futures/usdt/order_book")
                .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v4/futures/usdt/order_book");
        assert_eq!(evidence.checked_at, GATE_ORDERBOOK_CHECKED_AT);
        assert_eq!(evidence.doc_version, GATE_ORDERBOOK_DOC_VERSION);
        assert_eq!(evidence.schema_hash, GATE_ORDERBOOK_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, GATE_ORDERBOOK_FIXTURE_ID);
        assert_eq!(evidence.parser_test, GATE_ORDERBOOK_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, GATE_ORDERBOOK_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.contains("query-futures-market-depth-information")));
        assert!(evidence.use_cases.contains(&"hot_path_fallback".to_owned()));
        assert!(evidence.data_kinds.contains(&"order_book".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn gate_futures_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("gate", HttpMethod::Get, "/api/v4/futures/usdt/tickers")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v4/futures/usdt/tickers");
        assert_eq!(evidence.checked_at, GATE_FUTURES_TICKERS_CHECKED_AT);
        assert_eq!(evidence.doc_version, GATE_FUTURES_TICKERS_DOC_VERSION);
        assert_eq!(evidence.schema_hash, GATE_FUTURES_TICKERS_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, GATE_FUTURES_TICKERS_FIXTURE_ID);
        assert_eq!(evidence.parser_test, GATE_FUTURES_TICKERS_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            GATE_FUTURES_TICKERS_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#list-futures-tickers")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"perp_ticker".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn gate_spot_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("gate", HttpMethod::Get, "/api/v4/spot/tickers").expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v4/spot/tickers",
                checked_at: GATE_SPOT_TICKERS_CHECKED_AT,
                doc_version: GATE_SPOT_TICKERS_DOC_VERSION,
                schema_hash: GATE_SPOT_TICKERS_SCHEMA_HASH,
                fixture_id: GATE_SPOT_TICKERS_FIXTURE_ID,
                parser_test: GATE_SPOT_TICKERS_PARSER_TEST,
                request_builder_test: GATE_SPOT_TICKERS_REQUEST_TEST,
                auth_kind: PUBLIC_AUTH_KIND,
                weight: 1,
            },
        );
        assert_url_matches(&evidence, "#list-spot-tickers");
        assert_list_contains(&evidence.use_cases, "baseline");
        assert_list_contains(&evidence.data_kinds, "spot_ticker");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn htx_server_time_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("htx", HttpMethod::Get, "/api/v1/timestamp").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v1/timestamp");
        assert_eq!(evidence.checked_at, HTX_SERVER_TIME_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_SERVER_TIME_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_SERVER_TIME_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_SERVER_TIME_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_SERVER_TIME_TEST);
        assert_eq!(evidence.request_builder_test, HTX_SERVER_TIME_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#get-current-system-timestamp")));
        assert!(evidence.use_cases.contains(&"calibration".to_owned()));
        assert!(evidence.data_kinds.contains(&"server_time".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn htx_market_depth_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("htx", HttpMethod::Get, "/linear-swap-ex/market/depth")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/linear-swap-ex/market/depth");
        assert_eq!(evidence.checked_at, HTX_MARKET_DEPTH_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_MARKET_DEPTH_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_MARKET_DEPTH_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_MARKET_DEPTH_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_MARKET_DEPTH_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, HTX_MARKET_DEPTH_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#general-get-market-depth")));
        assert!(evidence.use_cases.contains(&"hot_path_fallback".to_owned()));
        assert!(evidence.data_kinds.contains(&"order_book".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn htx_contract_info_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Get,
            "/linear-swap-api/v1/swap_contract_info",
        )
        .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/linear-swap-api/v1/swap_contract_info");
        assert_eq!(evidence.checked_at, HTX_CONTRACT_INFO_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_CONTRACT_INFO_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_CONTRACT_INFO_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_CONTRACT_INFO_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_CONTRACT_INFO_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            HTX_CONTRACT_INFO_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#general-query-swap-info")));
        assert!(evidence.use_cases.contains(&"metadata".to_owned()));
        assert!(evidence
            .data_kinds
            .contains(&"instrument_metadata".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn htx_batch_funding_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Get,
            "/linear-swap-api/v1/swap_batch_funding_rate",
        )
        .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/linear-swap-api/v1/swap_batch_funding_rate");
        assert_eq!(evidence.checked_at, HTX_BATCH_FUNDING_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_BATCH_FUNDING_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_BATCH_FUNDING_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_BATCH_FUNDING_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_BATCH_FUNDING_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            HTX_BATCH_FUNDING_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#general-query-a-batch-of-funding-rate")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"funding_rate".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn htx_market_detail_merged_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Get,
            "/linear-swap-ex/market/detail/merged",
        )
        .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/linear-swap-ex/market/detail/merged");
        assert_eq!(evidence.checked_at, HTX_MARKET_DETAIL_MERGED_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_MARKET_DETAIL_MERGED_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_MARKET_DETAIL_MERGED_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_MARKET_DETAIL_MERGED_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_MARKET_DETAIL_MERGED_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            HTX_MARKET_DETAIL_MERGED_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#general-get-market-data-overview")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"perp_ticker".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn htx_mark_price_kline_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Get,
            "/index/market/history/linear_swap_mark_price_kline",
        )
        .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(
            evidence.path,
            "/index/market/history/linear_swap_mark_price_kline"
        );
        assert_eq!(evidence.checked_at, HTX_MARK_PRICE_KLINE_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_MARK_PRICE_KLINE_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_MARK_PRICE_KLINE_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_MARK_PRICE_KLINE_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_MARK_PRICE_KLINE_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            HTX_MARK_PRICE_KLINE_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#general-get-kline-data-of-mark-price")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"mark_index".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn htx_swap_index_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("htx", HttpMethod::Get, "/linear-swap-api/v1/swap_index")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/linear-swap-api/v1/swap_index");
        assert_eq!(evidence.checked_at, HTX_SWAP_INDEX_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_SWAP_INDEX_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_SWAP_INDEX_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_SWAP_INDEX_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_SWAP_INDEX_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, HTX_SWAP_INDEX_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#general-query-swap-index-price-information")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"mark_index".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn htx_open_interest_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence(
            "htx",
            HttpMethod::Get,
            "/linear-swap-api/v1/swap_open_interest",
        )
        .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/linear-swap-api/v1/swap_open_interest");
        assert_eq!(evidence.checked_at, HTX_OPEN_INTEREST_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_OPEN_INTEREST_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_OPEN_INTEREST_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_OPEN_INTEREST_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_OPEN_INTEREST_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            HTX_OPEN_INTEREST_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#general-get-swap-open-interest-information")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"open_interest".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn htx_spot_market_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("htx", HttpMethod::Get, "/market/tickers").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/market/tickers");
        assert_eq!(evidence.checked_at, HTX_SPOT_MARKET_TICKERS_CHECKED_AT);
        assert_eq!(evidence.doc_version, HTX_SPOT_MARKET_TICKERS_DOC_VERSION);
        assert_eq!(evidence.schema_hash, HTX_SPOT_MARKET_TICKERS_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, HTX_SPOT_MARKET_TICKERS_FIXTURE_ID);
        assert_eq!(evidence.parser_test, HTX_SPOT_MARKET_TICKERS_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            HTX_SPOT_MARKET_TICKERS_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 1);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("#get-market-tickers")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"spot_ticker".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn kucoin_server_time_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/timestamp").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v1/timestamp");
        assert_eq!(evidence.checked_at, KUCOIN_SERVER_TIME_CHECKED_AT);
        assert_eq!(evidence.doc_version, KUCOIN_SERVER_TIME_DOC_VERSION);
        assert_eq!(evidence.schema_hash, KUCOIN_SERVER_TIME_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, KUCOIN_SERVER_TIME_FIXTURE_ID);
        assert_eq!(evidence.parser_test, KUCOIN_SERVER_TIME_TEST);
        assert_eq!(evidence.request_builder_test, KUCOIN_SERVER_TIME_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 2);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/get-server-time")));
        assert!(evidence.use_cases.contains(&"calibration".to_owned()));
        assert!(evidence.data_kinds.contains(&"server_time".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn kucoin_depth20_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/level2/depth20")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v1/level2/depth20");
        assert_eq!(evidence.checked_at, KUCOIN_DEPTH20_CHECKED_AT);
        assert_eq!(evidence.doc_version, KUCOIN_DEPTH20_DOC_VERSION);
        assert_eq!(evidence.schema_hash, KUCOIN_DEPTH20_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, KUCOIN_DEPTH20_FIXTURE_ID);
        assert_eq!(evidence.parser_test, KUCOIN_DEPTH20_PARSER_TEST);
        assert_eq!(evidence.request_builder_test, KUCOIN_DEPTH20_REQUEST_TEST);
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 5);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/get-part-orderbook")));
        assert!(evidence.use_cases.contains(&"hot_path_fallback".to_owned()));
        assert!(evidence.data_kinds.contains(&"order_book".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn kucoin_contracts_active_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/contracts/active")
            .expect("evidence");

        assert_recorded_evidence(
            &evidence,
            ExpectedEvidence {
                method: "GET",
                path: "/api/v1/contracts/active",
                checked_at: KUCOIN_CONTRACTS_NATIVE_CHECKED_AT,
                doc_version: KUCOIN_CONTRACTS_NATIVE_DOC_VERSION,
                schema_hash: KUCOIN_CONTRACTS_NATIVE_SCHEMA_HASH,
                fixture_id: KUCOIN_CONTRACTS_NATIVE_FIXTURE_ID,
                parser_test: KUCOIN_CONTRACTS_NATIVE_PARSER_TEST,
                request_builder_test: KUCOIN_CONTRACTS_ACTIVE_REQUEST_TEST,
                auth_kind: PUBLIC_AUTH_KIND,
                weight: 3,
            },
        );
        assert_url_matches(&evidence, "/get-all-symbols");
        assert_list_contains(&evidence.use_cases, "metadata");
        assert_list_contains(&evidence.use_cases, "baseline");
        assert_list_contains(&evidence.data_kinds, "instrument_metadata");
        assert_list_contains(&evidence.data_kinds, "funding_rate");
        assert_list_contains(&evidence.rate_scopes, "ip");
    }

    #[test]
    fn kucoin_futures_all_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence =
            endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/allTickers").expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v1/allTickers");
        assert_eq!(evidence.checked_at, KUCOIN_FUTURES_ALL_TICKERS_CHECKED_AT);
        assert_eq!(evidence.doc_version, KUCOIN_FUTURES_ALL_TICKERS_DOC_VERSION);
        assert_eq!(evidence.schema_hash, KUCOIN_FUTURES_ALL_TICKERS_SCHEMA_HASH);
        assert_eq!(evidence.fixture_id, KUCOIN_FUTURES_ALL_TICKERS_FIXTURE_ID);
        assert_eq!(evidence.parser_test, KUCOIN_FUTURES_ALL_TICKERS_PARSER_TEST);
        assert_eq!(
            evidence.request_builder_test,
            KUCOIN_FUTURES_ALL_TICKERS_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 5);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/get-all-tickers")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"perp_ticker".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn kucoin_spot_market_all_tickers_evidence_uses_recorded_fixture_metadata() {
        let evidence = endpoint_evidence("kucoin", HttpMethod::Get, "/api/v1/market/allTickers")
            .expect("evidence");

        assert_eq!(evidence.method, "GET");
        assert_eq!(evidence.path, "/api/v1/market/allTickers");
        assert_eq!(
            evidence.checked_at,
            KUCOIN_SPOT_MARKET_ALL_TICKERS_CHECKED_AT
        );
        assert_eq!(
            evidence.doc_version,
            KUCOIN_SPOT_MARKET_ALL_TICKERS_DOC_VERSION
        );
        assert_eq!(
            evidence.schema_hash,
            KUCOIN_SPOT_MARKET_ALL_TICKERS_SCHEMA_HASH
        );
        assert_eq!(
            evidence.fixture_id,
            KUCOIN_SPOT_MARKET_ALL_TICKERS_FIXTURE_ID
        );
        assert_eq!(
            evidence.parser_test,
            KUCOIN_SPOT_MARKET_ALL_TICKERS_PARSER_TEST
        );
        assert_eq!(
            evidence.request_builder_test,
            KUCOIN_SPOT_MARKET_ALL_TICKERS_REQUEST_TEST
        );
        assert_eq!(evidence.auth_kind, PUBLIC_AUTH_KIND);
        assert_eq!(evidence.weight, 15);
        assert!(evidence
            .doc_urls
            .iter()
            .any(|url| url.ends_with("/get-all-tickers")));
        assert!(evidence.use_cases.contains(&"baseline".to_owned()));
        assert!(evidence.data_kinds.contains(&"spot_ticker".to_owned()));
        assert!(evidence.rate_scopes.contains(&"ip".to_owned()));
    }

    #[test]
    fn metadata_specs_cover_current_adapter_metadata_endpoints() {
        for (venue, method, path, data_kind) in metadata_endpoints() {
            assert!(
                ENDPOINT_SPECS.iter().any(|spec| {
                    spec.venue == venue
                        && spec.method == method
                        && spec.path == path
                        && spec.use_case == EndpointUseCase::Metadata
                        && spec.data_kind == data_kind
                }),
                "missing metadata endpoint for {venue:?} {path}"
            );
        }
    }

    #[test]
    fn baseline_specs_cover_current_public_snapshot_endpoints() {
        for (venue, method, path, data_kind) in baseline_endpoints() {
            assert!(
                ENDPOINT_SPECS.iter().any(|spec| {
                    spec.venue == venue
                        && spec.method == method
                        && spec.path == path
                        && spec.use_case == EndpointUseCase::Baseline
                        && spec.data_kind == data_kind
                }),
                "missing baseline endpoint for {venue:?} {path} {data_kind:?}"
            );
        }
    }

    #[test]
    fn every_venue_has_docs_url() {
        for spec in VENUE_SPECS {
            assert!(spec.docs_url.starts_with("https://"));
            assert!(!spec.rest_base.is_empty());
        }
    }

    #[test]
    fn production_venue_roots_track_current_official_transports() {
        let binance = VENUE_SPECS
            .iter()
            .find(|spec| spec.venue == VenueId::Binance)
            .expect("binance venue spec");
        assert_eq!(
            binance.ws_market,
            Some("wss://fstream.binance.com/public/ws")
        );

        let okx = VENUE_SPECS
            .iter()
            .find(|spec| spec.venue == VenueId::Okx)
            .expect("okx venue spec");
        assert_eq!(okx.rest_base, "https://openapi.okx.com");

        let bitget = VENUE_SPECS
            .iter()
            .find(|spec| spec.venue == VenueId::Bitget)
            .expect("bitget venue spec");
        assert_eq!(bitget.ws_market, Some("wss://ws.bitget.com/v3/ws/public"));
        assert_eq!(bitget.ws_trade, Some("wss://ws.bitget.com/v3/ws/private"));
    }

    #[test]
    fn venue_defaults_are_non_zero() {
        for spec in VENUE_SPECS {
            let defaults = spec.venue.defaults();
            assert!(defaults.qps > 0);
            assert!(defaults.timeout_secs > 0);
            assert!(defaults.fanout_timeout_secs > 0);
        }
    }

    #[test]
    fn venue_name_mapping_covers_builder_dex() {
        assert_eq!(
            VenueId::from_exchange_name("hyperliquid:xyz"),
            Some(VenueId::Hyperliquid)
        );
        assert_eq!(VenueId::from_exchange_name("okx-live"), Some(VenueId::Okx));
        assert_eq!(VenueId::from_exchange_name("unknown"), None);
    }

    fn calibration_venues() -> [VenueId; 7] {
        [
            VenueId::Binance,
            VenueId::Okx,
            VenueId::Bybit,
            VenueId::Bitget,
            VenueId::Gate,
            VenueId::Htx,
            VenueId::Kucoin,
        ]
    }

    fn metadata_endpoints() -> [(VenueId, HttpMethod, &'static str, EndpointDataKind); 8] {
        [
            (
                VenueId::Binance,
                HttpMethod::Get,
                "/fapi/v1/exchangeInfo",
                EndpointDataKind::InstrumentMetadata,
            ),
            (
                VenueId::Okx,
                HttpMethod::Get,
                "/api/v5/public/instruments",
                EndpointDataKind::InstrumentMetadata,
            ),
            (
                VenueId::Bybit,
                HttpMethod::Get,
                "/v5/market/instruments-info",
                EndpointDataKind::InstrumentMetadata,
            ),
            (
                VenueId::Bitget,
                HttpMethod::Get,
                "/api/v3/market/instruments",
                EndpointDataKind::InstrumentMetadata,
            ),
            (
                VenueId::Gate,
                HttpMethod::Get,
                "/api/v4/futures/usdt/contracts",
                EndpointDataKind::InstrumentMetadata,
            ),
            (
                VenueId::Htx,
                HttpMethod::Get,
                "/linear-swap-api/v1/swap_contract_info",
                EndpointDataKind::InstrumentMetadata,
            ),
            (
                VenueId::Kucoin,
                HttpMethod::Get,
                "/api/v1/contracts/active",
                EndpointDataKind::InstrumentMetadata,
            ),
            (
                VenueId::Hyperliquid,
                HttpMethod::Post,
                "/info",
                EndpointDataKind::InstrumentMetadata,
            ),
        ]
    }

    fn baseline_endpoints() -> [(VenueId, HttpMethod, &'static str, EndpointDataKind); 32] {
        [
            (
                VenueId::Binance,
                HttpMethod::Get,
                "/fapi/v1/premiumIndex",
                EndpointDataKind::FundingRate,
            ),
            (
                VenueId::Binance,
                HttpMethod::Get,
                "/fapi/v1/premiumIndex",
                EndpointDataKind::MarkIndex,
            ),
            (
                VenueId::Binance,
                HttpMethod::Get,
                "/fapi/v1/openInterest",
                EndpointDataKind::OpenInterest,
            ),
            (
                VenueId::Binance,
                HttpMethod::Get,
                "/fapi/v1/ticker/24hr",
                EndpointDataKind::PerpTicker,
            ),
            (
                VenueId::Binance,
                HttpMethod::Get,
                "/api/v3/ticker/24hr",
                EndpointDataKind::SpotTicker,
            ),
            (
                VenueId::Okx,
                HttpMethod::Get,
                "/api/v5/market/tickers",
                EndpointDataKind::PerpTicker,
            ),
            (
                VenueId::Okx,
                HttpMethod::Get,
                "/api/v5/market/tickers",
                EndpointDataKind::SpotTicker,
            ),
            (
                VenueId::Okx,
                HttpMethod::Get,
                "/api/v5/public/funding-rate",
                EndpointDataKind::FundingRate,
            ),
            (
                VenueId::Okx,
                HttpMethod::Get,
                "/api/v5/public/mark-price",
                EndpointDataKind::MarkIndex,
            ),
            (
                VenueId::Okx,
                HttpMethod::Get,
                "/api/v5/market/index-tickers",
                EndpointDataKind::MarkIndex,
            ),
            (
                VenueId::Okx,
                HttpMethod::Get,
                "/api/v5/public/open-interest",
                EndpointDataKind::OpenInterest,
            ),
            (
                VenueId::Bybit,
                HttpMethod::Get,
                "/v5/market/tickers",
                EndpointDataKind::PerpTicker,
            ),
            (
                VenueId::Bybit,
                HttpMethod::Get,
                "/v5/market/tickers",
                EndpointDataKind::FundingRate,
            ),
            (
                VenueId::Bybit,
                HttpMethod::Get,
                "/v5/market/tickers",
                EndpointDataKind::SpotTicker,
            ),
            (
                VenueId::Bitget,
                HttpMethod::Get,
                "/api/v3/market/current-fund-rate",
                EndpointDataKind::FundingRate,
            ),
            (
                VenueId::Bitget,
                HttpMethod::Get,
                "/api/v3/market/tickers",
                EndpointDataKind::PerpTicker,
            ),
            (
                VenueId::Bitget,
                HttpMethod::Get,
                "/api/v3/market/tickers",
                EndpointDataKind::SpotTicker,
            ),
            (
                VenueId::Gate,
                HttpMethod::Get,
                "/api/v4/futures/usdt/tickers",
                EndpointDataKind::PerpTicker,
            ),
            (
                VenueId::Gate,
                HttpMethod::Get,
                "/api/v4/spot/tickers",
                EndpointDataKind::SpotTicker,
            ),
            (
                VenueId::Gate,
                HttpMethod::Get,
                "/api/v4/futures/usdt/contracts",
                EndpointDataKind::FundingRate,
            ),
            (
                VenueId::Htx,
                HttpMethod::Get,
                "/linear-swap-api/v1/swap_batch_funding_rate",
                EndpointDataKind::FundingRate,
            ),
            (
                VenueId::Htx,
                HttpMethod::Get,
                "/linear-swap-ex/market/detail/merged",
                EndpointDataKind::PerpTicker,
            ),
            (
                VenueId::Htx,
                HttpMethod::Get,
                "/index/market/history/linear_swap_mark_price_kline",
                EndpointDataKind::MarkIndex,
            ),
            (
                VenueId::Htx,
                HttpMethod::Get,
                "/linear-swap-api/v1/swap_index",
                EndpointDataKind::MarkIndex,
            ),
            (
                VenueId::Htx,
                HttpMethod::Get,
                "/linear-swap-api/v1/swap_open_interest",
                EndpointDataKind::OpenInterest,
            ),
            (
                VenueId::Htx,
                HttpMethod::Get,
                "/market/tickers",
                EndpointDataKind::SpotTicker,
            ),
            (
                VenueId::Kucoin,
                HttpMethod::Get,
                "/api/v1/allTickers",
                EndpointDataKind::PerpTicker,
            ),
            (
                VenueId::Kucoin,
                HttpMethod::Get,
                "/api/v1/contracts/active",
                EndpointDataKind::FundingRate,
            ),
            (
                VenueId::Kucoin,
                HttpMethod::Get,
                "/api/v1/market/allTickers",
                EndpointDataKind::SpotTicker,
            ),
            (
                VenueId::Hyperliquid,
                HttpMethod::Post,
                "/info",
                EndpointDataKind::PerpTicker,
            ),
            (
                VenueId::Hyperliquid,
                HttpMethod::Post,
                "/info",
                EndpointDataKind::SpotTicker,
            ),
            (
                VenueId::Hyperliquid,
                HttpMethod::Post,
                "/info",
                EndpointDataKind::FundingRate,
            ),
        ]
    }
}
