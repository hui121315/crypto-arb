use super::*;

#[test]
fn preview_rejects_non_executable_opportunity() {
    let opportunity = blocked_opportunity();
    let detail = validate_executable_opportunity(&opportunity)
        .map(|_| String::new())
        .unwrap_or_else(|error| error.to_string());

    assert!(detail.contains("资金费差为 0"));
}

#[test]
fn preview_rejects_raw_eligible_without_verified_fee_evidence() -> anyhow::Result<()> {
    let opportunity = raw_eligible_without_verified_fee_opportunity();

    assert!(!opportunity.execution_eligible);
    assert!(opportunity
        .execution_blockers
        .iter()
        .any(|blocker| blocker.contains("缺少双腿新鲜官方费率证据")));
    assert!(!shared_types::is_hedge_preview_ready(&opportunity));
    let error = match validate_executable_opportunity(&opportunity) {
        Ok(()) => anyhow::bail!("preview must reject incomplete shared readiness"),
        Err(error) => error,
    };

    assert_eq!(error.code(), codes::OPPORTUNITY_NOT_EXECUTABLE);
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert!(error.to_string().contains("缺少双腿新鲜官方费率证据"));
    Ok(())
}
