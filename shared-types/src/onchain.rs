use serde::{Deserialize, Serialize};

#[path = "onchain/credentials.rs"]
mod credentials;
#[path = "onchain/cross_chain.rs"]
mod cross_chain;
#[path = "onchain/dex_comparison.rs"]
mod dex_comparison;
#[path = "onchain/execution.rs"]
mod execution;
#[path = "onchain/path.rs"]
mod path;
#[path = "onchain/replenishment.rs"]
mod replenishment;
#[path = "onchain/spread_alert.rs"]
mod spread_alert;
#[path = "onchain/token_identity.rs"]
mod token_identity;

pub use credentials::*;
pub use cross_chain::*;
pub use dex_comparison::*;
pub use execution::*;
pub use path::*;
pub use replenishment::*;
pub use spread_alert::*;
pub use token_identity::*;

pub const EVM_NATIVE_TOKEN_ADDRESS: &str = "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnchainChainPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub chain_id: Option<u64>,
    pub provider: &'static str,
    pub provider_label: &'static str,
    pub provider_docs_url: &'static str,
    pub route: &'static str,
    pub base_token: &'static str,
    pub quote_token: &'static str,
    pub base_address: &'static str,
    pub quote_address: &'static str,
    pub base_decimals: u8,
    pub quote_decimals: u8,
    pub base_amount_raw: &'static str,
    pub quote_amount_raw: &'static str,
}

pub const ONCHAIN_CHAIN_PRESETS: [OnchainChainPreset; 8] = [
    OnchainChainPreset {
        id: "solana",
        label: "Solana",
        chain_id: None,
        provider: "jupiter_swap_v2",
        provider_label: "Jupiter Keyless",
        provider_docs_url: "https://developers.jup.ag/docs/swap/order-and-execute",
        route: "jupiter-keyless",
        base_token: "SOL",
        quote_token: "USDC",
        base_address: "So11111111111111111111111111111111111111112",
        quote_address: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        base_decimals: 9,
        quote_decimals: 6,
        base_amount_raw: "1000000000",
        quote_amount_raw: "100000000",
    },
    evm_preset(
        "ethereum",
        "Ethereum",
        1,
        "ETH",
        "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        6,
    ),
    evm_preset(
        "arbitrum",
        "Arbitrum One",
        42_161,
        "ETH",
        "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
        6,
    ),
    evm_preset(
        "base",
        "Base",
        8_453,
        "ETH",
        "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        6,
    ),
    evm_preset(
        "optimism",
        "OP Mainnet",
        10,
        "ETH",
        "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85",
        6,
    ),
    evm_preset(
        "polygon",
        "Polygon PoS",
        137,
        "POL",
        "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359",
        6,
    ),
    evm_preset(
        "bnb-smart-chain",
        "BNB Smart Chain",
        56,
        "BNB",
        "0x8AC76a51cc950d9822D68b83fE1Ad97B32Cd580d",
        18,
    ),
    evm_preset(
        "avalanche",
        "Avalanche C-Chain",
        43_114,
        "AVAX",
        "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E",
        6,
    ),
];

const fn evm_preset(
    id: &'static str,
    label: &'static str,
    chain_id: u64,
    base_token: &'static str,
    quote_address: &'static str,
    quote_decimals: u8,
) -> OnchainChainPreset {
    OnchainChainPreset {
        id,
        label,
        chain_id: Some(chain_id),
        provider: "zeroex_swap_v2",
        provider_label: "0x Swap API V2",
        provider_docs_url:
            "https://docs.0x.org/api-reference/evm-ap-is/swap/allowanceholder-getprice",
        route: "0x-allowance-holder",
        base_token,
        quote_token: "USDC",
        base_address: EVM_NATIVE_TOKEN_ADDRESS,
        quote_address,
        base_decimals: 18,
        quote_decimals,
        base_amount_raw: "1000000000000000000",
        quote_amount_raw: if quote_decimals == 18 {
            "100000000000000000000"
        } else {
            "100000000"
        },
    }
}

