use shared_types::{ApiProblem, ListStatus, VenueBalanceEnvelope, VenuePositionEnvelope};

fn assert_empty_envelope_explainability(value: &serde_json::Value, expected_problem: &str) {
    assert_eq!(value["rows"], serde_json::json!([]));
    assert_eq!(value["rowCount"], 0);
    assert_eq!(value["problems"][0]["code"], expected_problem);
    assert_eq!(value["operationHealth"], serde_json::json!([]));
    assert_eq!(value["fieldQuality"], serde_json::json!([]));
    assert_eq!(value["rowHealth"], serde_json::json!([]));
}

#[test]
fn empty_balance_and_position_envelopes_keep_explainability_fields() -> serde_json::Result<()> {
    let balance = VenueBalanceEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        "account_balance_runtime",
        42,
        vec![ApiProblem::new(
            "BALANCE_EVIDENCE_MISSING",
            "no balance rows have a runtime explanation",
        )],
        Vec::new(),
    );
    let position = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        "account_position_runtime",
        42,
        vec![ApiProblem::new(
            "POSITION_EVIDENCE_MISSING",
            "no position rows have a runtime explanation",
        )],
        Vec::new(),
    );

    assert_empty_envelope_explainability(
        &serde_json::to_value(balance)?,
        "BALANCE_EVIDENCE_MISSING",
    );
    assert_empty_envelope_explainability(
        &serde_json::to_value(position)?,
        "POSITION_EVIDENCE_MISSING",
    );

    Ok(())
}
