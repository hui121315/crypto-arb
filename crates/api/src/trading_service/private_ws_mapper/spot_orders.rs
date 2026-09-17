use super::*;

pub(crate) fn map_kraken_spot_execution(
    event: exchange::adapters::kraken::KrakenSpotExecution,
) -> Vec<PrivateWsEvent> {
    let mut events = Vec::with_capacity(2);
    if let Some(order) = event.order {
        events.push(PrivateWsEvent::Order(PrivateOrderDelta {
            client_order_id: order.client_order_id.clone().unwrap_or_default(),
            order,
            received_at_ms: event.received_at_ms,
        }));
    }
    if let Some(fill) = event.fill {
        events.push(PrivateWsEvent::Fill(PrivateFillDelta {
            venue: "kraken".into(),
            venue_event_id: format!("kraken_spot_trade:{}:{}", fill.order_id, fill.execution_id),
            exchange_order_id: fill.order_id,
            client_order_id: fill.client_order_id,
            symbol: fill.symbol,
            side: fill.side,
            quantity: fill.quantity,
            price: fill.price,
            fee_amount: fill.fee_amount,
            fee_currency: fill.fee_currency,
            occurred_at_ms: fill.occurred_at_ms,
        }));
    }
    events
}

pub(crate) fn map_gate_spot_orders(
    rows: Vec<gate_spot_ws_user::GateSpotOrderUpdate>,
) -> Vec<PrivateWsEvent> {
    rows.into_iter().flat_map(gate_spot_order).collect()
}

fn gate_spot_order(row: gate_spot_ws_user::GateSpotOrderUpdate) -> Vec<PrivateWsEvent> {
    let received_at_ms = row.received_at_ms;
    let client_order_id = row.client_order_id.trim().to_owned();
    if client_order_id.is_empty() && row.order.order_id.trim().is_empty() {
        return dirty_account(
            "gate",
            PrivateAccountScope::All,
            "spot_order_update_missing_identity",
        );
    }
    vec![PrivateWsEvent::Order(PrivateOrderDelta {
        client_order_id,
        order: row.order,
        received_at_ms,
    })]
}

pub(crate) fn map_kucoin_spot_event(event: kucoin_ws_user::KucoinUserEvent) -> Vec<PrivateWsEvent> {
    let kucoin_ws_user::KucoinUserEvent::Order(row) = event else {
        return dirty_account(
            "kucoin",
            PrivateAccountScope::All,
            "unexpected_spot_private_event",
        );
    };
    kucoin_spot_order(*row)
}

fn kucoin_spot_order(row: kucoin_ws_user::KucoinOrderUpdate) -> Vec<PrivateWsEvent> {
    let client_order_id = row.client_order_id.trim().to_owned();
    if client_order_id.is_empty() && row.order.order_id.trim().is_empty() {
        return dirty_account(
            "kucoin",
            PrivateAccountScope::All,
            "spot_order_update_missing_identity",
        );
    }
    let mut events = Vec::with_capacity(1 + usize::from(row.fill.is_some()));
    events.push(PrivateWsEvent::Order(PrivateOrderDelta {
        client_order_id,
        order: row.order,
        received_at_ms: row.event_time_ms,
    }));
    if let Some(fill) = row.fill {
        events.push(PrivateWsEvent::Fill(kucoin_spot_fill(fill)));
    }
    events
}

fn kucoin_spot_fill(row: kucoin_ws_user::KucoinFillUpdate) -> PrivateFillDelta {
    PrivateFillDelta {
        venue: "kucoin".to_owned(),
        exchange_order_id: row.exchange_order_id,
        client_order_id: row.client_order_id,
        symbol: non_empty_text(&row.symbol),
        side: Some(row.side),
        venue_event_id: row.venue_event_id,
        quantity: row.quantity,
        price: row.price,
        fee_amount: row.fee_amount,
        fee_currency: row.fee_currency,
        occurred_at_ms: row.occurred_at_ms,
    }
}
