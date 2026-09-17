//! Bitget adapter configuration types.
//!
//! V3 / UTA inherits `BitgetMarginMode` / `BitgetCredentials` / `BitgetConfig`
//! verbatim. The legacy V2 `PRODUCT_TYPE` constant was removed alongside the
//! rest of the V2 surface in PR-DP-13 · B-7; product routing is now
//! expressed through `BitgetUtaCategory` in `bitget_uta_config.rs`.

use crate::venue_spec::VenueId;

#[derive(Debug, Clone)]
pub struct BitgetCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub passphrase: String,
}

/// Bitget V2 `marginMode`（修复 P1 4.2）。
///
/// Bitget 下单时必填，可选 `crossed` / `isolated`，必须匹配账户实际仓位模式。
/// 用户在 Bitget 客户端把仓位模式设为 isolated 后，传 `crossed` 下单会触发
/// 错误码 40725 / 40808。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BitgetMarginMode {
    #[default]
    Crossed,
    Isolated,
}

impl BitgetMarginMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Crossed => "crossed",
            Self::Isolated => "isolated",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BitgetConfig {
    pub credentials: Option<BitgetCredentials>,
    pub allow_live_writes: bool,
    pub timeout_secs: u64,
    pub qps: u32,
    pub base_url_override: Option<String>,
    /// 修复 P1 4.2：下单时使用的 marginMode，默认 `Crossed`，isolated 用户需显式设置。
    pub margin_mode: BitgetMarginMode,
}

impl Default for BitgetConfig {
    fn default() -> Self {
        let defaults = VenueId::Bitget.defaults();
        Self {
            credentials: None,
            allow_live_writes: false,
            timeout_secs: defaults.timeout_secs,
            qps: defaults.qps,
            base_url_override: None,
            margin_mode: BitgetMarginMode::Crossed,
        }
    }
}
