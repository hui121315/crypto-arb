use super::*;

/// Dispatcher 已声明会推 cache 的 venue 列表（mapper 真正会写入
/// `PrivateBalancesSnapshot.venue` 字段的 venue 字符串）。`list_configured_balances`
/// 用这个集合判断 cache fresh / fetch 路径覆盖。
pub(super) fn dispatcher_venues_for(credentials: &AdapterCredentials) -> Vec<String> {
    let mut venues = Vec::new();
    if credentials.binance_live.is_some() {
        push_dispatcher_venue(&mut venues, VenueId::Binance);
    }
    if credentials.bybit_live.is_some() {
        push_dispatcher_venue(&mut venues, VenueId::Bybit);
    }
    if credentials.bitget_live.is_some() {
        push_dispatcher_venue(&mut venues, VenueId::Bitget);
    }
    if credentials.gate_live.is_some() {
        push_dispatcher_venue(&mut venues, VenueId::Gate);
    }
    if credentials.gate_crossex_live.is_some() {
        push_dispatcher_venue(&mut venues, VenueId::GateCrossEx);
    }
    if credentials.kucoin_live.is_some() {
        push_dispatcher_venue(&mut venues, VenueId::Kucoin);
    }
    if credentials
        .kraken_live
        .as_ref()
        .is_some_and(KrakenAdapterCredentials::is_configured)
    {
        push_dispatcher_venue(&mut venues, VenueId::Kraken);
    }
    if credentials.okx_live.is_some() {
        push_dispatcher_venue(&mut venues, VenueId::Okx);
    }
    if credentials.hyperliquid_live.is_some() {
        push_dispatcher_venue(&mut venues, VenueId::Hyperliquid);
    }
    venues
}

pub(super) fn push_dispatcher_venue(venues: &mut Vec<String>, venue: VenueId) {
    venues.push(venue.as_str().to_owned());
}

pub(super) fn is_hyperliquid_cache_dispatcher_venue(venue: &str) -> bool {
    venue == "hyperliquid:spot" || is_hyperliquid_builder_venue(venue)
}

/// 实盘订单 mutation 前的审计链路 fail-closed 闸门：仅当 `Live` 模式且审计已配置却不可写时拒单。
pub(super) fn ensure_live_order_mutation_audit_trail(mode: ExecutionMode) -> TradingResult<()> {
    let snapshot = audit::health_snapshot(common::time::now_ms());
    ensure_live_order_mutation_audit_snapshot(mode, &snapshot)
}

pub(super) fn ensure_live_order_mutation_audit_snapshot(
    mode: ExecutionMode,
    snapshot: &audit::AuditSinkHealthSnapshot,
) -> TradingResult<()> {
    if mode != ExecutionMode::Live {
        return Ok(());
    }
    match audit::live_order_mutation_block_reason(snapshot) {
        Some(reason) => Err(TradingError::AuditLogUnavailable { reason }),
        None => Ok(()),
    }
}

pub(super) fn ensure_remote_cancel_audit_trail(record: &OrderRecord) -> TradingResult<()> {
    if !cancel_requires_remote_mutation(record.state) {
        return Ok(());
    }
    ensure_live_order_mutation_audit_trail(record.intent.mode)
}

#[cfg(test)]
pub(super) fn ensure_remote_cancel_audit_snapshot(
    record: &OrderRecord,
    snapshot: &audit::AuditSinkHealthSnapshot,
) -> TradingResult<()> {
    if !cancel_requires_remote_mutation(record.state) {
        return Ok(());
    }
    ensure_live_order_mutation_audit_snapshot(record.intent.mode, snapshot)
}

pub(super) fn cancel_requires_remote_mutation(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Submitted
            | LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::Unknown
    )
}

pub(crate) fn live_venue_capabilities(
    credentials: &AdapterCredentials,
) -> Vec<shared_types::TradingVenueCapability> {
    live_adapters::capability_rows_from_credentials(credentials)
}

