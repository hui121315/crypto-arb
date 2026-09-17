//! Hyperliquid adapter configuration types.

use crate::venue_spec::VenueId;

pub(super) const PROD_BASE: &str = "https://api.hyperliquid.xyz";
pub(super) const PROD_WS_TRADE: &str = "wss://api.hyperliquid.xyz/ws";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyperliquidMarket {
    Core,
    BuilderDex {
        venue: &'static str,
        dex: &'static str,
    },
}

impl HyperliquidMarket {
    /// Markets with currently verified executable instruments. Retired builder
    /// constants remain available for historical event parsing.
    pub const CONFIGURED_MARKETS: &[Self] = &[Self::Core, Self::XYZ];

    pub const XYZ: Self = Self::BuilderDex {
        venue: "hyperliquid:xyz",
        dex: "xyz",
    };
    pub const CASH: Self = Self::BuilderDex {
        venue: "hyperliquid:cash",
        dex: "cash",
    };
    pub const FLX: Self = Self::BuilderDex {
        venue: "hyperliquid:flx",
        dex: "flx",
    };
    pub const KM: Self = Self::BuilderDex {
        venue: "hyperliquid:km",
        dex: "km",
    };
    pub const VNTL: Self = Self::BuilderDex {
        venue: "hyperliquid:vntl",
        dex: "vntl",
    };

    pub const fn venue(self) -> &'static str {
        match self {
            Self::Core => "hyperliquid",
            Self::BuilderDex { venue, .. } => venue,
        }
    }

    pub(super) const fn dex(self) -> Option<&'static str> {
        match self {
            Self::Core => None,
            Self::BuilderDex { dex, .. } => Some(dex),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HyperliquidCredentials {
    /// Public EVM address used for account reads.
    pub user_address: String,
    /// Private key used only by live write actions.
    pub private_key: Option<String>,
    /// Optional vault address for owner-key delegated vault actions.
    pub vault_address: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HyperliquidConfig {
    pub credentials: Option<HyperliquidCredentials>,
    pub market: HyperliquidMarket,
    pub allow_live_writes: bool,
    pub timeout_secs: u64,
    pub qps: u32,
    pub base_url_override: Option<String>,
    /// Optional live action expiry window in milliseconds.
    pub action_expires_after_ms: Option<u64>,
}

impl Default for HyperliquidConfig {
    fn default() -> Self {
        let defaults = VenueId::Hyperliquid.defaults();
        Self {
            credentials: None,
            market: HyperliquidMarket::Core,
            allow_live_writes: false,
            timeout_secs: defaults.timeout_secs,
            qps: defaults.qps,
            base_url_override: None,
            action_expires_after_ms: None,
        }
    }
}