pub fn onchain_chain_preset(chain: &str) -> Option<&'static OnchainChainPreset> {
    ONCHAIN_CHAIN_PRESETS
        .iter()
        .find(|preset| preset.id.eq_ignore_ascii_case(chain.trim()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnchainQuoteProviderOption {
    pub id: &'static str,
    pub label: &'static str,
    pub official_docs_url: &'static str,
    pub default_route: &'static str,
    pub supports_solana: bool,
    pub supports_evm: bool,
}

pub const ONCHAIN_QUOTE_PROVIDERS: [OnchainQuoteProviderOption; 5] = [
    OnchainQuoteProviderOption {
        id: "jupiter_swap_v2",
        label: "Jupiter Keyless",
        official_docs_url: "https://developers.jup.ag/docs/swap/order-and-execute",
        default_route: "jupiter-keyless",
        supports_solana: true,
        supports_evm: false,
    },
    OnchainQuoteProviderOption {
        id: "jupiter_swap_v2_keyed",
        label: "Jupiter API Key",
        official_docs_url: "https://developers.jup.ag/docs/portal/rate-limits",
        default_route: "jupiter-api-key",
        supports_solana: true,
        supports_evm: false,
    },
    OnchainQuoteProviderOption {
        id: "zeroex_swap_v2",
        label: "0x Swap API V2",
        official_docs_url:
            "https://docs.0x.org/api-reference/evm-ap-is/swap/allowanceholder-getprice",
        default_route: "0x-allowance-holder",
        supports_solana: false,
        supports_evm: true,
    },
    OnchainQuoteProviderOption {
        id: "okx_dex_v6",
        label: "OKX DEX Aggregator V6",
        official_docs_url: "https://web3.okx.com/zh-hans/onchainos/dev-docs/trade/dex-get-quote",
        default_route: "okx-dex-aggregator",
        supports_solana: false,
        supports_evm: true,
    },
    OnchainQuoteProviderOption {
        id: "cow_protocol",
        label: "CoW Protocol Fast Quote",
        official_docs_url: "https://api.cow.fi/docs/#/default/post_api_v1_quote",
        default_route: "cow-solver-fast",
        supports_solana: false,
        supports_evm: true,
    },
];

pub fn onchain_quote_provider(provider: &str) -> Option<&'static OnchainQuoteProviderOption> {
    ONCHAIN_QUOTE_PROVIDERS
        .iter()
        .find(|option| option.id.eq_ignore_ascii_case(provider.trim()))
}

pub fn onchain_quote_provider_family(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "jupiter_swap_v2" | "jupiter_swap_v2_keyed" => Some("jupiter"),
        "zeroex_swap_v2" => Some("zeroex"),
        "okx_dex_v6" => Some("okx"),
        "cow_protocol" => Some("cow"),
        _ => None,
    }
}

pub fn onchain_quote_providers_independent(left: &str, right: &str) -> bool {
    match (
        onchain_quote_provider_family(left),
        onchain_quote_provider_family(right),
    ) {
        (Some(left), Some(right)) => left != right,
        _ => false,
    }
}

