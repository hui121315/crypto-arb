use shared_types::{venue_names_equal, ArbitrageOpportunityDto};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategySpec {
    pub id: String,
    pub when: WhenClause,
    pub size: SizeClause,
    pub hedge: HedgeMode,
    pub limits: StrategyLimits,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhenClause {
    #[serde(default)]
    pub min_net_yield: Option<f64>,
    #[serde(default)]
    pub max_min_hold_periods: Option<u32>,
    #[serde(default)]
    pub venues_in: Vec<String>,
    #[serde(default)]
    pub symbols_in: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SizeClause {
    FixedUsd { value: f64 },
    CapitalPct { value: f64 },
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HedgeMode {
    SimultaneousBoth,
    LongFirst,
    ShortFirst,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyLimits {
    pub max_open_hedges: u32,
    pub max_daily_loss_usd: f64,
    pub cooldown_secs: u64,
}

impl StrategySpec {
    pub fn matches(&self, opp: &ArbitrageOpportunityDto) -> bool {
        self.enabled && self.when.matches(opp)
    }
}

impl WhenClause {
    pub fn matches(&self, opp: &ArbitrageOpportunityDto) -> bool {
        self.min_yield_matches(opp)
            && self.max_hold_matches(opp)
            && self.symbol_matches(opp)
            && self.venues_match(opp)
    }

    fn min_yield_matches(&self, opp: &ArbitrageOpportunityDto) -> bool {
        match self.min_net_yield {
            Some(min) => opp.net_single_yield >= min,
            None => true,
        }
    }

    fn max_hold_matches(&self, opp: &ArbitrageOpportunityDto) -> bool {
        match self.max_min_hold_periods {
            Some(max_hold) => opp.min_holding_periods <= max_hold,
            None => true,
        }
    }

    fn symbol_matches(&self, opp: &ArbitrageOpportunityDto) -> bool {
        self.symbols_in.is_empty()
            || self
                .symbols_in
                .iter()
                .any(|symbol| symbol.eq_ignore_ascii_case(&opp.symbol))
    }

    fn venues_match(&self, opp: &ArbitrageOpportunityDto) -> bool {
        self.venues_in.is_empty()
            || (self.has_venue(&opp.long_exchange) && self.has_venue(&opp.short_exchange))
    }

    fn has_venue(&self, venue: &str) -> bool {
        self.venues_in
            .iter()
            .any(|item| venue_names_equal(item, venue))
    }
}

fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use shared_types::{ArbitrageType, Recommendation, RiskLevel};

    fn opp() -> ArbitrageOpportunityDto {
        ArbitrageOpportunityDto {
            id: "o1".into(),
            symbol: "BTC".into(),
            arb_type: ArbitrageType::CrossExchange,
            type_label: "cross".into(),
            long_exchange: "binance".into(),
            short_exchange: "okx".into(),
            spread_8h: 0.001,
            long_rate_8h: -0.0005,
            short_rate_8h: 0.0005,
            long_rate: -0.0005,
            short_rate: 0.0005,
            single_yield: 0.001,
            net_single_yield: 0.0008,
            raw_single_yield: 0.001,
            settlement_interval: 8,
            risk_adjusted_yield: 0.0007,
            trading_cost_rate: 0.0002,
            min_holding_periods: 1,
            risk_level: RiskLevel::Low,
            volatility: 0.1,
            sharpe_ratio: 1.0,
            score: 90.0,
            score_breakdown: None,
            ranking_key: None,
            recommendation: Recommendation::StrongBuy,
            optimal_position: 1000.0,
            max_position: 2000.0,
            liquidity_score: 0.9,
            volume_24h: 10_000_000.0,
            long_volume_24h: 10_000_000.0,
            short_volume_24h: 12_000_000.0,
            data_source: "test".into(),
            confidence: 0.9,
            updated_at: Utc::now(),
            long_funding_interval: 8,
            short_funding_interval: 8,
            settlement_time_diff: false,
            strategy_description: String::new(),
            long_action: String::new(),
            short_action: String::new(),
            long_next_funding_time: 0,
            short_next_funding_time: 0,
            time_to_settlement_ms: 0,
            is_snipe_ready: false,
            long_price: Some(50_000.0),
            short_price: Some(50_010.0),
            long_leg_market_evidence: None,
            short_leg_market_evidence: None,
            quote_conversions: Vec::new(),
            price_deviation: None,
            basis_spread: None,
            basis_annual_cost: None,
            risk_warnings: Vec::new(),
            execution_eligible: true,
            execution_blockers: Vec::new(),
            execution_cost: None,
            index_composition: None,
            strategy_kind: None,
            strategy_category: None,
            spot_leg_mode: None,
            basis_bps: None,
            annualized_funding_bps: None,
            triangular_path: None,
            onchain_metadata: None,
            predicted_next_funding: None,
            funding_diff_window: None,
            funding_diff_windows: Vec::new(),
            borrow_cost_bps_per_day: None,
            funding_window_alignment_minutes: None,
            funding_cap_distance_bps: None,
            min_hold_hours: None,
            settlement_countdown_seconds: None,
        }
    }

    #[test]
    fn when_clause_matches_expected_opportunity() {
        let spec = StrategySpec {
            id: "s1".into(),
            when: WhenClause {
                min_net_yield: Some(0.0005),
                max_min_hold_periods: Some(1),
                venues_in: vec!["binance".into(), "okx".into()],
                symbols_in: vec!["BTC".into()],
            },
            size: SizeClause::FixedUsd { value: 1000.0 },
            hedge: HedgeMode::SimultaneousBoth,
            limits: StrategyLimits {
                max_open_hedges: 1,
                max_daily_loss_usd: 100.0,
                cooldown_secs: 300,
            },
            enabled: true,
        };
        assert!(spec.matches(&opp()));
    }

    #[test]
    fn venue_filter_matches_builder_venue_via_shared_normalizer() {
        let mut row = opp();
        row.long_exchange = "Hyperliquid:XYZ".into();
        row.short_exchange = "kucoin".into();

        let spec = StrategySpec {
            id: "s-builder".into(),
            when: WhenClause {
                min_net_yield: None,
                max_min_hold_periods: None,
                venues_in: vec!["hyperliquid:xyz".into(), "KuCoin".into()],
                symbols_in: Vec::new(),
            },
            size: SizeClause::FixedUsd { value: 1000.0 },
            hedge: HedgeMode::SimultaneousBoth,
            limits: StrategyLimits {
                max_open_hedges: 1,
                max_daily_loss_usd: 100.0,
                cooldown_secs: 300,
            },
            enabled: true,
        };

        assert!(spec.matches(&row));
    }
}
