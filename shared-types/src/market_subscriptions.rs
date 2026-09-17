use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueMarketSubscription {
    pub venue: String,
    pub spot_enabled: bool,
    pub perp_enabled: bool,
    pub funding_enabled: bool,
}

impl VenueMarketSubscription {
    pub fn enabled(venue: impl Into<String>) -> Self {
        Self {
            venue: venue.into(),
            spot_enabled: true,
            perp_enabled: true,
            funding_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSubscriptionsResponse {
    pub updated_at_ms: i64,
    pub venues: Vec<VenueMarketSubscription>,
    #[serde(default)]
    pub runtime: Vec<VenueMarketSubscriptionRuntime>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketSubscriptionRuntimeState {
    Disabled,
    #[default]
    Warming,
    Live,
    Degraded,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSubscriptionFeedRuntime {
    pub state: MarketSubscriptionRuntimeState,
    pub rows: u64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub observed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueMarketSubscriptionRuntime {
    pub venue: String,
    pub spot: MarketSubscriptionFeedRuntime,
    pub perp: MarketSubscriptionFeedRuntime,
    pub funding: MarketSubscriptionFeedRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSubscriptionPatch {
    pub venue: String,
    #[serde(default)]
    pub spot_enabled: Option<bool>,
    #[serde(default)]
    pub perp_enabled: Option<bool>,
    #[serde(default)]
    pub funding_enabled: Option<bool>,
}
