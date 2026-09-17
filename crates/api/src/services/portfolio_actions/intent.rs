use super::*;

pub(super) fn position_notional(row: &PositionRow) -> f64 {
    if row.quantity.is_finite() && row.mark_price.is_finite() {
        row.quantity.abs() * row.mark_price.max(0.0)
    } else {
        0.0
    }
}

pub(super) fn close_run_id(scope: CloseRunScope) -> String {
    let nonce = Uuid::new_v4().simple().to_string();
    let nonce = nonce.chars().take(12).collect::<String>();
    format!("close-{}-{nonce}", close_scope_key(scope))
}

fn close_scope_key(scope: CloseRunScope) -> &'static str {
    match scope {
        CloseRunScope::Single => "single",
        CloseRunScope::Pair => "pair",
        CloseRunScope::All => "all",
    }
}

#[cfg(test)]
pub(super) fn close_intent(
    row: &PositionRow,
    mode: ExecutionMode,
) -> Result<OrderIntent, AppError> {
    close_intent_with_key(row, mode, close_order_key(row))
}

pub(super) fn close_intent_for_context(
    row: &PositionRow,
    mode: ExecutionMode,
    context: &CloseRequestContext,
    leg_index: usize,
) -> Result<OrderIntent, AppError> {
    close_intent_with_key(
        row,
        mode,
        close_order_key_for_context(row, context, leg_index),
    )
}

pub(super) fn close_order_plan(intent: &OrderIntent) -> OrderCompilePlan {
    OrderCompilePlan {
        role: HedgeLegRole::Long,
        exchange: intent.exchange.clone(),
        symbol: intent.symbol.clone(),
        client_order_id_policy: intent.client_order_id_policy.clone().unwrap_or_default(),
        product: FeeProduct::Perp,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: intent.order_type,
        effective_order_type: intent.order_type,
        requested_time_in_force: intent.time_in_force,
        effective_time_in_force: intent.time_in_force,
        available_order_types: Vec::new(),
        available_time_in_force: Vec::new(),
        available_margin_modes: Vec::new(),
        venue_capability: VenueSymbolCapability::default(),
        market_order_style: None,
        venue_order_kind: VenueOrderKind::NativeMarket,
        payload_price_policy: OrderPayloadPricePolicy::Omit,
        reference_price: intent.price,
        protection_price: intent.price,
        payload_price: None,
        slippage_tolerance_bps: intent.slippage_tolerance_bps,
        summary: "CloseRun shared live-order preflight".to_owned(),
        blockers: Vec::new(),
    }
}

fn close_intent_with_key(
    row: &PositionRow,
    mode: ExecutionMode,
    key: String,
) -> Result<OrderIntent, AppError> {
    if !row.quantity.is_finite() || row.quantity <= 0.0 {
        return Err(AppError::BadRequest(format!(
            "position quantity is invalid: {}/{}",
            row.venue, row.symbol
        )));
    }
    if !row.mark_price.is_finite() || row.mark_price <= 0.0 {
        return Err(AppError::BadRequest(format!(
            "mark price is required before closing: {}/{}",
            row.venue, row.symbol
        )));
    }

    Ok(OrderIntent {
        id: key.clone(),
        source: OrderSource::Manual,
        strategy: None,
        mode,
        exchange: normalized_venue_name(&row.venue),
        symbol: row.symbol.clone(),
        side: close_side(row.side),
        order_type: OrderType::Market,
        quantity: row.quantity,
        price: Some(row.mark_price),
        slippage_tolerance_bps: None,
        reduce_only: true,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: valid_leverage(row.leverage),
        client_order_id: key,
        client_order_id_policy: None,
        created_at_ms: common::time::now_ms(),
    })
}

fn close_side(side: PositionSide) -> OrderSide {
    match side {
        PositionSide::Long => OrderSide::Sell,
        PositionSide::Short => OrderSide::Buy,
    }
}

pub(super) fn execution_mode(adapter_name: &str) -> ExecutionMode {
    if adapter_name == "mock" {
        ExecutionMode::DryRun
    } else if adapter_name.ends_with("_testnet") {
        ExecutionMode::Testnet
    } else {
        ExecutionMode::Live
    }
}

fn valid_leverage(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

fn close_order_key(row: &PositionRow) -> String {
    let nonce = Uuid::new_v4().simple().to_string();
    let nonce = nonce.chars().take(8).collect::<String>();
    format!(
        "pc-{}-{}-{}-{nonce}",
        compact_key(&row.venue, 8),
        compact_key(&row.symbol, 12),
        side_key(row.side),
    )
}

pub(super) fn close_order_key_for_context(
    row: &PositionRow,
    context: &CloseRequestContext,
    leg_index: usize,
) -> String {
    let Some(action_key) = context.idempotency_key.as_deref() else {
        return close_order_key(row);
    };
    let key_material = format!(
        "{}|{}|{}|{}|{leg_index}|{action_key}",
        close_scope_key(context.scope),
        normalized_venue_name(&row.venue),
        row.symbol.trim().to_ascii_uppercase(),
        side_key(row.side),
    );
    format!("pc-{}", stable_key_hash(&key_material))
}

fn stable_key_hash(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn compact_key(value: &str, max_len: usize) -> String {
    let key = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(max_len)
        .collect::<String>()
        .to_ascii_lowercase();
    if key.is_empty() {
        "x".into()
    } else {
        key
    }
}

fn side_key(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "l",
        PositionSide::Short => "s",
    }
}
