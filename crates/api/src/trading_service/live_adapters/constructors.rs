use super::*;

pub(super) fn binance_live_adapter(
    api_key: String,
    api_secret: String,
) -> Result<LiveAdapter, ExchangeError> {
    Ok(Arc::new(Binance::new(binance_live_config(
        api_key, api_secret,
    ))?))
}

fn binance_live_config(api_key: String, api_secret: String) -> BinanceConfig {
    BinanceConfig {
        credentials: Some(BinanceCredentials {
            api_key,
            api_secret,
        }),
        testnet: false,
        allow_live_writes: true,
        timeout_secs: 10,
        // Binance QPS is expressed in official request-weight units. Keeping the venue default
        // avoids making a weight-30 background ledger read starve private account hot paths.
        qps: shared_types::VenueId::Binance.defaults().qps,
        base_url_override: None,
    }
}

pub(super) fn bybit_live_adapter(
    api_key: String,
    api_secret: String,
) -> Result<LiveAdapter, ExchangeError> {
    Ok(Arc::new(Bybit::new(BybitConfig {
        credentials: Some(BybitCredentials {
            api_key,
            api_secret,
        }),
        testnet: false,
        allow_live_writes: true,
        timeout_secs: 10,
        qps: 2,
        ..Default::default()
    })?))
}

pub(super) fn bitget_live_adapter(
    api_key: String,
    api_secret: String,
    passphrase: String,
) -> Result<LiveAdapter, ExchangeError> {
    Ok(Arc::new(Bitget::new(bitget_live_config(
        api_key, api_secret, passphrase,
    ))?))
}

fn bitget_live_config(api_key: String, api_secret: String, passphrase: String) -> BitgetConfig {
    BitgetConfig {
        credentials: Some(BitgetCredentials {
            api_key,
            api_secret,
            passphrase,
        }),
        allow_live_writes: true,
        timeout_secs: 10,
        // UTA account, position and open-order reads each permit 20 requests/s per UID. Keep
        // the shared conservative venue budget instead of imposing a second two-request queue.
        qps: shared_types::VenueId::Bitget.defaults().qps,
        base_url_override: None,
        margin_mode: exchange::adapters::BitgetMarginMode::Crossed,
    }
}

pub(super) fn gate_live_adapter(
    api_key: String,
    api_secret: String,
) -> Result<LiveAdapter, ExchangeError> {
    Ok(Arc::new(Gate::new(GateConfig {
        credentials: Some(GateCredentials {
            api_key,
            api_secret,
        }),
        allow_live_writes: true,
        timeout_secs: 10,
        qps: 2,
        base_url_override: None,
        testnet: false,
    })?))
}

pub(super) fn gate_crossex_live_adapter(
    api_key: String,
    api_secret: String,
) -> Result<LiveAdapter, ExchangeError> {
    Ok(Arc::new(GateCrossEx::new(GateCrossExConfig {
        credentials: Some(GateCrossExCredentials {
            api_key,
            api_secret,
        }),
        allow_live_writes: true,
        timeout_secs: 10,
        qps: shared_types::VenueId::GateCrossEx.defaults().qps,
        ..Default::default()
    })?))
}

pub(super) fn kraken_live_adapter(
    credentials: KrakenAdapterCredentials,
) -> Result<LiveAdapter, ExchangeError> {
    let spot = credentials
        .spot
        .map(|(api_key, api_secret)| KrakenSpotCredentials {
            api_key,
            api_secret,
        });
    let futures = credentials
        .futures
        .map(|(api_key, api_secret)| KrakenFuturesCredentials {
            api_key,
            api_secret,
        });
    Ok(Arc::new(Kraken::new(KrakenConfig {
        credentials: Some(KrakenCredentials { spot, futures }),
        allow_live_writes: true,
        timeout_secs: 10,
        qps: shared_types::VenueId::Kraken.defaults().qps,
        ..Default::default()
    })?))
}

pub(super) fn kucoin_live_adapter(
    api_key: String,
    api_secret: String,
    passphrase: String,
) -> Result<LiveAdapter, ExchangeError> {
    Ok(Arc::new(Kucoin::new(kucoin_live_config(
        api_key, api_secret, passphrase,
    ))?))
}

