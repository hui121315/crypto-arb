use super::super::projection_support::*;
use crate::trading_service::private_ws_events::{
    PrivateBalancesSnapshot, PrivatePositionsSnapshot, PrivateWsEvent,
};
use crate::trading_service::AdapterCredentials;
use shared_types::{OrderSide, PositionInfo, VenueBalanceInfo};

#[tokio::test]
async fn private_ws_event_bus_projects_order_position_and_balance_batch() -> anyhow::Result<()> {
    let state = isolated_private_ws_state().await?;
    let mut intent = private_ws_intent("event-bus-order", OrderSide::Buy, false);
    intent.exchange = "okx".into();
    state.trading_service().update_risk_config(|config| {
        config.allowed_exchanges.insert("okx".to_owned());
    });
    let record = state.trading_service().submit(intent).await?;
    let mut orders = state.ws_hub().subscribe(realtime::channels::ORDERS);
    let position = private_ws_position();
    let balance = private_ws_balance();

    super::super::super::apply::apply_events(
        &state,
        "okx",
        vec![
            private_order_filled_event(&record, record.updated_at_ms.saturating_add(1))?,
            PrivateWsEvent::Positions(PrivatePositionsSnapshot {
                venue: "okx".into(),
                rows: vec![position.clone()],
            }),
            PrivateWsEvent::Balances(Box::new(PrivateBalancesSnapshot {
                venue: "okx".into(),
                rows: vec![balance.clone()],
            })),
        ],
    )
    .await;

    let message = tokio::time::timeout(std::time::Duration::from_secs(1), orders.recv()).await??;
    let Some(message) = message.payload_json() else {
        return Err(anyhow::anyhow!("private order event was not JSON"));
    };
    assert_eq!(message["event"], "private_ws_order_update");
    let orders = state.trading_service().list_orders();
    assert!(
        orders.iter().any(|order| {
            order.intent.id == "event-bus-order"
                && order.state == shared_types::LiveOrderState::Filled
                && order.last_update_source == shared_types::OrderUpdateSource::PrivateWs
        }),
        "orders={orders:?}"
    );
    assert_eq!(state.trading_service().list_positions().await?, [position]);
    assert_eq!(
        state
            .trading_service()
            .list_configured_balances(AdapterCredentials {
                okx_live: Some(("key".into(), "secret".into(), "passphrase".into())),
                ..AdapterCredentials::default()
            })
            .await?,
        [balance]
    );
    Ok(())
}

fn private_ws_position() -> PositionInfo {
    PositionInfo {
        symbol: "BTC".into(),
        exchange: "okx".into(),
        side: "long".into(),
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 101.0,
        unrealized_pnl: 1.0,
        leverage: 1.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 100.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

fn private_ws_balance() -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: "okx".into(),
        currency: "USDT".into(),
        total: 100.0,
        available: 90.0,
        frozen: 10.0,
        unrealized_pnl: 1.0,
    }
}
