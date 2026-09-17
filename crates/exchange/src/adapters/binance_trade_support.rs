use super::Binance;
use crate::adapters::binance_config::{PROD_WS_TRADE, SPOT_PROD_WS_TRADE, TESTNET_WS_TRADE};
use crate::adapters::binance_ws_trade::WsTradeConfig;
use crate::error::{ExchangeError, ExchangeResult};
use std::sync::atomic::Ordering;

impl Binance {
    pub(super) fn ensure_write_adapter(&self) -> ExchangeResult<()> {
        if self.config.testnet
            || self.config.allow_live_writes
            || self.config.base_url_override.is_some()
        {
            Ok(())
        } else {
            Err(ExchangeError::Auth(
                "binance live trading disabled; enable live writes explicitly".into(),
            ))
        }
    }

    fn ws_trade_url(&self) -> &'static str {
        if self.config.testnet {
            TESTNET_WS_TRADE
        } else {
            PROD_WS_TRADE
        }
    }

    pub(super) fn use_ws_request_api(&self) -> bool {
        self.config.base_url_override.is_none()
    }

    pub(super) fn ws_trade_config(&self) -> ExchangeResult<WsTradeConfig<'_>> {
        let credentials = self.require_credentials()?;
        Ok(WsTradeConfig {
            url: self.ws_trade_url(),
            api_key: &credentials.api_key,
            api_secret: &credentials.api_secret,
            timeout_secs: self.config.timeout_secs,
            time_offset_ms: self.time_offset_ms.load(Ordering::Relaxed),
        })
    }

    pub(super) fn spot_ws_trade_config(&self) -> ExchangeResult<WsTradeConfig<'_>> {
        if self.config.testnet {
            return Err(ExchangeError::NotImplemented(
                "binance spot testnet WebSocket trading",
            ));
        }
        let credentials = self.require_credentials()?;
        Ok(WsTradeConfig {
            url: SPOT_PROD_WS_TRADE,
            api_key: &credentials.api_key,
            api_secret: &credentials.api_secret,
            timeout_secs: self.config.timeout_secs,
            time_offset_ms: self.time_offset_ms.load(Ordering::Relaxed),
        })
    }
}
