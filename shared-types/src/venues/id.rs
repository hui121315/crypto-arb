use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueId {
    Binance,
    Okx,
    Bybit,
    Bitget,
    Gate,
    #[serde(rename = "gate_crossex")]
    GateCrossEx,
    Htx,
    Kraken,
    Kucoin,
    Hyperliquid,
}

impl VenueId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Binance => "binance",
            Self::Okx => "okx",
            Self::Bybit => "bybit",
            Self::Bitget => "bitget",
            Self::Gate => "gate",
            Self::GateCrossEx => "gate_crossex",
            Self::Htx => "htx",
            Self::Kraken => "kraken",
            Self::Kucoin => "kucoin",
            Self::Hyperliquid => "hyperliquid",
        }
    }

    pub fn from_exchange_name(name: &str) -> Option<Self> {
        let family = venue_family(name).to_ascii_lowercase();
        match strip_live_suffix(&family) {
            "binance" => Some(Self::Binance),
            "okx" => Some(Self::Okx),
            "bybit" => Some(Self::Bybit),
            "bitget" => Some(Self::Bitget),
            "gate" => Some(Self::Gate),
            "gate_crossex" | "gate-crossex" | "crossex" => Some(Self::GateCrossEx),
            "htx" => Some(Self::Htx),
            "kraken" => Some(Self::Kraken),
            "kucoin" => Some(Self::Kucoin),
            "hyperliquid" => Some(Self::Hyperliquid),
            _ => None,
        }
    }

    pub const fn defaults(self) -> VenueDefaults {
        match self {
            Self::Binance => VenueDefaults {
                qps: 20,
                timeout_secs: 30,
                fanout_timeout_secs: 10,
            },
            Self::Okx => VenueDefaults {
                qps: 8,
                timeout_secs: 30,
                fanout_timeout_secs: 30,
            },
            Self::Bybit => VenueDefaults {
                qps: 50,
                timeout_secs: 30,
                fanout_timeout_secs: 10,
            },
            Self::Bitget => VenueDefaults {
                qps: 10,
                timeout_secs: 30,
                fanout_timeout_secs: 10,
            },
            Self::Gate => VenueDefaults {
                qps: 20,
                timeout_secs: 30,
                fanout_timeout_secs: 10,
            },
            Self::GateCrossEx => VenueDefaults {
                qps: 10,
                timeout_secs: 30,
                fanout_timeout_secs: 10,
            },
            Self::Htx => VenueDefaults {
                qps: 10,
                timeout_secs: 30,
                fanout_timeout_secs: 10,
            },
            Self::Kraken => VenueDefaults {
                qps: 5,
                timeout_secs: 30,
                fanout_timeout_secs: 10,
            },
            Self::Kucoin => VenueDefaults {
                qps: 20,
                timeout_secs: 30,
                fanout_timeout_secs: 20,
            },
            Self::Hyperliquid => VenueDefaults {
                qps: 20,
                timeout_secs: 30,
                fanout_timeout_secs: 15,
            },
        }
    }
}

fn strip_live_suffix(value: &str) -> &str {
    value
        .strip_suffix("_live")
        .or_else(|| value.strip_suffix("-live"))
        .unwrap_or(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueDefaults {
    pub qps: u32,
    pub timeout_secs: u64,
    pub fanout_timeout_secs: u64,
}