pub fn onchain_quote_provider_supported(provider: &str, chain: &str) -> bool {
    let Some(provider) = onchain_quote_provider(provider) else {
        return false;
    };
    if provider.id == "cow_protocol" {
        return [
            "ethereum",
            "arbitrum",
            "base",
            "polygon",
            "bnb-smart-chain",
            "avalanche",
        ]
        .iter()
        .any(|supported| supported.eq_ignore_ascii_case(chain.trim()));
    }
    onchain_chain_preset(chain).is_some_and(|preset| {
        if preset.chain_id.is_some() {
            provider.supports_evm
        } else {
            provider.supports_solana
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnchainKnownToken {
    pub symbol: &'static str,
    pub address: &'static str,
    pub decimals: u8,
}

pub fn onchain_known_token(chain: &str, symbol: &str) -> Option<OnchainKnownToken> {
    let preset = onchain_chain_preset(chain)?;
    if preset.base_token.eq_ignore_ascii_case(symbol.trim()) {
        return Some(OnchainKnownToken {
            symbol: preset.base_token,
            address: preset.base_address,
            decimals: preset.base_decimals,
        });
    }
    preset
        .quote_token
        .eq_ignore_ascii_case(symbol.trim())
        .then_some(OnchainKnownToken {
            symbol: preset.quote_token,
            address: preset.quote_address,
            decimals: preset.quote_decimals,
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnchainCexVenueOption {
    pub id: &'static str,
    pub label: &'static str,
}

pub const ONCHAIN_CEX_VENUES: [OnchainCexVenueOption; 7] = [
    OnchainCexVenueOption {
        id: "binance",
        label: "Binance",
    },
    OnchainCexVenueOption {
        id: "okx",
        label: "OKX",
    },
    OnchainCexVenueOption {
        id: "bybit",
        label: "Bybit",
    },
    OnchainCexVenueOption {
        id: "bitget",
        label: "Bitget",
    },
    OnchainCexVenueOption {
        id: "gate",
        label: "Gate",
    },
    OnchainCexVenueOption {
        id: "kucoin",
        label: "KuCoin",
    },
    OnchainCexVenueOption {
        id: "kraken",
        label: "Kraken",
    },
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainRpcMode {
    #[default]
    ProviderManaged,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainRpcConfig {
    pub mode: OnchainRpcMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainSourceConfigPatch {
    pub provider: Option<String>,
    pub rpc_mode: Option<OnchainRpcMode>,
    pub custom_rpc_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainRpcStatus {
    pub mode: OnchainRpcMode,
    pub configured: bool,
    pub endpoint_label: Option<String>,
    pub expected_chain_id: Option<u64>,
    pub observed_chain_id: Option<u64>,
    pub block_number: Option<u64>,
    pub latency_ms: Option<i64>,
    pub observed_at_ms: Option<i64>,
    pub ready: bool,
    pub problem: Option<String>,
    pub official_docs_url: String,
}

impl Default for OnchainRpcStatus {
    fn default() -> Self {
        Self {
            mode: OnchainRpcMode::ProviderManaged,
            configured: false,
            endpoint_label: None,
            expected_chain_id: None,
            observed_chain_id: None,
            block_number: None,
            latency_ms: None,
            observed_at_ms: None,
            ready: true,
            problem: None,
            official_docs_url: "https://ethereum.org/developers/docs/apis/json-rpc/".to_owned(),
        }
    }
}

const fn default_identity_resolved() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainComparisonConfig {
    pub enabled: bool,
    pub chain: String,
    pub provider: String,
    pub pool_or_route: String,
    pub base_token: String,
    pub quote_token: String,
    #[serde(default = "default_identity_resolved")]
    pub base_identity_resolved: bool,
    #[serde(default = "default_identity_resolved")]
    pub quote_identity_resolved: bool,
    pub base_mint: String,
    pub quote_mint: String,
    pub base_decimals: u8,
    pub quote_decimals: u8,
    pub base_amount_raw: String,
    pub quote_amount_raw: String,
    #[serde(default)]
    pub wallet_address: String,
    pub cex_venue: String,
    pub cex_symbol: String,
    pub cex_taker_fee_bps: f64,
    pub gas_usd: f64,
    pub slippage_bps: f64,
    pub min_liquidity_usd: f64,
    pub max_age_ms: i64,
    pub rpc: OnchainRpcConfig,
    pub spread_alert: OnchainSpreadAlertConfig,
    #[serde(default)]
    pub dex_comparison: OnchainDexComparisonConfig,
    #[serde(default)]
    pub cross_chain: OnchainCrossChainConfig,
}

impl Default for OnchainComparisonConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chain: "solana".to_owned(),
            provider: "jupiter_swap_v2".to_owned(),
            pool_or_route: "jupiter-keyless".to_owned(),
            base_token: "SOL".to_owned(),
            quote_token: "USDC".to_owned(),
            base_identity_resolved: true,
            quote_identity_resolved: true,
            base_mint: "So11111111111111111111111111111111111111112".to_owned(),
            quote_mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_owned(),
            base_decimals: 9,
            quote_decimals: 6,
            base_amount_raw: "1000000000".to_owned(),
            quote_amount_raw: "100000000".to_owned(),
            wallet_address: String::new(),
            cex_venue: "binance".to_owned(),
            cex_symbol: "SOL/USDC".to_owned(),
            cex_taker_fee_bps: 10.0,
            gas_usd: 0.01,
            slippage_bps: 10.0,
            min_liquidity_usd: 100.0,
            max_age_ms: 10_000,
            rpc: OnchainRpcConfig::default(),
            spread_alert: OnchainSpreadAlertConfig::default(),
            dex_comparison: OnchainDexComparisonConfig::default(),
            cross_chain: OnchainCrossChainConfig::default(),
        }
    }
}

fn onchain_cex_pair_tokens(symbol: &str) -> Option<(&str, &str)> {
    let (base, quote) = symbol.trim().split_once('/')?;
    let base = base.trim();
    let quote = quote.trim();
    (!base.is_empty() && !quote.is_empty() && !quote.contains('/')).then_some((base, quote))
}

pub fn onchain_cex_base_token(symbol: &str) -> Option<&str> {
    onchain_cex_pair_tokens(symbol).map(|(base, _)| base)
}

pub fn onchain_cex_quote_token(symbol: &str) -> Option<&str> {
    onchain_cex_pair_tokens(symbol).map(|(_, quote)| quote)
}

pub fn onchain_quotes_match(config: &OnchainComparisonConfig) -> bool {
    if !config.quote_identity_resolved {
        return false;
    }
    onchain_cex_quote_token(&config.cex_symbol)
        .is_some_and(|quote| quote.eq_ignore_ascii_case(config.quote_token.trim()))
}

pub fn onchain_cex_pair_matches(config: &OnchainComparisonConfig) -> bool {
    if !config.base_identity_resolved {
        return false;
    }
    onchain_cex_base_token(&config.cex_symbol)
        .is_some_and(|base| base.eq_ignore_ascii_case(config.base_token.trim()))
        && onchain_quotes_match(config)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainComparisonConfigPatch {
    pub enabled: Option<bool>,
    pub chain: Option<String>,
    pub pool_or_route: Option<String>,
    pub base_token: Option<String>,
    pub quote_token: Option<String>,
    pub base_identity_resolved: Option<bool>,
    pub quote_identity_resolved: Option<bool>,
    pub base_mint: Option<String>,
    pub quote_mint: Option<String>,
    pub base_decimals: Option<u8>,
    pub quote_decimals: Option<u8>,
    pub base_amount_raw: Option<String>,
    pub quote_amount_raw: Option<String>,
    pub wallet_address: Option<String>,
    pub cex_venue: Option<String>,
    pub cex_symbol: Option<String>,
    pub cex_taker_fee_bps: Option<f64>,
    pub gas_usd: Option<f64>,
    pub slippage_bps: Option<f64>,
    pub min_liquidity_usd: Option<f64>,
    pub max_age_ms: Option<i64>,
    pub source: Option<OnchainSourceConfigPatch>,
    pub spread_alert: Option<OnchainSpreadAlertConfigPatch>,
    pub dex_comparison: Option<OnchainDexComparisonConfigPatch>,
    pub cross_chain: Option<OnchainCrossChainConfigPatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainComparisonQuality {
    Disabled,
    Pending,
    ValuationPending,
    Fresh,
    RawCrossQuote,
    RawCustomPair,
    Stale,
    LowLiquidity,
    MappingInvalid,
    UpstreamUnavailable,
    NoNetProfit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainComparisonDirection {
    BuyOnchainSellCex,
    BuyCexSellOnchain,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainQuoteEvidence {
    pub provider: String,
    pub endpoint: String,
    pub official_docs_url: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount_raw: String,
    pub output_amount_raw: String,
    pub router: Option<String>,
    pub transaction_requested: bool,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainQuoteConversionEvidence {
    pub venue: String,
    pub symbol: String,
    pub source: String,
    pub cex_quote: String,
    pub onchain_quote: String,
    pub source_bid: f64,
    pub source_ask: f64,
    /// On-chain quote received when selling one unit of the CEX quote.
    pub cex_to_onchain_bid: f64,
    /// On-chain quote required to acquire one unit of the CEX quote.
    pub cex_to_onchain_ask: f64,
    /// On-chain quote notional observable at the conversion best price for CEX -> on-chain.
    pub cex_to_onchain_capacity: f64,
    /// On-chain quote notional observable at the conversion best price for on-chain -> CEX.
    pub onchain_to_cex_capacity: f64,
    pub freshness_ms: i64,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainCexComparison {
    pub direction: OnchainComparisonDirection,
    pub onchain_price: f64,
    pub cex_price: f64,
    pub gross_spread_bps: f64,
    pub cex_fee_bps: f64,
    #[serde(default)]
    pub quote_conversion_fee_bps: f64,
    pub slippage_bps: f64,
    pub gas_usd: f64,
    pub gas_bps: f64,
    pub total_cost_bps: f64,
    pub net_spread_bps: f64,
    pub observable_notional_usd: f64,
    pub executable: bool,
}

pub const ONCHAIN_BATCH_MAX_ITEMS: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainBatchItemSnapshot {
    pub item_id: String,
    pub config: OnchainComparisonConfig,
    pub quality: OnchainComparisonQuality,
    pub best_direction: Option<OnchainComparisonDirection>,
    #[serde(default)]
    pub best_gross_spread_bps: Option<f64>,
    pub best_net_spread_bps: Option<f64>,
    pub observable_notional_usd: Option<f64>,
    pub quote_observed_at_ms: Option<i64>,
    pub onchain_freshness_ms: Option<i64>,
    pub onchain_latency_ms: Option<i64>,
    pub quote_interval_ms: i64,
    pub cex_source: String,
    pub cex_freshness_ms: Option<i64>,
    pub cex_observed_at_ms: Option<i64>,
    #[serde(default)]
    pub quote_conversion: Option<OnchainQuoteConversionEvidence>,
    #[serde(default)]
    pub quote_usd_valuation: Option<OnchainUsdValuation>,
    #[serde(default)]
    pub dex_quality: OnchainDexComparisonQuality,
    #[serde(default)]
    pub best_dex_direction: Option<OnchainDexComparisonDirection>,
    #[serde(default)]
    pub best_dex_net_return_bps: Option<f64>,
    #[serde(default)]
    pub dex_problem: Option<String>,
    #[serde(default)]
    pub cross_chain_quality: OnchainCrossChainQuality,
    #[serde(default)]
    pub best_cross_chain_net_return_bps: Option<f64>,
    #[serde(default)]
    pub cross_chain_problem: Option<String>,
    #[serde(default)]
    pub cex_problem: Option<String>,
    #[serde(default)]
    pub cex_retry_after_ms: Option<i64>,
    #[serde(alias = "providerReady")]
    pub provider_configured: bool,
    pub provider_problem: Option<String>,
    #[serde(default)]
    pub provider_retry_after_ms: Option<i64>,
    pub degradation_reasons: Vec<String>,
    pub observed_at_ms: i64,
}

impl OnchainBatchItemSnapshot {
    pub fn pending(
        item_id: String,
        config: OnchainComparisonConfig,
        quote_interval_ms: i64,
        now_ms: i64,
    ) -> Self {
        let dex_quality = if config.dex_comparison.enabled {
            OnchainDexComparisonQuality::Pending
        } else {
            OnchainDexComparisonQuality::Disabled
        };
        let cross_chain_quality = if config.cross_chain.enabled {
            OnchainCrossChainQuality::Pending
        } else {
            OnchainCrossChainQuality::Disabled
        };
        Self {
            item_id,
            config,
            quality: OnchainComparisonQuality::Pending,
            best_direction: None,
            best_gross_spread_bps: None,
            best_net_spread_bps: None,
            observable_notional_usd: None,
            quote_observed_at_ms: None,
            onchain_freshness_ms: None,
            onchain_latency_ms: None,
            quote_interval_ms,
            cex_source: "not_started".to_owned(),
            cex_freshness_ms: None,
            cex_observed_at_ms: None,
            quote_conversion: None,
            quote_usd_valuation: None,
            dex_quality,
            best_dex_direction: None,
            best_dex_net_return_bps: None,
            dex_problem: None,
            cross_chain_quality,
            best_cross_chain_net_return_bps: None,
            cross_chain_problem: None,
            cex_problem: None,
            cex_retry_after_ms: None,
            provider_configured: true,
            provider_problem: None,
            provider_retry_after_ms: None,
            degradation_reasons: vec!["waiting for the first scheduled quote".to_owned()],
            observed_at_ms: now_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainBatchSnapshot {
    pub items: Vec<OnchainBatchItemSnapshot>,
    pub max_items: usize,
    pub estimated_sweep_ms: i64,
    pub projection_interval_ms: i64,
    pub observed_at_ms: i64,
}

impl Default for OnchainBatchSnapshot {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            max_items: ONCHAIN_BATCH_MAX_ITEMS,
            estimated_sweep_ms: 0,
            projection_interval_ms: 250,
            observed_at_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainBatchRemoveRequest {
    pub item_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainComparisonSnapshot {
    pub config: OnchainComparisonConfig,
    pub quality: OnchainComparisonQuality,
    pub comparisons: Vec<OnchainCexComparison>,
    pub quote_evidence: Vec<OnchainQuoteEvidence>,
    pub quote_observed_at_ms: Option<i64>,
    pub onchain_freshness_ms: Option<i64>,
    pub onchain_latency_ms: Option<i64>,
    pub quote_interval_ms: i64,
    pub cex_source: String,
    pub cex_freshness_ms: Option<i64>,
    pub cex_observed_at_ms: Option<i64>,
    #[serde(default)]
    pub quote_conversion: Option<OnchainQuoteConversionEvidence>,
    #[serde(default)]
    pub quote_usd_valuation: Option<OnchainUsdValuation>,
    #[serde(default)]
    pub cex_problem: Option<String>,
    #[serde(default)]
    pub cex_retry_after_ms: Option<i64>,
    pub projection_interval_ms: i64,
    #[serde(alias = "providerReady")]
    pub provider_configured: bool,
    pub provider_problem: Option<String>,
    #[serde(default)]
    pub provider_retry_after_ms: Option<i64>,
    pub rpc_status: OnchainRpcStatus,
    pub cex_symbol_source: String,
    pub degradation_reasons: Vec<String>,
    pub read_only: bool,
    #[serde(default)]
    pub execution_readiness: OnchainExecutionReadiness,
    #[serde(default)]
    pub dex_comparison: OnchainDexComparisonSnapshot,
    #[serde(default)]
    pub cross_chain: OnchainCrossChainSnapshot,
    pub observed_at_ms: i64,
    pub batch: OnchainBatchSnapshot,
}

impl Default for OnchainComparisonSnapshot {
    fn default() -> Self {
        Self {
            config: OnchainComparisonConfig::default(),
            quality: OnchainComparisonQuality::Disabled,
            comparisons: Vec::new(),
            quote_evidence: Vec::new(),
            quote_observed_at_ms: None,
            onchain_freshness_ms: None,
            onchain_latency_ms: None,
            quote_interval_ms: 4_500,
            cex_source: "not_started".to_owned(),
            cex_freshness_ms: None,
            cex_observed_at_ms: None,
            quote_conversion: None,
            quote_usd_valuation: None,
            cex_problem: None,
            cex_retry_after_ms: None,
            projection_interval_ms: 100,
            provider_configured: true,
            provider_problem: None,
            provider_retry_after_ms: None,
            rpc_status: OnchainRpcStatus::default(),
            cex_symbol_source: "configured".to_owned(),
            degradation_reasons: vec!["on-chain comparison is disabled".to_owned()],
            read_only: true,
            execution_readiness: OnchainExecutionReadiness::default(),
            dex_comparison: OnchainDexComparisonSnapshot::default(),
            cross_chain: OnchainCrossChainSnapshot::default(),
            observed_at_ms: 0,
            batch: OnchainBatchSnapshot::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnchainUsdValuation {
    pub asset: String,
    pub venue: String,
    pub symbol: String,
    pub source: String,
    pub usd_bid: f64,
    pub usd_ask: f64,
    pub observed_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_catalog_keeps_solana_and_evm_capabilities_explicit() {
        assert!(onchain_quote_provider_supported(
            "jupiter_swap_v2",
            "solana"
        ));
        assert!(onchain_quote_provider_supported(
            "jupiter_swap_v2_keyed",
            "solana"
        ));
        assert!(!onchain_quote_provider_supported("okx_dex_v6", "solana"));
        assert!(onchain_quote_provider_supported("okx_dex_v6", "base"));
        assert!(onchain_quote_provider_supported("zeroex_swap_v2", "base"));
        assert!(onchain_quote_provider_supported("cow_protocol", "base"));
        assert!(!onchain_quote_provider_supported(
            "cow_protocol",
            "optimism"
        ));
        assert!(!onchain_quote_provider_supported("cow_protocol", "solana"));
    }

    #[test]
    fn onchain_cex_catalog_includes_kraken_spot() {
        assert!(ONCHAIN_CEX_VENUES.iter().any(|venue| venue.id == "kraken"));
    }

    #[test]
    fn explicit_cex_pair_controls_comparison_identity() {
        let mut config = OnchainComparisonConfig::default();
        assert!(onchain_quotes_match(&config));
        assert!(onchain_cex_pair_matches(&config));

        config.cex_symbol = "SOL/USD".to_owned();
        assert_eq!(onchain_cex_quote_token(&config.cex_symbol), Some("USD"));
        assert!(!onchain_quotes_match(&config));
        assert!(!onchain_cex_pair_matches(&config));

        config.cex_symbol = "ETH/USDC".to_owned();
        assert_eq!(onchain_cex_base_token(&config.cex_symbol), Some("ETH"));
        assert!(onchain_quotes_match(&config));
        assert!(!onchain_cex_pair_matches(&config));
    }

    #[test]
    fn unresolved_identity_flags_never_become_executable_matches() {
        let mut config = OnchainComparisonConfig::default();
        config.base_identity_resolved = false;
        config.cex_symbol = "SOL/USDC".to_owned();
        assert!(!onchain_cex_pair_matches(&config));

        config.base_identity_resolved = true;
        config.quote_identity_resolved = false;
        assert!(!onchain_cex_pair_matches(&config));
        assert!(!onchain_quotes_match(&config));
    }

    #[test]
    fn provider_configuration_serializes_without_claiming_runtime_readiness() {
        let snapshot = OnchainComparisonSnapshot::default();
        let mut encoded = serde_json::to_value(snapshot).expect("snapshot serializes");
        let object = encoded.as_object_mut().expect("snapshot is an object");

        assert_eq!(
            object.get("providerConfigured"),
            Some(&serde_json::Value::Bool(true))
        );
        assert!(!object.contains_key("providerReady"));

        let configured = object
            .remove("providerConfigured")
            .expect("providerConfigured exists");
        object.insert("providerReady".to_owned(), configured);
        let legacy: OnchainComparisonSnapshot =
            serde_json::from_value(encoded).expect("legacy providerReady alias deserializes");
        assert!(legacy.provider_configured);
    }
}
