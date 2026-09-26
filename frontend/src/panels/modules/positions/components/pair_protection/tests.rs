use super::*;
use shared_types::{PositionPairEvidence, PositionPairEvidenceSource, PositionSide};

#[test]
fn protection_status_does_not_invent_runtime_or_freshness() {
    let config = AutoProfitCloseConfig {
        enabled: true,
        ..Default::default()
    };
    let coverage = PairCoverage {
        pair_count: 1,
        unpaired_count: 0,
    };
    let ready = SectionData::ready(Some(config.clone()));
    assert_eq!(protection_state(&ready, true, coverage), "规则已开启");
    assert_eq!(protection_state(&ready, false, coverage), "持仓待确认");
    let problem = shared_types::ApiProblem::new("TIMEOUT", "test timeout");
    assert_eq!(
        protection_state(&SectionData::stale(Some(config), &problem), true, coverage),
        "配置已过期"
    );
    assert_eq!(
        protection_state(&SectionData::error(&problem), true, coverage),
        "配置读取失败"
    );
}

#[test]
fn thresholds_explain_backend_and_or_trigger_rules() {
    let config = AutoProfitCloseConfig {
        min_net_profit_usd: 0.25,
        min_roi_bps: 10.0,
        max_net_loss_usd: 1.5,
        max_loss_roi_bps: 20.0,
        ..Default::default()
    };
    assert_eq!(
        take_profit_threshold(Some(&config)),
        "净收益 >= $0.25 且收益率 >= 0.1%"
    );
    assert_eq!(
        stop_loss_threshold(Some(&config)),
        "亏损 >= $1.5 或亏损率 >= 0.2%"
    );
}

#[test]
fn pair_risk_uses_the_closest_leg() {
    let rows = pair_rows(Some(12.0), Some(7.0));

    let risk = pair_liquidation_risk(&rows[0], &rows);

    assert_eq!(
        risk,
        Some(PairLiquidationRisk {
            distance_pct: 7.0,
            venue: "binance".to_owned(),
            evidence_complete: true,
        })
    );
}

#[test]
fn pair_risk_keeps_known_leg_visible_when_partner_is_missing() {
    let rows = pair_rows(Some(12.0), None);

    assert_eq!(
        pair_liquidation_risk(&rows[0], &rows),
        Some(PairLiquidationRisk {
            distance_pct: 12.0,
            venue: "okx".to_owned(),
            evidence_complete: false,
        })
    );
    assert!(current_pair_risk_label(&rows, pair_coverage(&rows)).contains("另一腿待确认"));
}

#[test]
fn pair_risk_ignores_unrelated_row_from_the_same_run() {
    let mut rows = pair_rows(Some(12.0), Some(7.0));
    rows.insert(1, position("bybit", "okx", PositionSide::Short, Some(1.0)));

    let risk = pair_liquidation_risk(&rows[0], &rows);

    assert_eq!(
        risk,
        Some(PairLiquidationRisk {
            distance_pct: 7.0,
            venue: "binance".to_owned(),
            evidence_complete: true,
        })
    );
}

#[test]
fn breached_distance_is_explained_as_crossed_not_negative_margin() {
    let rows = pair_rows(Some(-0.5), Some(7.0));

    assert_eq!(
        current_pair_risk_label(&rows, pair_coverage(&rows)),
        "已越过报告强平线 0.50% · okx 腿"
    );
}

#[test]
fn unpaired_position_does_not_claim_active_pair_protection() {
    let mut row = position("binance", "okx", PositionSide::Long, Some(12.0));
    row.pair_evidence = None;
    let rows = vec![row];
    let coverage = pair_coverage(&rows);

    assert_eq!(coverage.pair_count, 0);
    assert_eq!(coverage.unpaired_count, 1);
    assert_eq!(
        current_pair_risk_label(&rows, coverage),
        "当前无双边仓位 · 保护规则未生效"
    );
}

#[test]
fn protection_copy_distinguishes_account_samples_from_liquidation_updates() {
    let config = AutoProfitCloseConfig {
        confirmation_samples: 4,
        ..Default::default()
    };

    assert_eq!(
        protection_evidence_label(Some(&config)),
        "止盈/止损需连续 4 份双腿账户样本 · 强平保护按最新交易所距离"
    );
}

fn pair_rows(first: Option<f64>, second: Option<f64>) -> Vec<PositionRow> {
    vec![
        position("okx", "binance", PositionSide::Long, first),
        position("binance", "okx", PositionSide::Short, second),
    ]
}

fn position(
    venue: &str,
    partner_venue: &str,
    side: PositionSide,
    distance: Option<f64>,
) -> PositionRow {
    PositionRow {
        venue: venue.to_owned(),
        symbol: "BTCUSDT".to_owned(),
        origin: Default::default(),
        side,
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 100.0,
        leverage: 2.0,
        unrealized_pnl_usd: 0.0,
        liquidation_price: None,
        liquidation_distance_pct: distance,
        next_funding_ms: None,
        funding_rate_8h: 0.0,
        funding_rate_verified: true,
        maintenance_margin_ratio: 0.05,
        pair_evidence: Some(PositionPairEvidence {
            source: PositionPairEvidenceSource::ExecutionRun,
            run_id: "run-1".to_owned(),
            ticket_id: "ticket-1".to_owned(),
            opportunity_id: "opp-1".to_owned(),
            venue: venue.to_owned(),
            symbol: "BTCUSDT".to_owned(),
            side,
            partner_venue: partner_venue.to_owned(),
            partner_symbol: "BTCUSDT".to_owned(),
            partner_side: match side {
                PositionSide::Long => PositionSide::Short,
                PositionSide::Short => PositionSide::Long,
            },
            leg_filled_quantity: 1.0,
            partner_filled_quantity: 1.0,
            matched_notional_usd: 200.0,
            updated_at_ms: 1,
        }),
        paired_with: None,
        margin_usd: 50.0,
        severity: Default::default(),
        seconds_until_funding: None,
    }
}
