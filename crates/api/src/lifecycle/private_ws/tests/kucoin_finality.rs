use super::projection_support::*;
use crate::trading_service::private_ws_mapper::map_kucoin_event;
use exchange::adapters::kucoin_ws_user;
use serde_json::{Map, Value};
use shared_types::{ExecutionLedgerEventType, HedgeLegRole, OrderSide};

#[tokio::test]
async fn kucoin_contract_count_ws_events_wait_for_authoritative_order_query() -> anyhow::Result<()>
{
    let state = isolated_private_ws_state().await?;
    let run = private_ws_unwind_run();
    state.execution_runs().insert(run.run_id.clone(), run);
    let mut intent = private_ws_intent("order-kucoin-finality", OrderSide::Buy, true);
    intent.exchange = "kucoin".to_owned();
    let record = state
        .trading_service()
        .submit_unwind_with_ledger_context(
            intent,
            trading::ExecutionLedgerOrderContext::new(
                "run-private-ws".to_owned(),
                "ticket-private-ws".to_owned(),
                HedgeLegRole::Long,
            ),
        )
        .await?;
    let baseline_filled_quantity = record.filled_quantity;
    let exchange_order_id = record
        .exchange_order_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("KuCoin test order missing exchange identity"))?;
    let event_time_ms = common::time::now_ms().saturating_add(1_000);

    let first_events = kucoin_finality_events(exchange_order_id, event_time_ms)?;
    super::super::apply::apply_events(&state, "kucoin", first_events).await;

    let projected = state
        .execution_runs()
        .get("run-private-ws")
        .ok_or_else(|| anyhow::anyhow!("ExecutionRun missing after KuCoin WS event"))?;
    let first_state = projected.state;
    let first_cost = projected.cost_reconciliation.clone();
    let first_updated_at_ms = projected.updated_at_ms;
    drop(projected);
    assert!(kucoin_fill_ledger_events(&state).is_empty());
    let journal = state
        .trading_service()
        .get_order("order-kucoin-finality")
        .ok_or_else(|| anyhow::anyhow!("KuCoin journal order missing"))?;
    assert_eq!(journal.state, shared_types::LiveOrderState::Filled);
    assert_eq!(journal.filled_quantity, baseline_filled_quantity);

    let duplicate_events = kucoin_finality_events(exchange_order_id, event_time_ms)?;
    super::super::apply::apply_events(&state, "kucoin", duplicate_events).await;

    let duplicate = state
        .execution_runs()
        .get("run-private-ws")
        .ok_or_else(|| anyhow::anyhow!("ExecutionRun missing after KuCoin duplicate"))?;
    assert_eq!(duplicate.state, first_state);
    assert_eq!(duplicate.cost_reconciliation, first_cost);
    assert_eq!(duplicate.updated_at_ms, first_updated_at_ms);
    assert!(kucoin_fill_ledger_events(&state).is_empty());
    Ok(())
}

fn kucoin_finality_events(
    exchange_order_id: &str,
    event_time_ms: i64,
) -> anyhow::Result<Vec<crate::trading_service::private_ws_events::PrivateWsEvent>> {
    let match_event = kucoin_fixture_event(
        include_str!("../../../../../exchange/fixtures/kucoin/classic_ws_trade_orders_match.json"),
        exchange_order_id,
        event_time_ms,
    )?;
    let filled_event = kucoin_fixture_event(
        include_str!("../../../../../exchange/fixtures/kucoin/classic_ws_trade_orders_filled.json"),
        exchange_order_id,
        event_time_ms.saturating_add(1),
    )?;
    let mut events = map_kucoin_event(match_event);
    events.extend(map_kucoin_event(filled_event));
    Ok(events)
}

fn kucoin_fixture_event(
    fixture: &str,
    exchange_order_id: &str,
    event_time_ms: i64,
) -> anyhow::Result<kucoin_ws_user::KucoinUserEvent> {
    let mut envelope: Value = serde_json::from_str(fixture)?;
    let data = envelope
        .get_mut("data")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("KuCoin fixture data object missing"))?;
    apply_order_identity(data, exchange_order_id, event_time_ms);
    let text = serde_json::to_string(&envelope)?;
    kucoin_ws_user::parse_user_event(&text)?
        .ok_or_else(|| anyhow::anyhow!("KuCoin fixture did not produce an event"))
}

fn apply_order_identity(data: &mut Map<String, Value>, order_id: &str, event_time_ms: i64) {
    data.insert("orderId".to_owned(), Value::String(order_id.to_owned()));
    data.remove("clientOid");
    let event_time_ns = event_time_ms.saturating_mul(1_000_000);
    data.insert("orderTime".to_owned(), Value::from(event_time_ns));
    data.insert("ts".to_owned(), Value::from(event_time_ns));
}

fn kucoin_fill_ledger_events(
    state: &crate::state::AppState,
) -> Vec<shared_types::ExecutionLedgerEvent> {
    state
        .trading_service()
        .list_execution_ledger_events()
        .into_iter()
        .filter(|event| event.event_type == ExecutionLedgerEventType::FillEvent)
        .collect()
}
