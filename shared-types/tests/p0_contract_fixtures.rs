use serde::de::DeserializeOwned;
use shared_types::contracts::account_health::{AccountDataHealth, VenueRuntimeHealth};
use shared_types::contracts::diagnostics::legacy::ExecutionMode;
use shared_types::contracts::execution::{
    CostEvidenceMeta, FeeEvidenceMeta, LegFinality, OrderFill,
};
use shared_types::contracts::market_data::MarketDataHealth;
use shared_types::contracts::p0::{
    OpportunityEnvelopeScope, OpportunityListEnvelope, OpportunityStreamEvent,
    P0OpportunityContract,
};
use shared_types::{
    ApiProblem, SpotLegMode, StrategyCategory, StrategyKind, DEFERRED_SPOT_PERP_TICKET_BLOCKER,
};

const REST_FIXTURE: &str = include_str!("../fixtures/p0_opportunity_list_v1.json");
const WS_FIXTURE: &str = include_str!("../fixtures/p0_opportunity_stream_v1.json");

#[test]
fn p0_rest_fixture_projects_into_four_stratified_contracts() -> Result<(), Box<ApiProblem>> {
    let envelope: OpportunityListEnvelope = decode_fixture(REST_FIXTURE)?;
    assert_eq!(envelope.scope, OpportunityEnvelopeScope::MainP0);
    assert_eq!(envelope.rows.len(), 1);

    let row = envelope.rows[0].clone();
    let contract = P0OpportunityContract::try_from(row.clone())?;
    assert_eq!(contract.core.strategy_kind, StrategyKind::PerpCross);
    assert_eq!(contract.core.strategy_category, StrategyCategory::Futures);
    assert!(contract.execution.eligible);
    assert!(contract.evidence.cost.verified);
    assert_eq!(
        shared_types::OpportunityListRow::from(contract.clone()),
        row
    );

    let value = encode_fixture(&contract)?;
    assert!(value.get("core").is_some());
    assert!(value.get("metrics").is_some());
    assert!(value.get("execution").is_some());
    assert!(value.get("evidence").is_some());
    Ok(())
}

#[test]
fn p0_ws_fixture_uses_the_same_fail_closed_row_contract() -> Result<(), Box<ApiProblem>> {
    let event: OpportunityStreamEvent = decode_fixture(WS_FIXTURE)?;
    assert_eq!(event.scope, OpportunityEnvelopeScope::MainP0);
    assert_eq!(event.changed_ids, vec!["p0-perp-cross-btc"]);
    assert_eq!(event.changed_rows.len(), 1);

    let contract = P0OpportunityContract::try_from(&event.changed_rows[0])?;
    assert_eq!(contract.core.id, event.changed_ids[0]);
    assert_eq!(contract.evidence.source, "market-data-cache");
    Ok(())
}

#[test]
fn p0_projection_rejects_non_p0_missing_category_and_false_execution_evidence(
) -> Result<(), Box<ApiProblem>> {
    let envelope: OpportunityListEnvelope = decode_fixture(REST_FIXTURE)?;
    let source = envelope.rows[0].clone();

    let mut non_p0 = source.clone();
    non_p0.strategy_kind = Some(StrategyKind::FundingCarry);
    assert_contract_rejected(non_p0, "allowlisted strategy kind");

    let mut missing_category = source.clone();
    missing_category.strategy_category = None;
    assert_contract_rejected(missing_category, "category is missing or mismatched");

    let mut false_execution_evidence = source;
    false_execution_evidence.cost.verified = false;
    assert_contract_rejected(
        false_execution_evidence,
        "requires allowed blockers, fresh legs and verified cost evidence",
    );
    Ok(())
}

#[test]
fn p0_projection_allows_only_ticket_bound_spot_perp_blockers_during_build(
) -> Result<(), Box<ApiProblem>> {
    let envelope: OpportunityListEnvelope = decode_fixture(REST_FIXTURE)?;
    let mut row = envelope.rows[0].clone();
    row.strategy_kind = Some(StrategyKind::SpotPerp);
    row.spot_leg_mode = Some(SpotLegMode::BuySpot);
    row.execution.blockers = vec![DEFERRED_SPOT_PERP_TICKET_BLOCKER.to_owned()];

    let contract = P0OpportunityContract::try_from(row.clone())?;
    assert!(contract.execution.eligible);

    row.execution
        .blockers
        .push("交易所标的身份未通过".to_owned());
    assert_contract_rejected(row, "requires allowed blockers");
    Ok(())
}

#[test]
fn contract_facades_keep_legacy_modes_outside_product_layers() {
    let _: Option<CostEvidenceMeta> = None;
    let _: Option<FeeEvidenceMeta> = None;
    let _: Option<OrderFill> = None;
    let _: Option<LegFinality> = None;
    let _: Option<MarketDataHealth> = None;
    let _: Option<AccountDataHealth> = None;
    let _: Option<VenueRuntimeHealth> = None;
    let _: Option<ExecutionMode> = None;
}

fn assert_contract_rejected(row: shared_types::OpportunityListRow, message: &str) {
    let result = P0OpportunityContract::try_from(row);
    assert!(
        result
            .as_ref()
            .is_err_and(|problem| problem.message.contains(message)),
        "expected typed P0 contract rejection containing {message}"
    );
}

fn decode_fixture<T: DeserializeOwned>(source: &str) -> Result<T, Box<ApiProblem>> {
    serde_json::from_str(source).map_err(|error| {
        Box::new(
            ApiProblem::new("P0_CONTRACT_FIXTURE_INVALID", error.to_string())
                .with_source("shared-types fixture"),
        )
    })
}

fn encode_fixture<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, Box<ApiProblem>> {
    serde_json::to_value(value).map_err(|error| {
        Box::new(
            ApiProblem::new("P0_CONTRACT_FIXTURE_ENCODE_FAILED", error.to_string())
                .with_source("shared-types fixture"),
        )
    })
}
