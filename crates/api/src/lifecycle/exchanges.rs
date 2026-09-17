use super::env;
use crate::services::trading_credentials;
use crate::state::AppState;
use exchange::{
    Binance, BinanceConfig, BinanceCredentials, Bitget, BitgetConfig, BitgetCredentials, Bybit,
    BybitConfig, BybitCredentials, ExchangeAdapter, Gate, GateConfig, GateCredentials, GateCrossEx,
    GateCrossExConfig, GateCrossExCredentials, Hyperliquid, HyperliquidConfig,
    HyperliquidCredentials, Kraken, KrakenConfig, KrakenCredentials, KrakenFuturesCredentials,
    KrakenSpotCredentials, Kucoin, KucoinConfig, KucoinCredentials, Okx, OkxConfig, OkxCredentials,
};
use std::sync::Arc;
use tracing::{info, warn};

/// 9 个交易场所家族适配器注册。没有 API key 时仍注册公共行情能力。
pub(super) fn register(state: &AppState) {
    register_cex_adapters(state);
    register_hyperliquid_family(state);
    initialize_account_reader_or_warn(state);
    log_registered_adapters(state);
}

fn register_cex_adapters(state: &AppState) {
    register_adapter(state, "binance", build_binance());
    register_adapter(state, "okx", build_okx());
    register_adapter(state, "bybit", build_bybit());
    register_adapter(state, "bitget", build_bitget());
    register_adapter(state, "gate", build_gate());
    register_adapter(state, "kucoin", build_kucoin());
    register_adapter(state, "kraken", build_kraken());
    register_adapter(state, "gate_crossex", build_gate_crossex());
}

fn initialize_account_reader_or_warn(state: &AppState) {
    if let Err(error) = initialize_account_reader(state) {
        warn!(%error, "failed to initialize credential account reader");
    }
}

fn log_registered_adapters(state: &AppState) {
    let names = state.aggregator().names();
    info!(count = names.len(), exchanges = ?names, "exchange registration complete");
}

pub(super) fn refresh(state: &AppState, venue: &str) -> Result<(), String> {
    let venue = shared_types::normalized_venue_name(venue);
    let venue_id = resolve_venue_id(&venue)?;
    if is_hyperliquid_core(venue_id, &venue) {
        return refresh_hyperliquid_account_family(state, &venue);
    }
    refresh_single_account_adapter(state, &venue)
}

fn resolve_venue_id(venue: &str) -> Result<exchange::VenueId, String> {
    exchange::VenueId::from_exchange_name(venue).ok_or_else(|| format!("unknown exchange: {venue}"))
}

fn is_hyperliquid_core(venue_id: exchange::VenueId, venue: &str) -> bool {
    venue_id == exchange::VenueId::Hyperliquid && venue == exchange::VenueId::Hyperliquid.as_str()
}

fn refresh_hyperliquid_account_family(state: &AppState, venue: &str) -> Result<(), String> {
    if state.aggregator().get(venue).is_none() {
        register_hyperliquid_family(state);
    }
    refresh_account_reader(state)?;
    info!(
        exchange = venue,
        "private account adapter family refreshed; public market adapters preserved"
    );
    Ok(())
}

fn refresh_single_account_adapter(state: &AppState, venue: &str) -> Result<(), String> {
    if state.aggregator().get(venue).is_none() {
        let adapter = build_adapter(venue)?;
        state.aggregator().register(adapter);
    }
    refresh_account_reader(state)?;
    info!(
        exchange = venue,
        "private account adapter refreshed; public market adapter preserved"
    );
    Ok(())
}

fn initialize_account_reader(state: &AppState) -> Result<(), String> {
    state
        .trading_service()
        .initialize_account_reader(trading_credentials::current_adapter_credentials())
        .map_err(|error| error.to_string())
}

fn refresh_account_reader(state: &AppState) -> Result<(), String> {
    state
        .trading_service()
        .refresh_account_reader(trading_credentials::current_adapter_credentials())
        .map_err(|error| error.to_string())
}

