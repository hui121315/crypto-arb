use shared_types::{
    PnlBreakdown, PortfolioNavBreakdown, PortfolioNavEvidence, PortfolioPnlEvidence,
    PortfolioSummary, PositionRow, PositionSide,
};

#[derive(Debug)]
pub struct SummaryInputs<'a> {
    pub positions: &'a [PositionRow],
    pub total_nav_usd: f64,
    pub nav_evidence: PortfolioNavEvidence,
    pub nav_breakdown: PortfolioNavBreakdown,
    pub nav_yesterday_usd: Option<f64>,
    pub realized_pnl_today_usd: f64,
    pub funding_today_usd: f64,
    pub fee_rebate_today_usd: f64,
    pub pnl_evidence: PortfolioPnlEvidence,
    pub now_ms: i64,
}

pub fn compute_summary(inputs: &SummaryInputs<'_>) -> PortfolioSummary {
    let mut net_delta = 0.0;
    let mut naked_exposure = 0.0;
    let mut naked_count = 0;

    for row in inputs.positions {
        let notional = row.quantity.abs() * row.mark_price;
        net_delta += match row.side {
            PositionSide::Long => notional,
            PositionSide::Short => -notional,
        };
        if row.pair_evidence.is_none() {
            naked_exposure += notional;
            naked_count += 1;
        }
    }

    let mut nav_evidence = inputs.nav_evidence.clone();
    nav_evidence.breakdown = inputs.nav_breakdown.clone();
    PortfolioSummary {
        total_nav_usd: inputs.total_nav_usd,
        nav_evidence,
        nav_change_24h_pct: inputs.nav_yesterday_usd.and_then(|nav_yesterday_usd| {
            optional_pct(inputs.total_nav_usd - nav_yesterday_usd, nav_yesterday_usd)
        }),
        net_delta_usd: net_delta,
        net_delta_pct_of_nav: pct(net_delta, inputs.total_nav_usd),
        naked_exposure_usd: naked_exposure,
        naked_position_count: naked_count,
        realized_pnl_today_usd: inputs.realized_pnl_today_usd,
        pnl_breakdown: PnlBreakdown {
            funding_usd: inputs.funding_today_usd,
            price_usd: inputs.realized_pnl_today_usd
                - inputs.funding_today_usd
                - inputs.fee_rebate_today_usd,
            fee_rebate_usd: inputs.fee_rebate_today_usd,
            evidence: inputs.pnl_evidence.clone(),
        },
        updated_at_ms: inputs.now_ms,
    }
}

fn pct(n: f64, d: f64) -> f64 {
    if d.abs() > f64::EPSILON {
        n / d * 100.0
    } else {
        0.0
    }
}

fn optional_pct(n: f64, d: f64) -> Option<f64> {
    (n.is_finite() && d.is_finite() && d.abs() > f64::EPSILON).then(|| n / d * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_nav_delta_and_naked_exposure_in_one_pass() {
        let rows = vec![
            row(PositionSide::Long, 2.0, Some("B@BTC")),
            row(PositionSide::Short, 1.0, None),
        ];

        let summary = compute_summary(&SummaryInputs {
            positions: &rows,
            total_nav_usd: 102.0,
            nav_evidence: PortfolioNavEvidence::default(),
            nav_breakdown: PortfolioNavBreakdown::default(),
            nav_yesterday_usd: Some(100.0),
            realized_pnl_today_usd: 10.0,
            funding_today_usd: 3.0,
            fee_rebate_today_usd: 2.0,
            pnl_evidence: PortfolioPnlEvidence::default(),
            now_ms: 7,
        });

        assert_eq!(summary.total_nav_usd, 102.0);
        assert_eq!(summary.nav_change_24h_pct, Some(2.0));
        assert_eq!(summary.net_delta_usd, 100.0);
        assert_eq!(summary.naked_exposure_usd, 100.0);
        assert_eq!(summary.pnl_breakdown.price_usd, 5.0);
    }

    #[test]
    fn legacy_pair_string_without_evidence_stays_naked() {
        let mut row = row(PositionSide::Long, 2.0, Some("B@BTC"));
        row.pair_evidence = None;

        let summary = compute_summary(&SummaryInputs {
            positions: &[row],
            total_nav_usd: 51.0,
            nav_evidence: PortfolioNavEvidence::default(),
            nav_breakdown: PortfolioNavBreakdown::default(),
            nav_yesterday_usd: Some(100.0),
            realized_pnl_today_usd: 0.0,
            funding_today_usd: 0.0,
            fee_rebate_today_usd: 0.0,
            pnl_evidence: PortfolioPnlEvidence::default(),
            now_ms: 7,
        });

        assert_eq!(summary.naked_position_count, 1);
        assert_eq!(summary.naked_exposure_usd, 200.0);
    }

    #[test]
    fn nav_change_distinguishes_missing_baseline_from_true_zero() {
        let mut inputs = SummaryInputs {
            positions: &[],
            total_nav_usd: 100.0,
            nav_evidence: PortfolioNavEvidence::default(),
            nav_breakdown: PortfolioNavBreakdown::default(),
            nav_yesterday_usd: None,
            realized_pnl_today_usd: 0.0,
            funding_today_usd: 0.0,
            fee_rebate_today_usd: 0.0,
            pnl_evidence: PortfolioPnlEvidence::default(),
            now_ms: 7,
        };

        assert_eq!(compute_summary(&inputs).nav_change_24h_pct, None);

        inputs.nav_yesterday_usd = Some(100.0);
        assert_eq!(compute_summary(&inputs).nav_change_24h_pct, Some(0.0));
    }

    fn row(side: PositionSide, qty: f64, paired_with: Option<&str>) -> PositionRow {
        let pair_evidence = paired_with.map(|pair| shared_types::PositionPairEvidence {
            source: shared_types::PositionPairEvidenceSource::ExecutionRun,
            run_id: "run-1".into(),
            ticket_id: "ticket-1".into(),
            opportunity_id: "opp-1".into(),
            venue: "OKX".into(),
            symbol: "BTC".into(),
            side,
            partner_venue: pair
                .split_once('@')
                .map(|(venue, _)| venue)
                .unwrap_or(pair)
                .into(),
            partner_symbol: pair
                .split_once('@')
                .map(|(_, symbol)| symbol)
                .unwrap_or("BTC")
                .into(),
            partner_side: match side {
                PositionSide::Long => PositionSide::Short,
                PositionSide::Short => PositionSide::Long,
            },
            leg_filled_quantity: qty,
            partner_filled_quantity: qty,
            matched_notional_usd: qty * 100.0,
            updated_at_ms: 7,
        });
        PositionRow {
            venue: "OKX".into(),
            symbol: "BTC".into(),
            origin: Default::default(),
            side,
            quantity: qty,
            entry_price: 100.0,
            mark_price: 100.0,
            leverage: 2.0,
            unrealized_pnl_usd: 1.0,
            liquidation_price: None,
            liquidation_distance_pct: None,
            next_funding_ms: None,
            funding_rate_8h: 0.0,
            funding_rate_verified: true,
            maintenance_margin_ratio: 0.05,
            pair_evidence,
            paired_with: paired_with.map(str::to_owned),
            margin_usd: 50.0,
            severity: shared_types::PositionSeverity::Ok,
            seconds_until_funding: None,
        }
    }
}
