//! 资金费周期统计的测试夹具：验证生产 DTO 投影、窗口诊断和旧字段拒绝语义。

use shared_types::{ArbitrageOpportunityDto, FundingDiffWindowStats, FundingHistoryEvidence};

use super::view_model::FundingCycleStatsView;

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        ArbitrageType, FundingDiffSampleHealth, Recommendation, RiskLevel, StrategyCategory,
        StrategyKind,
    };

    #[test]
    fn missing_cycle_windows_ignore_legacy_90d_payload() -> Result<(), String> {
        let dto = dto_with_legacy_90d_payload(87)?;

        let stats = FundingCycleStatsView::from_dto(&dto);

        assert_eq!(stats.current_percentile, 0);
        assert_eq!(stats.sample_count, 0);
        assert!(stats.windows.is_empty());
        assert_eq!(stats.percentile_text(), "无周期样本");
        assert_eq!(stats.detail_text(), "无周期样本");
        assert_eq!(stats.trend_text(), "无周期样本");
        Ok(())
    }

    #[test]
    fn cycle_windows_drive_percentile_display() {
        let mut dto = dto_template();
        dto.funding_diff_windows = vec![
            funding_window(1, 8, 3.0, 76),
            funding_window(3, 24, 3.6, 84),
            funding_window(9, 72, 4.0, 92),
        ];

        let stats = FundingCycleStatsView::from_dto(&dto);

        assert_eq!(stats.current_percentile, 92);
        assert_eq!(stats.percentile_text(), "P92 · 9周期");
        assert!(stats.detail_text().contains("9周期/72h"));
        assert!(stats.detail_text().contains("来源 test"));
        assert!(stats.detail_text().contains("新鲜度 0ms"));
        assert!(stats.trend_text().contains("1周期 +0.030%"));
        assert!(stats.trend_text().contains("来源 test"));
        assert_eq!(
            stats.windows[2].distribution,
            "P50 +0.040% · P75 +0.050% · P90 +0.060% · P95 +0.060%"
        );
        assert!(stats.windows[2].evidence.contains("历史 funding-history"));
        assert!(stats.windows[2].evidence.contains("检查 72000ms"));
        assert!(stats.windows[2].evidence.contains("样本 9 · 健康"));
    }

    #[test]
    fn thin_cycle_window_surfaces_sample_health() {
        let mut dto = dto_template();
        let mut window = funding_window(9, 72, 4.0, 92);
        window.sample_count = 2;
        window.sample_health = FundingDiffSampleHealth::Thin;
        window.problem = Some("funding diff window has 2 samples for 9 cycles".into());
        dto.funding_diff_window = Some(window.clone());
        dto.funding_diff_windows = vec![window];

        let stats = FundingCycleStatsView::from_dto(&dto);

        assert_eq!(stats.percentile_text(), "样本不足 · 9周期");
        assert!(stats.detail_text().contains("样本不足"));
        assert!(stats.trend_text().contains("样本不足"));
    }

    #[test]
    fn diagnostic_problem_and_retry_are_not_reported_as_healthy() {
        let mut dto = dto_template();
        let mut window = funding_window(3, 24, 2.5, 81);
        window.source = "cache".into();
        window.problem = Some("rate limited; using degraded history".into());
        window.retry_after_ms = Some(30_000);
        dto.funding_diff_window = Some(window.clone());
        dto.funding_diff_windows = vec![window];

        let stats = FundingCycleStatsView::from_dto(&dto);

        assert_eq!(stats.percentile_text(), "诊断异常 · 3周期");
        assert!(stats.detail_text().contains("诊断异常"));
        assert!(stats.detail_text().contains("来源 cache"));
        assert!(stats.detail_text().contains("重试 30.0s"));
        assert!(stats.detail_text().contains("rate limited"));
        assert!(stats.trend_text().contains("诊断异常"));
        assert!(stats.trend_text().contains("重试 30.0s"));
    }

    fn funding_window(
        cycles: u32,
        window_hours: u32,
        mean_diff_bps: f64,
        current_percentile: u8,
    ) -> FundingDiffWindowStats {
        FundingDiffWindowStats {
            cycles,
            window_hours,
            sample_count: cycles as usize,
            mean_diff_bps,
            p50_diff_bps: mean_diff_bps,
            p75_diff_bps: mean_diff_bps + 1.0,
            p90_diff_bps: mean_diff_bps + 2.0,
            p95_diff_bps: mean_diff_bps + 2.0,
            stddev_diff_bps: 1.2,
            positive_ratio: 0.89,
            reversal_count: 1,
            current_percentile,
            source: "test".into(),
            freshness_ms: Some(0),
            sample_health: FundingDiffSampleHealth::Ok,
            problem: None,
            problem_detail: None,
            retry_after_ms: None,
            evidence: FundingHistoryEvidence {
                source: "funding-history".into(),
                observed_at_ms: 72_000,
                latest_at_ms: 71_000,
                freshness_ms: Some(1_000),
                sample_count: cycles as usize,
                sample_health: FundingDiffSampleHealth::Ok,
                problem: None,
                retry_after_ms: None,
            },
        }
    }

    fn dto_template() -> ArbitrageOpportunityDto {
        ArbitrageOpportunityDto {
            id: "funding-stats-test".into(),
            symbol: "MU".into(),
            arb_type: ArbitrageType::CrossExchange,
            type_label: "永续跨所".into(),
            long_exchange: "hyperliquid:km".into(),
            short_exchange: "kucoin".into(),
            spread_8h: 0.0,
            long_rate_8h: 0.0,
            short_rate_8h: 0.0,
            long_rate: 0.0,
            short_rate: 0.0,
            single_yield: 0.0,
            net_single_yield: 0.0,
            raw_single_yield: 0.0,
            settlement_interval: 8,
            risk_adjusted_yield: 0.0,
            trading_cost_rate: 0.0,
            min_holding_periods: 1,
            risk_level: RiskLevel::Low,
            volatility: 0.0,
            sharpe_ratio: 0.0,
            score: 0.0,
            score_breakdown: None,
            ranking_key: None,
            recommendation: Recommendation::Hold,
            optimal_position: 0.0,
            max_position: 0.0,
            liquidity_score: 0.0,
            volume_24h: 0.0,
            long_volume_24h: 0.0,
            short_volume_24h: 0.0,
            data_source: "test".into(),
            confidence: 0.0,
            updated_at: chrono::Utc::now(),
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
            long_price: None,
            short_price: None,
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
            strategy_kind: Some(StrategyKind::PerpCross),
            strategy_category: Some(StrategyCategory::Futures),
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

    fn dto_with_legacy_90d_payload(percentile: u8) -> Result<ArbitrageOpportunityDto, String> {
        let mut value = serde_json::to_value(dto_template()).map_err(|error| error.to_string())?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| "dto template did not serialize to an object".to_owned())?;
        object.insert("fundingHistoryPercentile90d".into(), percentile.into());
        serde_json::from_value(value).map_err(|error| error.to_string())
    }
}