fn build_adapter(venue: &str) -> Result<Arc<dyn ExchangeAdapter>, String> {
    let venue = shared_types::normalized_venue_name(venue);
    match venue.as_str() {
        "binance" => arc(build_binance()),
        "okx" => arc(build_okx()),
        "bybit" => arc(build_bybit()),
        "bitget" => arc(build_bitget()),
        "gate" => arc(build_gate()),
        "kucoin" => arc(build_kucoin()),
        "kraken" => arc(build_kraken()),
        "gate_crossex" => arc(build_gate_crossex()),
        "hyperliquid" => arc(build_hyperliquid()),
        "hyperliquid:xyz" => arc(build_hyperliquid_builder(exchange::HyperliquidMarket::XYZ)),
        "hyperliquid:cash" => arc(build_hyperliquid_builder(exchange::HyperliquidMarket::CASH)),
        "hyperliquid:flx" => arc(build_hyperliquid_builder(exchange::HyperliquidMarket::FLX)),
        "hyperliquid:km" => arc(build_hyperliquid_builder(exchange::HyperliquidMarket::KM)),
        "hyperliquid:vntl" => arc(build_hyperliquid_builder(exchange::HyperliquidMarket::VNTL)),
        other => Err(format!("unknown exchange: {other}")),
    }
}

fn arc<T>(result: Result<T, exchange::ExchangeError>) -> Result<Arc<dyn ExchangeAdapter>, String>
where
    T: ExchangeAdapter + 'static,
{
    result
        .map(|adapter| Arc::new(adapter) as Arc<dyn ExchangeAdapter>)
        .map_err(|error| error.to_string())
}

fn register_adapter<T>(
    state: &AppState,
    name: &'static str,
    result: Result<T, exchange::ExchangeError>,
) where
    T: ExchangeAdapter + 'static,
{
    if let Some(adapter) = adapter_from_result(name, result) {
        state.aggregator().register(adapter);
        info!(exchange = name, "registered");
    }
}

fn adapter_from_result<T>(
    name: &'static str,
    result: Result<T, exchange::ExchangeError>,
) -> Option<Arc<dyn ExchangeAdapter>>
where
    T: ExchangeAdapter + 'static,
{
    match result {
        Ok(adapter) => Some(Arc::new(adapter)),
        Err(error) => {
            log_adapter_error(name, &error);
            None
        }
    }
}

fn log_adapter_error(name: &'static str, error: &exchange::ExchangeError) {
    warn!(exchange = name, error = %error, "failed to register; skipping");
}

fn register_hyperliquid_family(state: &AppState) {
    register_adapter(state, "hyperliquid", build_hyperliquid());
    for market in HYPERLIQUID_BUILDER_MARKETS {
        register_adapter(state, market.venue(), build_hyperliquid_builder(*market));
    }
}

fn build_binance() -> Result<Binance, exchange::ExchangeError> {
    Binance::new(BinanceConfig {
        credentials: env::pair("BINANCE_API_KEY", "BINANCE_API_SECRET").map(|(k, s)| {
            BinanceCredentials {
                api_key: k,
                api_secret: s,
            }
        }),
        allow_live_writes: false,
        ..Default::default()
    })
}

fn build_okx() -> Result<Okx, exchange::ExchangeError> {
    Okx::new(OkxConfig {
        credentials: env::triple("OKX_API_KEY", "OKX_API_SECRET", "OKX_PASSPHRASE").map(
            |(k, s, p)| OkxCredentials {
                api_key: k,
                api_secret: s,
                passphrase: p,
            },
        ),
        ..Default::default()
    })
}

fn build_bybit() -> Result<Bybit, exchange::ExchangeError> {
    Bybit::new(BybitConfig {
        credentials: env::pair("BYBIT_API_KEY", "BYBIT_API_SECRET").map(|(k, s)| {
            BybitCredentials {
                api_key: k,
                api_secret: s,
            }
        }),
        allow_live_writes: false,
        ..Default::default()
    })
}

