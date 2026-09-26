use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementSource {
    #[default]
    All,
    Onchain,
    CrossChain,
    Stocks,
    StockPeer,
}

impl SettlementSource {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::All => "all", Self::Onchain => "onchain", Self::CrossChain => "cross_chain",
            Self::Stocks => "stocks", Self::StockPeer => "stock_peer",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "all" => Self::All, "onchain" => Self::Onchain, "cross_chain" => Self::CrossChain,
            "stocks" => Self::Stocks, "stock_peer" => Self::StockPeer, _ => return None,
        })
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "全部来源", Self::Onchain => "链上 / CEX", Self::CrossChain => "跨链",
            Self::Stocks => "Backpack 股票", Self::StockPeer => "股票跨所",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettlementReviewQuery {
    #[serde(default)]
    pub source: SettlementSource,
    pub record: Option<String>,
}

impl SettlementReviewQuery {
    pub fn is_valid(&self) -> bool {
        self.record.as_ref().is_none_or(|id| self.source != SettlementSource::All
            && !id.trim().is_empty() && id.chars().count() <= 160 && !id.chars().any(char::is_control))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementReviewAmount {
    pub label: String,
    pub asset: String,
    pub amount: Option<String>,
}

/// Read-only projection of saved receipts. Amounts are not a unified realized-PnL ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementReviewRecord {
    pub source: SettlementSource,
    pub id: String,
    pub title: String,
    pub execution_state: String,
    pub accounting_state: String,
    pub attention: bool,
    pub updated_at_ms: i64,
    pub amounts: Vec<SettlementReviewAmount>,
    pub references: Vec<(String, String)>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementReviewSnapshot {
    pub rows: Vec<SettlementReviewRecord>,
    pub observed_at_ms: i64,
    pub truncated: bool,
    pub problems: Vec<String>,
}
