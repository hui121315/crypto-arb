use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioSummary {
    pub total_nav_usd: f64,
    #[serde(default)]
    pub nav_evidence: PortfolioNavEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav_change_24h_pct: Option<f64>,
    pub net_delta_usd: f64,
    pub net_delta_pct_of_nav: f64,
    pub naked_exposure_usd: f64,
    pub naked_position_count: u32,
    pub realized_pnl_today_usd: f64,
    pub pnl_breakdown: PnlBreakdown,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioNavEvidence {
    pub status: AccountFieldQualityStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default)]
    pub breakdown: PortfolioNavBreakdown,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covered_venues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_venues: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl Default for PortfolioNavEvidence {
    fn default() -> Self {
        Self {
            status: AccountFieldQualityStatus::Missing,
            source: "account_state.account_summaries".to_owned(),
            observed_at_ms: 0,
            breakdown: PortfolioNavBreakdown::default(),
            covered_venues: Vec::new(),
            missing_venues: Vec::new(),
            problem: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioNavBreakdown {
    pub wallet_equity: PortfolioValueEvidence,
    pub position_equity: PortfolioValueEvidence,
    pub cash: PortfolioValueEvidence,
    pub unrealized_pnl: PortfolioValueEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioValueEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_usd: Option<f64>,
    pub status: AccountFieldQualityStatus,
    pub source: String,
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl Default for PortfolioValueEvidence {
    fn default() -> Self {
        Self {
            value_usd: None,
            status: AccountFieldQualityStatus::Missing,
            source: "portfolio_nav_component_unavailable".to_owned(),
            observed_at_ms: 0,
            problem: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PnlBreakdown {
    pub funding_usd: f64,
    pub price_usd: f64,
    pub fee_rebate_usd: f64,
    #[serde(default)]
    pub evidence: PortfolioPnlEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioPnlEvidence {
    pub quality: ExecutionLedgerQuality,
    pub source: String,
    pub observed_at_ms: i64,
    pub realized_group_count: u32,
    pub close_run_count: u32,
    pub unwind_run_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actual_fields: Vec<ReviewPnlField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub estimated_fields: Vec<ReviewPnlField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_fields: Vec<ReviewPnlField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
}

impl Default for PortfolioPnlEvidence {
    fn default() -> Self {
        Self {
            quality: ExecutionLedgerQuality::Missing,
            source: "portfolio_pnl_unavailable".to_owned(),
            observed_at_ms: 0,
            realized_group_count: 0,
            close_run_count: 0,
            unwind_run_count: 0,
            actual_fields: Vec::new(),
            estimated_fields: Vec::new(),
            missing_fields: Vec::new(),
            problem: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionRow {
    pub venue: String,
    pub symbol: String,
    #[serde(default)]
    pub origin: PositionOrigin,
    pub side: PositionSide,
    pub quantity: f64,
    pub entry_price: f64,
    pub mark_price: f64,
    pub leverage: f64,
    pub unrealized_pnl_usd: f64,
    pub liquidation_price: Option<f64>,
    pub liquidation_distance_pct: Option<f64>,
    pub next_funding_ms: Option<i64>,
    #[serde(default)]
    pub funding_rate_8h: f64,
    #[serde(default)]
    pub funding_rate_verified: bool,
    #[serde(default)]
    pub maintenance_margin_ratio: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair_evidence: Option<PositionPairEvidence>,
    pub paired_with: Option<String>,
    pub margin_usd: f64,
    #[serde(default)]
    pub severity: PositionSeverity,
    #[serde(default)]
    pub seconds_until_funding: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionOrigin {
    #[default]
    AccountPrivate,
    ExecutionLedger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionPairEvidenceSource {
    ExecutionRun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionPairEvidence {
    pub source: PositionPairEvidenceSource,
    pub run_id: String,
    pub ticket_id: String,
    pub opportunity_id: String,
    pub venue: String,
    pub symbol: String,
    pub side: PositionSide,
    pub partner_venue: String,
    pub partner_symbol: String,
    pub partner_side: PositionSide,
    pub leg_filled_quantity: f64,
    pub partner_filled_quantity: f64,
    pub matched_notional_usd: f64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSide {
    Long,
    Short,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSeverity {
    #[default]
    Unknown,
    Ok,
    Warn,
    Danger,
}