pub(super) fn scoped_balance_venues(venues: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut scoped = Vec::with_capacity(venues.len());
    for venue in venues {
        let normalized = normalized_venue_name(venue);
        if !normalized.is_empty() && seen.insert(normalized.clone()) {
            scoped.push(normalized);
        }
    }
    scoped
}

pub(super) fn refresh_order_query_client_ids(record: &OrderRecord) -> Vec<String> {
    let identity = record.identity_snapshot();
    let mut ids = Vec::with_capacity(2);
    push_unique_order_query_id(&mut ids, identity.venue_client_order_id.as_deref());
    push_unique_order_query_id(&mut ids, Some(identity.public_client_order_id.as_str()));
    push_unique_order_query_id(&mut ids, Some(record.intent.client_order_id.as_str()));
    ids
}

pub(super) fn push_unique_order_query_id(ids: &mut Vec<String>, id: Option<&str>) {
    let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
        return;
    };
    if !ids.iter().any(|existing| existing == id) {
        ids.push(id.to_owned());
    }
}

pub(super) fn balance_fetch_backoff_ms(error: &exchange::ExchangeError) -> u64 {
    error
        .retry_after_ms()
        .unwrap_or(BALANCE_FETCH_ERROR_BACKOFF_MS)
}

pub(super) fn latest_balance_replay_events(
    events: &[SqlBalanceLedgerReplayEvent],
) -> HashMap<(String, String), SqlBalanceLedgerReplayEvent> {
    let mut latest = HashMap::new();
    for event in events {
        let venue = normalized_venue_name(&event.row.venue);
        let currency = event.row.currency.trim().to_ascii_uppercase();
        if venue.is_empty() || currency.is_empty() {
            continue;
        }
        let key = (venue, currency);
        if latest_balance_should_replace(latest.get(&key), event) {
            latest.insert(key, event.clone());
        }
    }
    latest
}

pub(super) fn latest_balance_should_replace(
    current: Option<&SqlBalanceLedgerReplayEvent>,
    next: &SqlBalanceLedgerReplayEvent,
) -> bool {
    let Some(current) = current else {
        return true;
    };
    (next.observed_at_ms, next.captured_at_ms) >= (current.observed_at_ms, current.captured_at_ms)
}

#[derive(Debug, Default)]
pub(crate) struct ReconcileOutcome {
    pub(crate) diffs: Vec<trading::ReconcileDiff>,
    pub(crate) refreshed: Vec<OrderRecord>,
    pub(crate) refresh_failures: Vec<ReconcileRefreshFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReconcileRefreshFailure {
    pub(crate) internal_order_id: String,
    pub(crate) venue: String,
    pub(crate) error: String,
}

pub(super) fn repairable_reconcile_internal_ids(diffs: &[trading::ReconcileDiff]) -> Vec<String> {
    diffs
        .iter()
        .filter(|diff| {
            matches!(
                diff.kind,
                trading::ReconcileDiffKind::RemoteMissing
                    | trading::ReconcileDiffKind::StateMismatch
            )
        })
        .filter_map(|diff| diff.internal_order_id.clone())
        .collect()
}

pub(super) fn filled_quantity_changes_account_state(filled_quantity: Option<f64>) -> bool {
    filled_quantity.is_some_and(|quantity| quantity.is_finite() && quantity > 0.0)
}

pub(super) fn order_info_changes_account_state(order: &shared_types::OrderInfo) -> bool {
    order.filled_quantity.is_finite() && order.filled_quantity > 0.0
}

impl AdapterCredentials {
    pub(crate) fn available_count(&self) -> usize {
        [
            self.binance_live.is_some(),
            self.bitget_live.is_some(),
            self.bybit_live.is_some(),
            self.gate_live.is_some(),
            self.gate_crossex_live.is_some(),
            self.hyperliquid_live.is_some(),
            self.kucoin_live.is_some(),
            self.kraken_live
                .as_ref()
                .is_some_and(KrakenAdapterCredentials::is_configured),
            self.okx_live.is_some(),
        ]
        .into_iter()
        .filter(|available| *available)
        .count()
    }
}
