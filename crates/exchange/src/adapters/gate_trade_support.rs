use super::Gate;
use crate::adapters::gate_config::{
    PROD_WS_TRADE, SPOT_PROD_WS_TRADE, SPOT_TESTNET_WS_TRADE, TESTNET_WS_TRADE,
};
use crate::adapters::gate_spot_ws_trade::WsSpotTradeConfig;
use crate::adapters::gate_ws_trade::WsTradeConfig;
use crate::error::{ExchangeError, ExchangeResult};
use std::sync::atomic::Ordering;

impl Gate {
    pub(super) fn ensure_write_adapter(&self) -> ExchangeResult<()> {
        if self.config.allow_live_writes {
            Ok(())
        } else {
            Err(ExchangeError::Auth(
                "gate live trading disabled; enable live writes explicitly".into(),
            ))
        }
    }

    pub(super) fn ws_trade_config(&self) -> ExchangeResult<WsTradeConfig<'_>> {
        let credentials = self.require_credentials()?;
        // 修复 P2 5.4：testnet 时切换 WS endpoint。
        let url = if self.config.testnet {
            TESTNET_WS_TRADE
        } else {
            PROD_WS_TRADE
        };
        Ok(WsTradeConfig {
            url,
            api_key: &credentials.api_key,
            api_secret: &credentials.api_secret,
            timeout_secs: self.config.timeout_secs,
            // 修复 P2 5.9：把 server-local offset 透传给 WS login。
            time_offset_secs: self.time_offset_secs.load(Ordering::Relaxed),
        })
    }

    pub(super) fn spot_ws_trade_config(&self) -> ExchangeResult<WsSpotTradeConfig<'_>> {
        let credentials = self.require_credentials()?;
        Ok(WsSpotTradeConfig {
            url: if self.config.testnet {
                SPOT_TESTNET_WS_TRADE
            } else {
                SPOT_PROD_WS_TRADE
            },
            api_key: &credentials.api_key,
            api_secret: &credentials.api_secret,
            timeout_secs: self.config.timeout_secs,
            time_offset_secs: self
                .time_offset_secs
                .load(std::sync::atomic::Ordering::Relaxed),
        })
    }
}
