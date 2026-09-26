use crate::state::AppState;

pub(super) fn seed_settlement_reviews(state: &AppState) -> anyhow::Result<()> {
    let now = common::time::now_ms();
    let run: shared_types::OnchainExecutionSubmitResponse = serde_json::from_value(serde_json::json!({
        "runId":"paper-onchain-review", "buildId":"paper-build-review", "status":"completed",
        "legs":[{"position":1,"kind":"primary_cex","status":"filled","venue":"fixture","symbol":"SOL/USDC",
            "orderId":"paper-order-review", "transactionId":null, "filledQuantity":1, "message":"synthetic receipt"}],
        "estimatedNetProfitUsd":5,"remainingExposureUsd":0,"quantityReconciled":false,
        "message":"synthetic completed legs; fees missing", "startedAtMs":now,"updatedAtMs":now,
        "accounting":{"status":"pending_receipts","flows":[],"netAssets":[],"usdValue":null,"problems":["CEX 实际手续费待确认"]}
    }))?;
    state.onchain_execution_runs().insert(run.run_id.clone(), run);
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../../../../shared-types/fixtures/stocks_restock.json"))?;
    let plan = serde_json::from_value(fixture["before"]["plans"][0].clone())?;
    let peers: serde_json::Value = serde_json::from_str(include_str!("../../../../../../../shared-types/fixtures/stocks_peer_settlement.json"))?;
    let peer = serde_json::from_value(peers["peerPlans"][0].clone())?;
    state.backpack_stocks().seed_review_records(plan, peer);
    Ok(())
}
