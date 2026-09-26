//! Synthetic incidents over the real order journal; never loads account credentials.
use crate::{services::close_runs, state::AppState};
use serde_json::json;
use shared_types::{CloseRun, LiveOrderState, OrderAck, OrderIntent, OrderRecord};

pub(super) fn seed(state: &AppState) -> anyhow::Result<()> {
    let now = common::time::now_ms();
    let journal = state.trading_service().mock_order_journal();
    let intent: OrderIntent = serde_json::from_value(json!({
        "id": "paper-partial-compensation", "clientOrderId": "paper-partial-compensation",
        "source": "close_run_compensation", "mode": "dry_run", "exchange": "binance",
        "symbol": "BTC", "side": "buy", "orderType": "limit",
        "quantity": 1.0, "price": 100.0, "createdAtMs": now
    }))?;
    journal.insert_created(intent.clone(), now);
    for status in [LiveOrderState::RiskChecked, LiveOrderState::Submitted] {
        anyhow::ensure!(journal
            .update_state(&intent.id, status, None, now)
            .is_some());
    }
    let mut ack: OrderAck = serde_json::from_value(json!({
        "internalOrderId": intent.id, "clientOrderId": intent.client_order_id,
        "exchangeOrderId": "paper-partial-venue-order", "state": "accepted",
        "acceptedAtMs": now
    }))?;
    anyhow::ensure!(journal.apply_ack(&ack).is_some());
    ack.state = LiveOrderState::PartiallyFilled;
    ack.filled_quantity = Some(0.4);
    ack.filled_price = Some(100.0);
    let order = journal
        .apply_ack(&ack)
        .ok_or_else(|| anyhow::anyhow!("partial fill not recorded"))?;
    record(
        state,
        "paper-close-partial",
        &order,
        "partially_filled",
        now,
    )?;
    for (id, quantity) in [
        ("paper-close-zero", Some(0.0)),
        ("paper-close-unknown", None),
    ] {
        let mut terminal = distinct_order(&order, format!("{id}-order"));
        terminal.state = LiveOrderState::Cancelled;
        terminal.filled_quantity = quantity;
        terminal.filled_price = None;
        record(state, id, &terminal, "cancelled", now)?;
    }
    Ok(())
}

fn distinct_order(order: &OrderRecord, id: String) -> OrderRecord {
    let mut distinct = order.clone();
    distinct.intent.id = id.clone();
    distinct.intent.client_order_id = id.clone();
    distinct.exchange_order_id = Some(format!("exchange-{id}"));
    distinct.identity = Default::default();
    distinct.identity = distinct.identity_snapshot();
    distinct
}

fn record(
    state: &AppState,
    id: &str,
    order: &OrderRecord,
    status: &str,
    now: i64,
) -> anyhow::Result<()> {
    let mut closed = distinct_order(order, format!("{id}-closed-leg"));
    closed.intent.source = shared_types::OrderSource::Manual;
    closed.intent.side = shared_types::OrderSide::Sell;
    closed.state = LiveOrderState::Filled;
    closed.filled_quantity = Some(1.0);
    let mut failed = distinct_order(&closed, format!("{id}-failed-leg"));
    failed.intent.exchange = "okx".into();
    failed.intent.side = shared_types::OrderSide::Buy;
    failed.state = LiveOrderState::Cancelled;
    failed.filled_quantity = Some(0.0);
    let run: CloseRun = serde_json::from_value(json!({
        "id": id, "scope": "pair", "status": "compensation_submitted",
        "snapshotVersion": "paper-partial-snapshot", "expectedLegCount": 2,
        "submittedOrderCount": 2, "failedLegCount": 1, "nakedExposureUsd": 100.0,
        "message": "isolated compensation incident", "startedAtMs": now, "updatedAtMs": now,
        "legs": [
            {"venue": "binance", "symbol": "BTC", "side": "long", "status": "filled",
             "quantity": 1.0, "markPrice": 100.0, "notionalUsd": 100.0, "order": closed,
             "confirmedFilledAtMs": now, "finalitySource": "order_query"},
            {"venue": "okx", "symbol": "BTC", "side": "short", "status": "cancelled",
             "quantity": 1.0, "markPrice": 100.0, "notionalUsd": 100.0, "order": failed,
             "finalitySource": "order_query"}
        ],
        "unwindPlan": {
            "status": "compensation_submitted",
            "filledLegs": [], "failedLegs": [], "remainingPositions": [],
            "compensationCandidates": [], "nextActions": [], "requiredEvidence": [],
            "compensationAttempts": [{
                "venue": "binance", "symbol": "BTC", "side": "long",
                "compensationOrderSide": "buy", "targetQuantity": 1.0, "status": status,
                "order": order, "submittedAtMs": now, "updatedAtMs": now
            }]
        }
    }))?;
    close_runs::record(state, run);
    Ok(())
}
