use super::super::super::*;
use super::super::fixtures::*;

#[tokio::test]
async fn compensation_submit_runtime_requires_fresh_orderbook() {
    let state = test_state().await;
    let intent = compensation_order_record("close-1", LiveOrderState::Submitted).intent;

    let error = match validate_compensation_submit_runtime(&state, &intent).await {
        Ok(()) => panic!("missing orderbook must block compensation submit"),
        Err(error) => error,
    };

    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(
        error.code(),
        crate::services::market_data::MarketQuality::Missing.problem_code()
    );

    state.market_data().store_orderbook(
        orderbook("binance", "MUUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );

    assert!(validate_compensation_submit_runtime(&state, &intent)
        .await
        .is_ok());
}

#[tokio::test]
async fn generic_order_cannot_spoof_close_run_compensation_attempt() {
    let state = test_state().await;
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    record(&state, run);

    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-1", 1.0));
    let _ = project_ledger_event_update(
        &state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );
    let mut spoofed = compensation_order_record("close-1-spoof", LiveOrderState::Submitted);
    spoofed.intent.source = OrderSource::Manual;

    let updated = project_order_update(&state, &spoofed);

    assert!(updated.is_empty());
    let run = state
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("close run missing"));
    assert_eq!(run.status, CloseRunStatus::UnwindRequired);
    assert_eq!(
        run.unwind_plan
            .as_ref()
            .map(|plan| plan.compensation_attempts.len()),
        Some(0)
    );
}

#[tokio::test]
async fn funding_payment_with_pair_evidence_updates_close_run_cost() {
    let state = test_state().await;
    let mut leg = close_leg("order-1", CloseLegStatus::Submitted);
    leg.pair_evidence = Some(pair_evidence("run-1", "ticket-1"));
    let run = close_run("close-1", leg);
    record(&state, run);

    let ignored = project_ledger_event_update(
        &state,
        &ledger_funding_event(
            "open-order-1",
            Some("other-run"),
            Some("ticket-1"),
            Some(HedgeLegRole::Long),
            -0.25,
        ),
    );
    assert!(ignored.is_empty());

    let updated = project_ledger_event_update(
        &state,
        &ledger_funding_event(
            "open-order-1",
            Some("run-1"),
            Some("ticket-1"),
            Some(HedgeLegRole::Long),
            -0.25,
        ),
    );

    assert_eq!(updated.len(), 1);
    let cost = updated[0]
        .cost_reconciliation
        .as_ref()
        .unwrap_or_else(|| panic!("cost reconciliation missing"));
    assert_eq!(cost.funding_usd, Some(-0.25));
    assert_eq!(
        cost.funding_event_ids,
        vec!["funding:open-order-1:-0.25".to_owned()]
    );
    assert_eq!(cost.total_actual_cost_usd, Some(-0.25));

    let duplicate = project_ledger_event_update(
        &state,
        &ledger_funding_event(
            "open-order-1",
            Some("run-1"),
            Some("ticket-1"),
            Some(HedgeLegRole::Long),
            -0.25,
        ),
    );
    assert!(duplicate.is_empty());
}

#[tokio::test]
async fn funding_payment_without_run_context_does_not_update_close_run() {
    let state = test_state().await;
    let mut leg = close_leg("order-1", CloseLegStatus::Submitted);
    leg.pair_evidence = Some(pair_evidence("run-1", "ticket-1"));
    record(&state, close_run("close-1", leg));

    let updated = project_ledger_event_update(
        &state,
        &ledger_funding_event("open-order-1", None, None, None, -0.25),
    );

    assert!(updated.is_empty());
}

#[tokio::test]
async fn durable_close_projection_retry_does_not_double_incremental_fill() -> anyhow::Result<()> {
    let path = durable_close_path("retry");
    let config = durable_close_config(&path);
    let state = AppState::new(config.clone()).await?;
    record(
        &state,
        close_run(
            "durable-close",
            close_leg("order-1", CloseLegStatus::Submitted),
        ),
    );
    let event = ledger_fill_event("order-1", 0.4);

    let first = project_ledger_event_update(&state, &event);
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0].legs[0]
            .order
            .as_ref()
            .and_then(|order| order.filled_quantity),
        Some(0.4)
    );

    let restored = AppState::new(config).await?;
    let retry = project_ledger_event_update(&restored, &event);
    let replayed = restored
        .close_runs()
        .get("durable-close")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("replayed close run missing"))?;

    assert!(retry.is_empty());
    assert_eq!(
        replayed.legs[0]
            .order
            .as_ref()
            .and_then(|order| order.filled_quantity),
        Some(0.4)
    );
    let _ = std::fs::remove_file(path);
    Ok(())
}

fn pair_evidence(run_id: &str, ticket_id: &str) -> PositionPairEvidence {
    PositionPairEvidence {
        source: shared_types::PositionPairEvidenceSource::ExecutionRun,
        run_id: run_id.to_owned(),
        ticket_id: ticket_id.to_owned(),
        opportunity_id: "opp-1".to_owned(),
        venue: "binance".to_owned(),
        symbol: "MUUSDT".to_owned(),
        side: PositionSide::Long,
        partner_venue: "okx".to_owned(),
        partner_symbol: "MUUSDT".to_owned(),
        partner_side: PositionSide::Short,
        leg_filled_quantity: 1.0,
        partner_filled_quantity: 1.0,
        matched_notional_usd: 100.0,
        updated_at_ms: 1,
    }
}

fn durable_close_config(path: &std::path::Path) -> common::config::AppConfig {
    let mut config = test_config();
    config.storage.close_run_ledger_path = Some(path.display().to_string());
    config
}

fn durable_close_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-close-projection-{label}-{}-{}.jsonl",
        std::process::id(),
        common::time::now_ms()
    ))
}