fn kucoin_live_config(api_key: String, api_secret: String, passphrase: String) -> KucoinConfig {
    KucoinConfig {
        credentials: Some(KucoinCredentials {
            api_key,
            api_secret,
            passphrase,
        }),
        allow_live_writes: true,
        timeout_secs: 10,
        // KuCoin Classic Futures private endpoints consume weighted units from one Futures pool.
        // Match the shared venue budget so weight-5 reads do not queue behind a local 2-unit cap.
        qps: shared_types::VenueId::Kucoin.defaults().qps,
        base_url_override: None,
        margin_mode: exchange::adapters::KucoinMarginMode::Cross,
        default_leverage: 1,
    }
}

pub(super) fn okx_live_adapter(
    api_key: String,
    api_secret: String,
    passphrase: String,
) -> Result<LiveAdapter, ExchangeError> {
    Ok(Arc::new(OkxLive::new(okx_live_config(
        api_key, api_secret, passphrase,
    ))?))
}

fn okx_live_config(api_key: String, api_secret: String, passphrase: String) -> OkxLiveConfig {
    OkxLiveConfig {
        credentials: OkxLiveCredentials {
            api_key,
            api_secret,
            passphrase,
        },
        testnet: false,
        timeout_secs: 10,
        // OKX account endpoints own independent user-scoped limits. The shared conservative
        // venue budget prevents aggregate bursts without adding another two-request bottleneck.
        qps: shared_types::VenueId::Okx.defaults().qps,
        base_url_override: None,
        td_mode: exchange::adapters::OkxTdMode::Cross,
    }
}

pub(super) fn hyperliquid_live_adapter(
    user_address: String,
    private_key: String,
    vault_address: Option<String>,
    market: exchange::HyperliquidMarket,
) -> Result<LiveAdapter, ExchangeError> {
    Ok(Arc::new(Hyperliquid::new(HyperliquidConfig {
        credentials: Some(HyperliquidCredentials {
            user_address,
            private_key: Some(private_key),
            vault_address,
        }),
        market,
        allow_live_writes: true,
        timeout_secs: 10,
        // `/info` is weighted in official units (1200/minute), not raw request
        // count. The shared family limiter still caps core plus builder DEXes.
        qps: shared_types::VenueId::Hyperliquid.defaults().qps,
        base_url_override: None,
        action_expires_after_ms: None,
    })?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binance_live_config_uses_weight_unit_budget() {
        let config = binance_live_config("key".to_owned(), "secret".to_owned());

        assert_eq!(config.qps, shared_types::VenueId::Binance.defaults().qps);
        assert_eq!(config.qps, 20);
    }

    #[test]
    fn kucoin_live_config_uses_futures_weight_unit_budget() {
        let config = kucoin_live_config(
            "key".to_owned(),
            "secret".to_owned(),
            "passphrase".to_owned(),
        );

        assert_eq!(config.qps, shared_types::VenueId::Kucoin.defaults().qps);
        assert_eq!(config.qps, 20);
    }

    #[test]
    fn bitget_live_config_uses_conservative_uta_budget() {
        let config = bitget_live_config(
            "key".to_owned(),
            "secret".to_owned(),
            "passphrase".to_owned(),
        );

        assert_eq!(config.qps, shared_types::VenueId::Bitget.defaults().qps);
        assert_eq!(config.qps, 10);
    }

    #[test]
    fn okx_live_config_uses_conservative_account_budget() {
        let config = okx_live_config(
            "key".to_owned(),
            "secret".to_owned(),
            "passphrase".to_owned(),
        );

        assert_eq!(config.qps, shared_types::VenueId::Okx.defaults().qps);
        assert_eq!(config.qps, 8);
    }

    #[tokio::test]
    async fn kraken_live_adapter_preserves_independent_product_credentials(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let adapter = kraken_live_adapter(KrakenAdapterCredentials {
            spot: Some(("spot-key".to_owned(), "c2VjcmV0".to_owned())),
            futures: None,
        })?;
        let capabilities = adapter.capabilities();
        assert!(capabilities.supports_live);
        assert!(capabilities.supports_spot);
        assert!(capabilities.supports_perp);
        Ok(())
    }
}
