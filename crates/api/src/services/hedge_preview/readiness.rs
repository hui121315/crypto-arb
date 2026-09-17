use crate::services::opportunity;
use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    problem::codes, venue_names_equal, ArbitrageOpportunityDto, HedgeLegRole,
    OpportunityLegMarketEvidence,
};

pub(crate) fn validate_executable_opportunity(
    opportunity: &ArbitrageOpportunityDto,
) -> Result<(), AppError> {
    let mut blockers = leg_market_contract_blockers(opportunity);
    if !opportunity::is_ticket_build_ready(opportunity) {
        for blocker in execution_readiness_blockers(opportunity) {
            if !blockers.contains(&blocker) {
                blockers.push(blocker);
            }
        }
    }
    if blockers.is_empty() {
        return Ok(());
    }
    let detail = blockers.join("; ");
    Err(AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::OPPORTUNITY_NOT_EXECUTABLE,
        detail,
    )
    .with_details(serde_json::json!({ "blockers": blockers })))
}

fn leg_market_contract_blockers(opportunity: &ArbitrageOpportunityDto) -> Vec<String> {
    [
        (
            HedgeLegRole::Long,
            opportunity.long_exchange.as_str(),
            opportunity.long_leg_market_evidence.as_ref(),
        ),
        (
            HedgeLegRole::Short,
            opportunity.short_exchange.as_str(),
            opportunity.short_leg_market_evidence.as_ref(),
        ),
    ]
    .into_iter()
    .flat_map(|(role, venue, evidence)| leg_market_contract_blockers_for(role, venue, evidence))
    .collect()
}

fn leg_market_contract_blockers_for(
    role: HedgeLegRole,
    expected_venue: &str,
    evidence: Option<&OpportunityLegMarketEvidence>,
) -> Vec<String> {
    let label = match role {
        HedgeLegRole::Long => "long",
        HedgeLegRole::Short => "short",
    };
    let Some(evidence) = evidence else {
        return vec![format!("{label} leg market evidence is missing")];
    };
    let mut blockers = Vec::new();
    if !venue_names_equal(&evidence.venue, expected_venue) {
        blockers.push(format!(
            "{label} leg market evidence venue {} does not match {}",
            evidence.venue, expected_venue
        ));
    }
    if evidence.symbol.trim().is_empty() {
        blockers.push(format!("{label} leg market evidence symbol is missing"));
    }
    if !evidence
        .price
        .is_some_and(|price| price.is_finite() && price > 0.0)
    {
        blockers.push(format!("{label} leg market evidence price is missing"));
    }
    blockers
}

fn execution_readiness_blockers(opportunity: &ArbitrageOpportunityDto) -> Vec<String> {
    if !opportunity.execution_blockers.is_empty() {
        return opportunity.execution_blockers.clone();
    }
    vec!["需要 P0 策略、Fresh 双腿行情证据和 verified round-trip fee evidence".to_owned()]
}
