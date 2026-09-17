fn apply_order_stream_probes(
    rows: &mut Vec<VenueOperationHealth>,
    credentials: &[VenueCredentialStatus],
    orders: &[OrderRecord],
    observed_at_ms: i64,
    ws_venues: &[ExchangeWsVenue],
) {
    let unresolved = unresolved_live_orders_by_venue(orders);
    for credential in credentials {
        let key = normalized_venue_name(&credential.venue);
        let Some(probe) = unresolved.get(&key) else {
            continue;
        };
        if !credential.live_write || !credentials_configured(credential) {
            continue;
        }
        if has_fresh_order_stream_row(rows, &key, probe.oldest_created_at_ms) {
            continue;
        }
        let warning = order_stream_probe_row(credential, probe, observed_at_ms, ws_venues);
        upsert_order_stream_probe_row(rows, &key, warning);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnresolvedOrderStreamProbe {
    count: u64,
    oldest_created_at_ms: i64,
}

fn unresolved_live_orders_by_venue(
    orders: &[OrderRecord],
) -> BTreeMap<String, UnresolvedOrderStreamProbe> {
    let mut rows = BTreeMap::new();
    for order in orders
        .iter()
        .filter(|order| order.intent.mode == ExecutionMode::Live)
        .filter(|order| unresolved_order_state(order.state))
    {
        let key = normalized_venue_name(&order.intent.exchange);
        rows.entry(key)
            .and_modify(|probe: &mut UnresolvedOrderStreamProbe| {
                probe.count = probe.count.saturating_add(1);
                probe.oldest_created_at_ms =
                    probe.oldest_created_at_ms.min(order.intent.created_at_ms);
            })
            .or_insert(UnresolvedOrderStreamProbe {
                count: 1,
                oldest_created_at_ms: order.intent.created_at_ms,
            });
    }
    rows
}

fn unresolved_order_state(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Created
            | LiveOrderState::RiskChecked
            | LiveOrderState::Submitted
            | LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::CancelRequested
            | LiveOrderState::Unknown
    )
}

fn has_fresh_order_stream_row(
    rows: &[VenueOperationHealth],
    venue_key: &str,
    oldest_order_created_at_ms: i64,
) -> bool {
    rows.iter().any(|row| {
        normalized_venue_name(&row.venue) == venue_key
            && row.operation == OP_PRIVATE_WS_ORDER_STREAM
            && row.source == SOURCE_PRIVATE_WS_RUNTIME
            && row.status == VenueOperationStatus::Ok
            && row.rows.unwrap_or(0) > 0
            && row.freshness_ms.is_some_and(is_fresh_order_stream_age)
            && row.observed_at_ms >= oldest_order_created_at_ms
    })
}

fn is_fresh_order_stream_age(freshness_ms: i64) -> bool {
    (0..=ORDER_STREAM_PROBE_RETRY_AFTER_MS as i64).contains(&freshness_ms)
}

fn upsert_order_stream_probe_row(
    rows: &mut Vec<VenueOperationHealth>,
    venue_key: &str,
    warning: VenueOperationHealth,
) {
    if let Some(row) = rows.iter_mut().find(|row| {
        normalized_venue_name(&row.venue) == venue_key
            && row.operation == OP_PRIVATE_WS_ORDER_STREAM
            && row.source == SOURCE_PRIVATE_WS_RUNTIME
            && !matches!(
                row.status,
                VenueOperationStatus::Blocked | VenueOperationStatus::Unsupported
            )
    }) {
        *row = warning;
    } else {
        rows.push(warning);
    }
}
