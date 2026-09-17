use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskSnapshot {
    pub var_99_1d_usd: f64,
    pub var_pct_of_nav: f64,
    /// 计算 99% 历史 VaR 所用的样本数量；样本不足时（< `VAR_99_MIN_SAMPLES`）
    /// 该 VaR 不具统计意义，消费端应提示「样本不足」而非当作可信风险数。
    #[serde(default)]
    pub var_sample_size: usize,
    pub funding_clustering: Vec<FundingCluster>,
    pub delta_concentration: Vec<DeltaPerAsset>,
    pub margin_utilization: Vec<MarginPerVenue>,
    pub hard_limits: HardLimitsUsage,
    pub updated_at_ms: i64,
}

/// 历史 99% VaR 至少需要的样本数：1% 尾部要落到真实观测上需要约百级样本，
/// 低于该阈值时分位数会塌缩到最差单点，不具统计意义。
pub const VAR_99_MIN_SAMPLES: usize = 100;

impl RiskSnapshot {
    /// 历史 VaR 样本是否充足；不足时前端应提示「样本不足」而非展示可信风险值。
    pub fn var_sample_sufficient(&self) -> bool {
        self.var_sample_size >= VAR_99_MIN_SAMPLES
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingCluster {
    pub settles_in_minutes: u32,
    pub position_count: u32,
    pub total_outflow_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaPerAsset {
    pub asset: String,
    pub net_qty: f64,
    pub net_notional_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarginPerVenue {
    pub venue: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utilization_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_margin_usd: Option<f64>,
    pub maintenance_margin_usd: f64,
    pub equity_usd: f64,
    /// 该 venue 缺少可用账户摘要，当前占用率只能退回仓位口径估算。
    #[serde(default)]
    pub estimated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardLimitsUsage {
    pub open_orders_used: u32,
    pub open_orders_max: u32,
    pub max_symbol_notional_usd: f64,
    pub max_order_notional_usd: f64,
    pub kill_switch_active: bool,
}
