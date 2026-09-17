use super::*;
use shared_types::{
    ExecutionFillConfidence, ExecutionLedgerEventType, ExecutionLedgerOrderRef,
    ExecutionLedgerPayload, ExecutionLedgerQuality, ExecutionRun, ExecutionRunLeg,
    ExecutionRunState, FeeLedgerSnapshot, FillLedgerSnapshot, HedgeLegRole, LiveOrderState,
    OrderSide, OrderUpdateSource, VenueOrderIdentity,
};

const DATABASE_URL_ENV: &str = "CROSSLINE_TEST_POSTGRES_URL";

#[test]
fn projection_retry_backoff_is_bounded() {
    assert_eq!(projection_retry_delay_ms(1), 500);
    assert_eq!(projection_retry_delay_ms(2), 1_000);
    assert_eq!(projection_retry_delay_ms(8), 60_000);
    assert_eq!(projection_retry_delay_ms(i32::MAX), 60_000);
}

#[tokio::test]
async fn unconfigured_projection_worker_is_reported_as_disabled() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let mut tasks = BackgroundTasks::new(state.task_registry().clone());

    spawn_worker(&state, &mut tasks);

    let snapshot = state
        .task_registry()
        .task_snapshots(common::time::now_ms())
        .into_iter()
        .find(|snapshot| snapshot.name == "ledger_projection_jobs")
        .ok_or_else(|| anyhow::anyhow!("disabled projection task health missing"))?;
    assert!(!snapshot.enabled);
    assert!(!snapshot.running);
    assert!(snapshot.issue.is_none());
    assert!(tasks.shutdown(Duration::from_millis(50)).await);
    Ok(())
}

#[tokio::test]
#[ignore = "requires CROSSLINE_TEST_POSTGRES_URL"]
async fn startup_projection_worker_catches_pending_fill_once() -> anyhow::Result<()> {
    let database_url = std::env::var(DATABASE_URL_ENV)
        .map_err(|_| anyhow::anyhow!("{DATABASE_URL_ENV} must be configured"))?;
    let seed = unique_id("startup-catchup");
    let execution_path = temp_path(&seed, "execution");
    let close_path = temp_path(&seed, "close");
    let state = AppState::new(test_config(&database_url, &execution_path, &close_path)).await?;
    let event = fill_event(&seed);
    let run = execution_run(&seed, &event.order.identity);
    state.execution_runs().insert(run.run_id.clone(), run);

    state
        .trading_service()
        .persist_ledger_event_group_durable(std::slice::from_ref(&event))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        state
            .execution_runs()
            .get(&format!("run-{seed}"))
            .and_then(|run| run.long_leg.filled_quantity),
        None
    );

    let mut tasks = BackgroundTasks::new(state.task_registry().clone());
    spawn_worker(&state, &mut tasks);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if state
                .execution_runs()
                .get(&format!("run-{seed}"))
                .and_then(|run| run.long_leg.filled_quantity)
                == Some(1.0)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("startup projection worker did not catch up"))?;

    assert!(state
        .execution_run_store()
        .has_projection_receipt(trading::EXECUTION_RUN_PROJECTOR, &event.event_id));
    let verification_jobs = state
        .trading_service()
        .claim_sql_projection_jobs(
            trading::EXECUTION_RUN_PROJECTOR,
            common::time::now_ms().saturating_add(PROJECTION_JOB_LEASE_MS),
            256,
            PROJECTION_JOB_LEASE_MS,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(verification_jobs
        .iter()
        .all(|job| job.event_id != event.event_id));
    for job in verification_jobs {
        state
            .trading_service()
            .retry_sql_projection_job(&job, common::time::now_ms(), "test claim released")
            .await
            .map_err(anyhow::Error::msg)?;
    }
    let projected = state
        .execution_runs()
        .get(&format!("run-{seed}"))
        .map(|run| run.value().clone())
        .ok_or_else(|| anyhow::anyhow!("projected run missing"))?;
    tokio::time::sleep(WORKER_INTERVAL.saturating_mul(2)).await;
    assert_eq!(
        state
            .execution_runs()
            .get(&format!("run-{seed}"))
            .map(|run| run.value().clone()),
        Some(projected)
    );

    assert!(tasks.shutdown(Duration::from_secs(2)).await);
    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .map_err(anyhow::Error::msg)?;
    let _ = std::fs::remove_file(execution_path);
    let _ = std::fs::remove_file(close_path);
    Ok(())
}

fn test_config(
    database_url: &str,
    execution_path: &std::path::Path,
    close_path: &std::path::Path,
) -> common::config::AppConfig {
    let mut config = common::config::AppConfig::default();
    config.storage.postgres_url = Some(database_url.to_owned());
    config.storage.portfolio_nav_path = None;
    config.storage.execution_ledger_path = None;
    config.storage.order_snapshot_path = None;
    config.storage.execution_run_ledger_path = Some(execution_path.display().to_string());
    config.storage.close_run_ledger_path = Some(close_path.display().to_string());
    config
}

fn fill_event(seed: &str) -> ExecutionLedgerEvent {
    let identity = VenueOrderIdentity {
        internal_order_id: format!("order-{seed}"),
        public_client_order_id: format!("client-{seed}"),
        venue_client_order_id: None,
        exchange_order_id: Some(format!("exchange-{seed}")),
        product: shared_types::FeeProduct::Perp,
        client_order_id_policy: None,
        transport_metadata: Default::default(),
    };
    ExecutionLedgerEvent {
        event_id: format!("fill-{seed}"),
        event_type: ExecutionLedgerEventType::FillEvent,
        source: OrderUpdateSource::PrivateWs,
        order: ExecutionLedgerOrderRef {
            run_id: Some(format!("run-{seed}")),
            ticket_id: Some(format!("ticket-{seed}")),
            leg_role: Some(HedgeLegRole::Long),
            reduce_only: Some(false),
            exchange: "postgres-test".to_owned(),
            symbol: "BTC-USDC".to_owned(),
            side: OrderSide::Buy,
            identity,
        },
        payload: ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
            quantity: 1.0,
            average_price: 101.0,
            quote_value: 101.0,
            quality: ExecutionLedgerQuality::Actual,
            confidence: ExecutionFillConfidence::VenueFill,
            fee: Some(FeeLedgerSnapshot {
                amount: 0.1,
                currency: Some("USDC".to_owned()),
                quality: ExecutionLedgerQuality::Actual,
            }),
        }),
        occurred_at_ms: common::time::now_ms(),
        captured_at_ms: common::time::now_ms(),
    }
}

fn execution_run(seed: &str, identity: &VenueOrderIdentity) -> ExecutionRun {
    ExecutionRun {
        run_id: format!("run-{seed}"),
        ticket_id: format!("ticket-{seed}"),
        opportunity_id: format!("opportunity-{seed}"),
        state: ExecutionRunState::SubmittingFirstLeg,
        long_leg: run_leg(HedgeLegRole::Long, Some(identity.clone())),
        short_leg: run_leg(HedgeLegRole::Short, None),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "pending fill".to_owned(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn run_leg(role: HedgeLegRole, identity: Option<VenueOrderIdentity>) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "postgres-test".to_owned(),
        symbol: "BTC-USDC".to_owned(),
        order_ids: identity
            .as_ref()
            .map(|identity| vec![identity.internal_order_id.clone()])
            .unwrap_or_default(),
        identity,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Accepted,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 101.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}

fn unique_id(prefix: &str) -> String {
    format!("{prefix}-{}-{}", std::process::id(), common::time::now_ms())
}

fn temp_path(seed: &str, kind: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("crossline-{kind}-{seed}.jsonl"))
}
