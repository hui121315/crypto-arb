use super::*;

#[test]
fn missing_nav_component_never_renders_as_zero() {
    let evidence = PortfolioValueEvidence::default();

    assert_eq!(evidence_value(&evidence, false), "未知");
    assert!(evidence_detail(&evidence).starts_with("缺失"));
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
