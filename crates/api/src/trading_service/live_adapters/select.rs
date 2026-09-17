use super::*;

impl TradingService {
    #[cfg(test)]
    pub(crate) fn select_binance_testnet_adapter(
        &self,
        api_key: String,
        api_secret: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = Binance::new(BinanceConfig {
            credentials: Some(BinanceCredentials {
                api_key,
                api_secret,
            }),
            testnet: true,
            allow_live_writes: false,
            timeout_secs: 10,
            qps: 5,
            base_url_override: None,
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name("binance_testnet");
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = false;
            config.allowed_exchanges = BTreeSet::from(["binance".to_owned()]);
        }))
    }

    #[cfg(test)]
    pub(crate) fn select_binance_live_adapter(
        &self,
        api_key: String,
        api_secret: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = Binance::new(BinanceConfig {
            credentials: Some(BinanceCredentials {
                api_key,
                api_secret,
            }),
            testnet: false,
            allow_live_writes: true,
            timeout_secs: 10,
            qps: 2,
            base_url_override: None,
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name(BINANCE_LIVE_ADAPTER_ID);
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = true;
            config.allowed_exchanges = BTreeSet::from(["binance".to_owned()]);
        }))
    }

    #[cfg(test)]
    pub(crate) fn select_okx_testnet_adapter(
        &self,
        api_key: String,
        api_secret: String,
        passphrase: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = OkxLive::new(OkxLiveConfig {
            credentials: OkxLiveCredentials {
                api_key,
                api_secret,
                passphrase,
            },
            testnet: true,
            timeout_secs: 10,
            qps: 5,
            base_url_override: None,
            // 修复 P1 2.4：OKX V5 td_mode 必填，Cross 是衡生品账户默认
            td_mode: exchange::adapters::OkxTdMode::Cross,
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name("okx_testnet");
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = false;
            config.allowed_exchanges = BTreeSet::from(["okx".to_owned()]);
        }))
    }

    #[cfg(test)]
    pub(crate) fn select_bybit_live_adapter(
        &self,
        api_key: String,
        api_secret: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = Bybit::new(BybitConfig {
            credentials: Some(BybitCredentials {
                api_key,
                api_secret,
            }),
            testnet: false,
            allow_live_writes: true,
            timeout_secs: 10,
            qps: 2,
            ..Default::default()
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name(BYBIT_LIVE_ADAPTER_ID);
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = true;
            config.allowed_exchanges = BTreeSet::from(["bybit".to_owned()]);
        }))
    }

    #[cfg(test)]
    pub(crate) fn select_bitget_live_adapter(
        &self,
        api_key: String,
        api_secret: String,
        passphrase: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = Bitget::new(BitgetConfig {
            credentials: Some(BitgetCredentials {
                api_key,
                api_secret,
                passphrase,
            }),
            allow_live_writes: true,
            timeout_secs: 10,
            qps: 2,
            base_url_override: None,
            // 修复 P1 4.2：Bitget V2 marginMode 必填，Crossed 是衍生品账户默认
            margin_mode: exchange::adapters::BitgetMarginMode::Crossed,
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name(BITGET_LIVE_ADAPTER_ID);
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = true;
            config.allowed_exchanges = BTreeSet::from(["bitget".to_owned()]);
        }))
    }

    #[cfg(test)]
    pub(crate) fn select_gate_live_adapter(
        &self,
        api_key: String,
        api_secret: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = Gate::new(GateConfig {
            credentials: Some(GateCredentials {
                api_key,
                api_secret,
            }),
            allow_live_writes: true,
            timeout_secs: 10,
            qps: 2,
            base_url_override: None,
            // 修复 P2 5.4：默认接生产网；如需 testnet 通过外部配置覆盖即可。
            testnet: false,
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name(GATE_LIVE_ADAPTER_ID);
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = true;
            config.allowed_exchanges = BTreeSet::from(["gate".to_owned()]);
        }))
    }

    #[cfg(test)]
    pub(crate) fn select_kucoin_live_adapter(
        &self,
        api_key: String,
        api_secret: String,
        passphrase: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = Kucoin::new(KucoinConfig {
            credentials: Some(KucoinCredentials {
                api_key,
                api_secret,
                passphrase,
            }),
            allow_live_writes: true,
            timeout_secs: 10,
            qps: 2,
            base_url_override: None,
            margin_mode: exchange::adapters::KucoinMarginMode::Cross,
            default_leverage: 1,
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name(KUCOIN_LIVE_ADAPTER_ID);
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = true;
            config.allowed_exchanges = BTreeSet::from(["kucoin".to_owned()]);
        }))
    }

    #[cfg(test)]
    pub(crate) fn select_okx_live_adapter(
        &self,
        api_key: String,
        api_secret: String,
        passphrase: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = OkxLive::new(OkxLiveConfig {
            credentials: OkxLiveCredentials {
                api_key,
                api_secret,
                passphrase,
            },
            testnet: false,
            timeout_secs: 10,
            qps: 2,
            base_url_override: None,
            // 修复 P1 2.4：OKX V5 td_mode 必填，Cross 是衡生品账户默认
            td_mode: exchange::adapters::OkxTdMode::Cross,
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name(OKX_LIVE_ADAPTER_ID);
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = true;
            config.allowed_exchanges = BTreeSet::from(["okx".to_owned()]);
        }))
    }

    #[cfg(test)]
    pub(crate) fn select_hyperliquid_live_adapter(
        &self,
        user_address: String,
        private_key: String,
    ) -> Result<RiskConfig, exchange::ExchangeError> {
        let adapter = Hyperliquid::new(HyperliquidConfig {
            credentials: Some(HyperliquidCredentials {
                user_address,
                private_key: Some(private_key),
                // 修复 P2 9.8：默认主账户签名；未来通过 service API 支持显式 vault
                vault_address: None,
            }),
            market: exchange::HyperliquidMarket::Core,
            allow_live_writes: true,
            timeout_secs: 10,
            qps: 2,
            base_url_override: None,
            // 修复 P2 9.8：默认不限 expiresAfter；运维可在 config 调；conservative=不破坏现状
            action_expires_after_ms: None,
        })?;
        self.engine.set_adapter(Arc::new(adapter));
        self.set_adapter_name(HYPERLIQUID_LIVE_ADAPTER_ID);
        Ok(self.risk.update_config(|config| {
            config.live_trading_enabled = true;
            config.allowed_exchanges = BTreeSet::from(["hyperliquid".to_owned()]);
        }))
    }
}
