use super::*;

#[test]
fn missing_nav_component_never_renders_as_zero() {
    let evidence = PortfolioValueEvidence::default();

    assert_eq!(evidence_value(&evidence, false), "未知");
    assert!(evidence_detail(&evidence).starts_with("缺失"));
}

#[test]
fn invalid_nav_component_ignores_zero_placeholder() {
    let mut evidence = PortfolioValueEvidence::default();
    evidence.value_usd = Some(0.0);
    evidence.status = AccountFieldQualityStatus::Invalid;
    assert_eq!(evidence_value(&evidence, false), "未知");
    evidence.status = AccountFieldQualityStatus::Actual;
    assert_eq!(evidence_value(&evidence, false), "$0");
}

#[test]
fn missing_pnl_evidence_does_not_claim_actual_fields() {
    let evidence = PortfolioPnlEvidence::default();
    assert!(pnl_evidence_label(&evidence).contains("账本数据依据待确认"));
    assert!(!pnl_evidence_label(&evidence).contains("字段实际"));
}

#[test]
fn pnl_label_keeps_missing_fields_and_ledger_source() {
    let evidence = PortfolioPnlEvidence {
        source: "trading_sql_realized_window+execution_ledger+close_runs".to_owned(),
        realized_group_count: 2,
        close_run_count: 1,
        missing_fields: vec![ReviewPnlField::Fee, ReviewPnlField::Funding],
        ..PortfolioPnlEvidence::default()
    };

    let label = pnl_evidence_label(&evidence);

    assert!(label.contains("缺失 费用/资金费"));
    assert!(label.contains("SQL 账本"));
    assert!(label.contains("1 次平仓"));
}
