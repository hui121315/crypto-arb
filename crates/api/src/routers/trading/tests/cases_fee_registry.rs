use super::super::*;

#[test]
fn fee_schedule_registry_failure_maps_to_typed_service_unavailable() {
    let error = fee_schedule_registry_error(
        &arbitrage::algorithms::fee_evidence::FeeScheduleRegistryError::Decode(
            "invalid fixture".to_owned(),
        ),
    );

    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.code(), codes::FEE_SCHEDULE_REGISTRY_UNAVAILABLE);
}
