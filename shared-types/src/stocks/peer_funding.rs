use super::comparison::SOLANA_USDC;
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockPeerFundingRequest {
    pub asset: String,
    pub selection: StockPeerSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockPeerFundingDirection {
    Deposit,
    Withdraw,
}

impl StockPeerFundingDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deposit => "deposit",
            Self::Withdraw => "withdraw",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Deposit => "充值",
            Self::Withdraw => "提现",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerFundingAmount {
    pub asset_class: String,
    pub asset: String,
    pub amount: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerFundingFees {
    pub base: StockPeerFundingAmount,
    pub included: bool,
    pub percentage: Option<String>,
    pub minimum: Option<StockPeerFundingAmount>,
    pub maximum: Option<StockPeerFundingAmount>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerFundingMethod {
    pub method_id: String,
    pub network_id: String,
    pub network_name: String,
    pub contract_address: Option<String>,
    pub minimum_amount: Option<String>,
    pub maximum_amount: Option<String>,
    pub fees: StockPeerFundingFees,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerFundingRoute {
    pub asset: String,
    pub asset_class: String,
    pub direction: StockPeerFundingDirection,
    /// Base token units, deliberately distinct from rebased orderbook shares.
    pub amount_unit: String,
    pub methods: Vec<StockPeerFundingMethod>,
    pub checked_at_ms: i64,
    pub source_url: String,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerFunding {
    pub asset: String,
    pub selection: StockPeerSelection,
    pub checked_at_ms: i64,
    pub routes: Vec<StockPeerFundingRoute>,
}

impl StockPeerFunding {
    pub fn current(&self, snapshot: &StockMarketSnapshot, now: i64) -> bool {
        let Some((stock, _)) = self.selection.native_symbol.split_once('/') else {
            return false;
        };
        snapshot
            .security
            .as_ref()
            .is_some_and(|s| s.asset == self.asset)
            && snapshot.peer.as_ref().is_some_and(|p| {
                p.selection == self.selection && p.share_unit_verified && p.problem.is_none()
            })
            && now >= self.checked_at_ms
            && now - self.checked_at_ms <= 60_000
            && self.routes.len() == 4
            && [(stock, "tokenized_asset"), ("USDC", "currency")]
                .into_iter()
                .all(|(asset, class)| {
                    [
                        StockPeerFundingDirection::Deposit,
                        StockPeerFundingDirection::Withdraw,
                    ]
                    .into_iter()
                    .all(|direction| {
                        self.routes
                            .iter()
                            .filter(|r| {
                                r.asset == asset
                                    && r.asset_class == class
                                    && r.direction == direction
                                    && r.amount_unit == "base"
                            })
                            .count()
                            == 1
                    })
                })
            && self
                .routes
                .iter()
                .all(|r| now >= r.checked_at_ms && now - r.checked_at_ms <= 60_000)
    }
}

/// Contract identity only. A match is not proof of account limits or a deposit address.
pub fn peer_funding_contract_matches(
    snapshot: &StockMarketSnapshot,
    route: &StockPeerFundingRoute,
    method: &StockPeerFundingMethod,
) -> Option<bool> {
    if !method.network_name.eq_ignore_ascii_case("solana") {
        return Some(false);
    }
    let actual = method
        .contract_address
        .as_deref()
        .filter(|s| !s.is_empty())?;
    let expected = if route.asset_class == "currency" && route.asset == "USDC" {
        SOLANA_USDC
    } else if route.asset_class == "tokenized_asset"
        && snapshot.peer.as_ref().is_some_and(|p| {
            p.selection
                .native_symbol
                .split_once('/')
                .is_some_and(|(base, _)| base == route.asset)
        })
    {
        &snapshot.comparison.as_ref()?.mint.address
    } else {
        return None;
    };
    Some(actual == expected)
}