fn build_bitget() -> Result<Bitget, exchange::ExchangeError> {
    Bitget::new(BitgetConfig {
        credentials: env::triple("BITGET_API_KEY", "BITGET_API_SECRET", "BITGET_PASSPHRASE").map(
            |(k, s, p)| BitgetCredentials {
                api_key: k,
                api_secret: s,
                passphrase: p,
            },
        ),
        allow_live_writes: false,
        ..Default::default()
    })
}

fn build_gate() -> Result<Gate, exchange::ExchangeError> {
    Gate::new(GateConfig {
        credentials: env::pair("GATE_API_KEY", "GATE_API_SECRET").map(|(k, s)| GateCredentials {
            api_key: k,
            api_secret: s,
        }),
        allow_live_writes: false,
        ..Default::default()
    })
}

fn build_gate_crossex() -> Result<GateCrossEx, exchange::ExchangeError> {
    GateCrossEx::new(GateCrossExConfig {
        credentials: env::pair("GATE_CROSSEX_API_KEY", "GATE_CROSSEX_API_SECRET").map(
            |(api_key, api_secret)| GateCrossExCredentials {
                api_key,
                api_secret,
            },
        ),
        ..Default::default()
    })
}

fn build_kraken() -> Result<Kraken, exchange::ExchangeError> {
    let spot =
        env::pair("KRAKEN_SPOT_API_KEY", "KRAKEN_SPOT_API_SECRET").map(|(api_key, api_secret)| {
            KrakenSpotCredentials {
                api_key,
                api_secret,
            }
        });
    let futures = env::pair("KRAKEN_FUTURES_API_KEY", "KRAKEN_FUTURES_API_SECRET").map(
        |(api_key, api_secret)| KrakenFuturesCredentials {
            api_key,
            api_secret,
        },
    );
    Kraken::new(KrakenConfig {
        credentials: (spot.is_some() || futures.is_some())
            .then_some(KrakenCredentials { spot, futures }),
        allow_live_writes: false,
        ..Default::default()
    })
}

fn build_kucoin() -> Result<Kucoin, exchange::ExchangeError> {
    Kucoin::new(KucoinConfig {
        credentials: env::triple("KUCOIN_API_KEY", "KUCOIN_API_SECRET", "KUCOIN_PASSPHRASE").map(
            |(k, s, p)| KucoinCredentials {
                api_key: k,
                api_secret: s,
                passphrase: p,
            },
        ),
        ..Default::default()
    })
}

fn build_hyperliquid() -> Result<Hyperliquid, exchange::ExchangeError> {
    build_hyperliquid_market(exchange::HyperliquidMarket::Core)
}

fn build_hyperliquid_builder(
    market: exchange::HyperliquidMarket,
) -> Result<Hyperliquid, exchange::ExchangeError> {
    build_hyperliquid_market(market)
}

fn build_hyperliquid_market(
    market: exchange::HyperliquidMarket,
) -> Result<Hyperliquid, exchange::ExchangeError> {
    Hyperliquid::new(HyperliquidConfig {
        credentials: hyperliquid_credentials(),
        market,
        allow_live_writes: false,
        ..Default::default()
    })
}

fn hyperliquid_credentials() -> Option<HyperliquidCredentials> {
    let user_address =
        env::var("HYPERLIQUID_ACCOUNT_ADDRESS").or_else(|| env::var("HYPERLIQUID_USER_ADDRESS"))?;
    Some(HyperliquidCredentials {
        user_address,
        private_key: env::var("HYPERLIQUID_PRIVATE_KEY"),
        // 修复 P2 9.8：可选 vault 地址（HYPERLIQUID_VAULT_ADDRESS），未设置 = 主账户
        vault_address: env::var("HYPERLIQUID_VAULT_ADDRESS"),
    })
}

const HYPERLIQUID_BUILDER_MARKETS: &[exchange::HyperliquidMarket] =
    &[exchange::HyperliquidMarket::XYZ];

#[cfg(test)]
#[path = "exchanges_tests.rs"]
mod tests;
