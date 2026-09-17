use crate::error::{TradingError, TradingResult};
use crate::journal::OrderJournal;
use crate::risk::RiskEngine;
use exchange::{ExchangeError, LiveTradingAdapter};
use parking_lot::RwLock;
use shared_types::{
    CancelOrderRequest, ExecutionMode, LiveOrderState, OrderAck, OrderIntent, OrderRecord,
    OrderSubmissionContext, OrderType, VenueBalanceInfo, VenueId,
};
use std::fmt;
use std::sync::Arc;

enum SubmitRisk {
    Standard,
    Unwind,
}

struct MarginNeed {
    exchange: String,
    exchange_key: String,
    currency: Option<&'static str>,
    required: f64,
    available: f64,
}

struct SubmitCancellationGuard {
    journal: Arc<OrderJournal>,
    internal_order_id: String,
    armed: bool,
}

impl SubmitCancellationGuard {
    fn new(journal: Arc<OrderJournal>, internal_order_id: String) -> Self {
        Self {
            journal,
            internal_order_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for SubmitCancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.journal.update_state(
            &self.internal_order_id,
            LiveOrderState::Unknown,
            Some(
                "order submission was interrupted before an adapter result; query by order identity before retry"
                    .to_owned(),
            ),
            common::time::now_ms(),
        );
    }
}

pub struct ExecutionEngine {
    adapter: RwLock<Arc<dyn LiveTradingAdapter>>,
    risk: RiskEngine,
    journal: Arc<OrderJournal>,
}

impl fmt::Debug for ExecutionEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionEngine")
            .field("risk", &self.risk)
            .field("journal", &self.journal)
            .finish_non_exhaustive()
    }
}

impl ExecutionEngine {
    pub fn new(
        adapter: Arc<dyn LiveTradingAdapter>,
        risk: RiskEngine,
        journal: Arc<OrderJournal>,
    ) -> Self {
        Self {
            adapter: RwLock::new(adapter),
            risk,
            journal,
        }
    }

    pub fn set_adapter(&self, adapter: Arc<dyn LiveTradingAdapter>) {
        *self.adapter.write() = adapter;
    }

    pub fn adapter(&self) -> Arc<dyn LiveTradingAdapter> {
        Arc::clone(&self.adapter.read())
    }

    pub fn journal(&self) -> &Arc<OrderJournal> {
        &self.journal
    }

    pub async fn submit(&self, intent: OrderIntent) -> TradingResult<OrderRecord> {
        self.submit_with_context(intent, OrderSubmissionContext::default())
            .await
    }

    pub async fn submit_with_context(
        &self,
        intent: OrderIntent,
        context: OrderSubmissionContext,
    ) -> TradingResult<OrderRecord> {
        self.submit_with_risk(intent, SubmitRisk::Standard, context)
            .await
    }

    pub async fn submit_unwind(&self, intent: OrderIntent) -> TradingResult<OrderRecord> {
        self.submit_with_risk(
            intent,
            SubmitRisk::Unwind,
            OrderSubmissionContext::default(),
        )
        .await
    }

    async fn submit_with_risk(
        &self,
        intent: OrderIntent,
        risk_mode: SubmitRisk,
        context: OrderSubmissionContext,
    ) -> TradingResult<OrderRecord> {
        // 原子占位（entry 锁）：并发/重放的同 client_order_id 不可能双双通过判重
        // 各自走完风控与 place_order——此前 get + insert 两步之间无原子性。
        let created_at = common::time::now_ms();
        match self
            .journal
            .claim_created_with_product(intent.clone(), context.product, created_at)
        {
            crate::journal::CreatedClaim::New(_) => {}
            crate::journal::CreatedClaim::Existing(existing) => return Ok(existing),
            crate::journal::CreatedClaim::Pending => {
                return Err(TradingError::SubmissionInFlight(
                    intent.client_order_id.clone(),
                ));
            }
        }

        let other_open_orders = open_orders_excluding_self(&self.journal);
        let risk = match risk_mode {
            SubmitRisk::Standard => self.risk.check_order(&intent, other_open_orders),
            SubmitRisk::Unwind => self.risk.check_unwind(&intent),
        };
        self.journal
            .mark_risk_checked(&intent.id, risk.clone(), common::time::now_ms())
            .ok_or_else(|| TradingError::OrderNotFound(intent.id.clone()))?;

        if !risk.allowed {
            return Err(TradingError::RiskBlocked(risk.reasons));
        }

        let adapter = self.adapter();
        if let Err(err) = ensure_sufficient_margin(adapter.as_ref(), &intent).await {
            self.journal.update_state(
                &intent.id,
                LiveOrderState::Failed,
                Some(err.to_string()),
                common::time::now_ms(),
            );
            return Err(err);
        }

        self.journal
            .mark_submitted(&intent.id, common::time::now_ms())
            .ok_or_else(|| TradingError::OrderNotFound(intent.id.clone()))?;
        let mut cancellation_guard =
            SubmitCancellationGuard::new(Arc::clone(&self.journal), intent.id.clone());
        let outcome = match adapter.place_order_with_context(&intent, &context).await {
            Ok(ack) => self
                .journal
                .apply_ack(&ack)
                .ok_or_else(|| TradingError::OrderNotFound(intent.id.clone())),
            Err(e) => {
                let state = place_error_state(&e);
                self.journal.update_state(
                    &intent.id,
                    state,
                    Some(e.to_string()),
                    common::time::now_ms(),
                );
                Err(TradingError::Exchange(e))
            }
        };
        cancellation_guard.disarm();
        outcome
    }

    pub async fn cancel(&self, internal_order_id: &str) -> TradingResult<OrderRecord> {
        let record = self
            .journal
            .get(internal_order_id)
            .ok_or_else(|| TradingError::OrderNotFound(internal_order_id.to_owned()))?;
        if !requires_adapter_cancel(record.state) {
            return Ok(record);
        }
        let request = CancelOrderRequest {
            exchange: record.intent.exchange.clone(),
            symbol: record.intent.symbol.clone(),
            internal_order_id: record.intent.id.clone(),
            exchange_order_id: record.exchange_order_id.clone(),
            client_order_id: record.intent.client_order_id.clone(),
        };
        let adapter = self.adapter();
        let context = OrderSubmissionContext {
            product: record.identity_snapshot().product,
            ..OrderSubmissionContext::default()
        };
        let ack: OrderAck = adapter
            .cancel_order_with_context(&request, &context)
            .await?;
        let updated = if cancel_ack_is_final(&record, &ack) {
            self.journal.apply_ack(&ack)
        } else {
            self.journal.apply_cancel_request_ack(&ack)
        };
        // Private finality can beat the adapter ACK. Return that newer state instead of
        // misreporting a terminal order as missing or regressing it to cancel-requested.
        updated
            .or_else(|| {
                self.journal
                    .get(internal_order_id)
                    .filter(|current| !requires_adapter_cancel(current.state))
            })
            .ok_or_else(|| TradingError::OrderNotFound(internal_order_id.to_owned()))
    }
}

fn place_error_state(error: &ExchangeError) -> LiveOrderState {
    if is_ambiguous_write_result(error) {
        LiveOrderState::Unknown
    } else {
        LiveOrderState::Failed
    }
}

fn is_ambiguous_write_result(error: &ExchangeError) -> bool {
    matches!(
        error,
        ExchangeError::Timeout { .. }
            | ExchangeError::Network(_)
            | ExchangeError::RateLimited { .. }
            | ExchangeError::Parse(_)
            | ExchangeError::WsClosed(_)
            | ExchangeError::Http {
                status: 500..=599,
                ..
            }
    )
}

fn open_orders_excluding_self(journal: &OrderJournal) -> usize {
    journal.open_order_count().saturating_sub(1)
}

fn requires_adapter_cancel(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Submitted
            | LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::Unknown
    )
}

fn cancel_ack_is_final(record: &OrderRecord, ack: &OrderAck) -> bool {
    matches!(record.intent.mode, ExecutionMode::DryRun)
        || !cancel_ack_only_confirms_request(ack.state)
}

fn cancel_ack_only_confirms_request(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Accepted | LiveOrderState::CancelRequested | LiveOrderState::Cancelled
    )
}

async fn ensure_sufficient_margin(
    adapter: &dyn LiveTradingAdapter,
    intent: &OrderIntent,
) -> TradingResult<()> {
    if !requires_margin_check(intent) {
        return Ok(());
    }

    let balances = adapter
        .get_exchange_balances(&intent.exchange, None)
        .await?;
    ensure_sufficient_margin_for_intents(&balances, &[intent])
}

pub fn ensure_sufficient_margin_for_intents(
    balances: &[VenueBalanceInfo],
    intents: &[&OrderIntent],
) -> TradingResult<()> {
    let mut needs = Vec::with_capacity(intents.len());
    for intent in intents
        .iter()
        .copied()
        .filter(|intent| requires_margin_check(intent))
    {
        push_margin_need(&mut needs, balances, intent);
    }
    for need in needs {
        if need.available + f64::EPSILON < need.required {
            return Err(TradingError::InsufficientMargin {
                exchange: need.exchange,
                required: need.required,
                available: need.available,
            });
        }
    }
    Ok(())
}

fn push_margin_need(
    needs: &mut Vec<MarginNeed>,
    balances: &[VenueBalanceInfo],
    intent: &OrderIntent,
) {
    let required = required_margin(intent);
    let (currency, available) = selected_margin_balance(balances, intent);
    let exchange_key = shared_types::normalized_venue_name(&intent.exchange);
    if let Some(existing) = needs
        .iter_mut()
        .find(|need| need.exchange_key == exchange_key && need.currency == currency)
    {
        existing.required += required;
        return;
    }
    needs.push(MarginNeed {
        exchange: intent.exchange.clone(),
        exchange_key,
        currency,
        required,
        available,
    });
}

fn selected_margin_balance(
    balances: &[VenueBalanceInfo],
    intent: &OrderIntent,
) -> (Option<&'static str>, f64) {
    for currency in margin_currency_candidates(&intent.symbol, &intent.exchange) {
        if let Some(available) = balance_available(balances, currency, &intent.exchange) {
            return (Some(currency), available);
        }
    }
    (None, 0.0)
}

pub fn selected_margin_currency_for_intent(
    balances: &[VenueBalanceInfo],
    intent: &OrderIntent,
) -> Option<&'static str> {
    selected_margin_balance(balances, intent).0
}

fn requires_margin_check(intent: &OrderIntent) -> bool {
    matches!(intent.mode, ExecutionMode::Live | ExecutionMode::Testnet) && !intent.reduce_only
}

fn required_margin(intent: &OrderIntent) -> f64 {
    let quantity = intent.quantity.abs();
    let price = intent.price.unwrap_or(0.0);
    let leverage = if intent.leverage.is_finite() && intent.leverage > 0.0 {
        intent.leverage
    } else {
        1.0
    };
    let buffer = match intent.order_type {
        OrderType::Market => 1.01,
        OrderType::Limit | OrderType::PostOnly => 1.0,
    };
    if !quantity.is_finite() || !price.is_finite() || price <= 0.0 {
        return f64::INFINITY;
    }
    quantity * price * buffer / leverage
}

#[cfg(test)]
fn available_margin(balances: &[VenueBalanceInfo], intent: &OrderIntent) -> f64 {
    selected_margin_balance(balances, intent).1
}

fn balance_available(balances: &[VenueBalanceInfo], currency: &str, exchange: &str) -> Option<f64> {
    balances
        .iter()
        .filter(|row| is_margin_balance(row, exchange))
        .filter(|row| row.currency.eq_ignore_ascii_case(currency))
        .map(|row| row.available.max(0.0))
        .max_by(f64::total_cmp)
}

fn is_margin_balance(row: &VenueBalanceInfo, exchange: &str) -> bool {
    margin_balance_venue_matches(&row.venue, exchange)
}

/// Matches balance evidence to the execution venue without allowing one
/// Hyperliquid DEX's balance to stand in for another DEX's account.
pub fn margin_balance_venue_matches(row_venue: &str, exchange: &str) -> bool {
    let row_venue = shared_types::normalized_venue_name(row_venue);
    let exchange = shared_types::normalized_venue_name(exchange);
    if row_venue.is_empty() || exchange.is_empty() {
        return false;
    }
    let row_is_hyperliquid = VenueId::from_exchange_name(&row_venue) == Some(VenueId::Hyperliquid);
    let exchange_is_hyperliquid =
        VenueId::from_exchange_name(&exchange) == Some(VenueId::Hyperliquid);
    if row_is_hyperliquid && exchange_is_hyperliquid {
        return row_venue == exchange || row_venue.ends_with(":spot");
    }
    if row_venue.ends_with(":spot") {
        return false;
    }
    match (
        VenueId::from_exchange_name(&row_venue),
        VenueId::from_exchange_name(&exchange),
    ) {
        (Some(row), Some(target)) => row == target,
        _ => shared_types::venue_family(&row_venue) == shared_types::venue_family(&exchange),
    }
}

fn margin_currency_candidates(symbol: &str, exchange: &str) -> Vec<&'static str> {
    let upper = symbol.to_ascii_uppercase();
    let mut candidates = Vec::with_capacity(6);
    for &currency in quote_priority(exchange) {
        if upper.ends_with(currency) || upper.contains(&format!("-{currency}")) {
            candidates.push(currency);
        }
    }
    for &currency in quote_priority(exchange) {
        if !candidates.contains(&currency) {
            candidates.push(currency);
        }
    }
    candidates
}

fn quote_priority(exchange: &str) -> &'static [&'static str] {
    if VenueId::from_exchange_name(exchange) != Some(VenueId::Hyperliquid) {
        return &["USDT", "USDC", "USDH", "USDT0", "USD"];
    }
    let normalized = shared_types::normalized_venue_name(exchange);
    match normalized.split_once(':').map(|(_, dex)| dex) {
        Some("cash") => &["USDT0", "USDT", "USDC", "USDH", "USD"],
        Some("km" | "flx" | "vntl") => &["USDH", "USDC", "USDT0", "USDT", "USD"],
        _ => &["USDC", "USDT0", "USDT", "USDH", "USD"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MockLiveAdapter, RiskConfig};
    use exchange::{ExchangeCapabilities, ExchangeError, ExchangeResult};
    use shared_types::{ExecutionMode, OrderSide, OrderSource, OrderType, TimeInForce};
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn submit_unwind_allows_reduce_only_market_without_price() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(MockLiveAdapter::new());
        let engine = ExecutionEngine::new(
            adapter,
            RiskEngine::new(RiskConfig::default()),
            Arc::new(OrderJournal::new()),
        );
        let mut intent = order_intent();
        intent.price = None;

        let record = engine.submit_unwind(intent).await.expect("unwind accepted");

        assert_eq!(record.state, LiveOrderState::Filled);
        assert!(record.risk.as_ref().is_some_and(|risk| risk.allowed));
    }

    #[tokio::test]
    async fn submit_unwind_rejects_non_reduce_only_market() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(MockLiveAdapter::new());
        let engine = ExecutionEngine::new(
            adapter,
            RiskEngine::new(RiskConfig::default()),
            Arc::new(OrderJournal::new()),
        );
        let mut intent = order_intent();
        intent.reduce_only = false;

        let error = engine.submit_unwind(intent).await.unwrap_err();

        assert!(matches!(error, TradingError::RiskBlocked(_)));
    }

    #[tokio::test]
    async fn live_submit_blocks_when_margin_is_insufficient() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(BalanceAdapter { available: 50.0 });
        let engine = ExecutionEngine::new(adapter, live_risk(), Arc::new(OrderJournal::new()));
        let mut intent = order_intent();
        intent.mode = ExecutionMode::Live;
        intent.reduce_only = false;
        intent.order_type = OrderType::Limit;
        intent.exchange = "binance".into();
        intent.quantity = 1.0;
        intent.price = Some(100.0);
        intent.client_order_id = "live-margin-client".into();

        let error = engine.submit(intent).await.unwrap_err();

        assert!(matches!(
            error,
            TradingError::InsufficientMargin {
                required: 100.0,
                available: 50.0,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn live_submit_ignores_non_hyperliquid_spot_balance_for_margin() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(SpotOnlyBalanceAdapter);
        let engine = ExecutionEngine::new(adapter, live_risk(), Arc::new(OrderJournal::new()));
        let mut intent = order_intent();
        intent.mode = ExecutionMode::Live;
        intent.reduce_only = false;
        intent.order_type = OrderType::Limit;
        intent.exchange = "binance".into();
        intent.quantity = 1.0;
        intent.price = Some(100.0);
        intent.client_order_id = "live-spot-balance-client".into();

        let error = engine.submit(intent).await.unwrap_err();

        assert!(matches!(
            error,
            TradingError::InsufficientMargin { available: 0.0, .. }
        ));
    }

    #[tokio::test]
    async fn hyperliquid_unified_spot_balance_counts_for_margin() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(SpotOnlyBalanceAdapter);
        let engine = ExecutionEngine::new(
            adapter,
            live_risk_for("hyperliquid:xyz"),
            Arc::new(OrderJournal::new()),
        );
        let mut intent = order_intent();
        intent.mode = ExecutionMode::Live;
        intent.reduce_only = false;
        intent.order_type = OrderType::Limit;
        intent.exchange = "hyperliquid:xyz".into();
        intent.symbol = "MRVL".into();
        intent.quantity = 3.0;
        intent.price = Some(200.0);
        intent.client_order_id = "live-hl-spot-margin-client".into();

        let record = engine.submit(intent).await.expect("unified spot margin");

        assert_eq!(record.state, LiveOrderState::Accepted);
    }

    #[test]
    fn hyperliquid_xyz_prefers_usdc_when_symbol_has_no_quote() {
        let mut intent = order_intent();
        intent.exchange = "hyperliquid:xyz".into();
        intent.symbol = "MRVL".into();
        let balances = vec![
            VenueBalanceInfo {
                venue: "hyperliquid:xyz:spot".into(),
                currency: "USDT0".into(),
                total: 5_000.0,
                available: 5_000.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            },
            VenueBalanceInfo {
                venue: "hyperliquid:xyz:spot".into(),
                currency: "USDC".into(),
                total: 100.0,
                available: 100.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            },
        ];

        assert_eq!(available_margin(&balances, &intent), 100.0);
    }

    #[test]
    fn hyperliquid_builder_quote_priority_uses_typed_family_parse() {
        assert_eq!(quote_priority("hyperliquid:cash")[0], "USDT0");
        assert_eq!(quote_priority("hyperliquid:km")[0], "USDH");
        assert_eq!(quote_priority("hyperliquid:xyz")[0], "USDC");
        assert_eq!(quote_priority("binance")[0], "USDT");
    }

    #[test]
    fn hedge_margin_check_sums_same_venue_currency() {
        let balances = vec![VenueBalanceInfo {
            venue: "binance".into(),
            currency: "USDT".into(),
            total: 150.0,
            available: 150.0,
            frozen: 0.0,
            unrealized_pnl: 0.0,
        }];
        let long = live_margin_intent("long", "binance", "BTCUSDT", 1.0, 100.0);
        let short = live_margin_intent("short", "BINANCE", "ETHUSDT", 1.0, 100.0);

        let error = ensure_sufficient_margin_for_intents(&balances, &[&long, &short])
            .expect_err("combined requirement must exceed balance");

        assert!(matches!(
            error,
            TradingError::InsufficientMargin {
                required: 200.0,
                available: 150.0,
                ..
            }
        ));
    }

    #[test]
    fn hedge_margin_check_keeps_cross_venue_balances_separate() {
        let balances = vec![
            VenueBalanceInfo {
                venue: "binance".into(),
                currency: "USDT".into(),
                total: 120.0,
                available: 120.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            },
            VenueBalanceInfo {
                venue: "okx".into(),
                currency: "USDT".into(),
                total: 120.0,
                available: 120.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            },
        ];
        let long = live_margin_intent("long", "binance", "BTCUSDT", 1.0, 100.0);
        let short = live_margin_intent("short", "okx", "BTC-USDT-SWAP", 1.0, 100.0);

        ensure_sufficient_margin_for_intents(&balances, &[&long, &short])
            .expect("each venue has enough balance independently");
    }

    #[test]
    fn hedge_margin_check_rejects_when_target_venue_is_insufficient() {
        let balances = vec![
            VenueBalanceInfo {
                venue: "binance".into(),
                currency: "USDT".into(),
                total: 1_000.0,
                available: 1_000.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            },
            VenueBalanceInfo {
                venue: "okx".into(),
                currency: "USDT".into(),
                total: 50.0,
                available: 50.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            },
        ];
        let long = live_margin_intent("long", "binance", "BTCUSDT", 1.0, 100.0);
        let short = live_margin_intent("short", "okx", "BTC-USDT-SWAP", 1.0, 100.0);

        let error = ensure_sufficient_margin_for_intents(&balances, &[&long, &short])
            .expect_err("okx leg must not borrow binance balance");

        assert!(matches!(
            error,
            TradingError::InsufficientMargin {
                exchange,
                required: 100.0,
                available: 50.0,
            } if exchange == "okx"
        ));
    }

    #[test]
    fn hedge_margin_check_rejects_unrelated_asset_as_collateral() {
        let balances = vec![VenueBalanceInfo {
            venue: "binance".into(),
            currency: "BTC".into(),
            total: 10.0,
            available: 10.0,
            frozen: 0.0,
            unrealized_pnl: 0.0,
        }];
        let intent = live_margin_intent("long", "binance", "BTCUSDT", 1.0, 100.0);

        let error = ensure_sufficient_margin_for_intents(&balances, &[&intent])
            .expect_err("an unrelated asset must not become fallback collateral");

        assert_eq!(
            selected_margin_currency_for_intent(&balances, &intent),
            None
        );
        assert!(matches!(
            error,
            TradingError::InsufficientMargin {
                exchange,
                required: 100.0,
                available: 0.0,
            } if exchange == "binance"
        ));
    }

    #[test]
    fn hedge_margin_check_rejects_sibling_hyperliquid_dex_balance() {
        let balances = vec![VenueBalanceInfo {
            venue: "hyperliquid:abc".into(),
            currency: "USDC".into(),
            total: 1_000.0,
            available: 1_000.0,
            frozen: 0.0,
            unrealized_pnl: 0.0,
        }];
        let intent = live_margin_intent("long", "hyperliquid:xyz", "MRVL", 1.0, 100.0);

        let error = ensure_sufficient_margin_for_intents(&balances, &[&intent])
            .expect_err("a sibling Hyperliquid DEX must not cover margin");

        assert!(matches!(
            error,
            TradingError::InsufficientMargin {
                exchange,
                required: 100.0,
                available: 0.0,
            } if exchange == "hyperliquid:xyz"
        ));
    }

    #[tokio::test]
    async fn live_submit_allows_sufficient_margin() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(BalanceAdapter { available: 150.0 });
        let engine = ExecutionEngine::new(adapter, live_risk(), Arc::new(OrderJournal::new()));
        let mut intent = order_intent();
        intent.mode = ExecutionMode::Live;
        intent.reduce_only = false;
        intent.order_type = OrderType::Limit;
        intent.exchange = "binance".into();
        intent.quantity = 1.0;
        intent.price = Some(100.0);
        intent.client_order_id = "live-margin-ok-client".into();

        let record = engine.submit(intent).await.expect("sufficient margin");

        assert_eq!(record.state, LiveOrderState::Accepted);
    }

    #[tokio::test]
    async fn cancel_error_does_not_advance_order_state() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(BalanceAdapter { available: 150.0 });
        let engine = ExecutionEngine::new(adapter, live_risk(), Arc::new(OrderJournal::new()));
        let mut intent = order_intent();
        intent.id = "cancel-fail".into();
        intent.mode = ExecutionMode::Live;
        intent.reduce_only = false;
        intent.order_type = OrderType::Limit;
        intent.exchange = "binance".into();
        intent.quantity = 1.0;
        intent.price = Some(100.0);
        intent.client_order_id = "cancel-fail-client".into();

        let record = engine.submit(intent.clone()).await.expect("submitted");
        assert_eq!(record.state, LiveOrderState::Accepted);

        engine.set_adapter(Arc::new(CancelFailAdapter));
        let error = engine.cancel(&intent.id).await.unwrap_err();

        assert!(matches!(error, TradingError::Exchange(_)));
        let after = engine.journal().get(&intent.id).expect("record remains");
        assert_eq!(after.state, LiveOrderState::Accepted);
    }

    #[tokio::test]
    async fn live_cancel_stays_pending_until_order_query_finality() {
        let cancel_calls = Arc::new(AtomicUsize::new(0));
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(CountingCancelAdapter {
            cancel_calls: Arc::clone(&cancel_calls),
        });
        let engine = ExecutionEngine::new(adapter, live_risk(), Arc::new(OrderJournal::new()));
        let mut intent = live_margin_intent("cancel-terminal", "binance", "BTCUSDT", 1.0, 100.0);
        intent.client_order_id = "cancel-terminal-client".into();

        let record = engine.submit(intent.clone()).await.expect("submitted");
        assert_eq!(record.state, LiveOrderState::Accepted);

        let pending = engine.cancel(&intent.id).await.expect("cancel requested");
        assert_eq!(pending.state, LiveOrderState::CancelRequested);
        assert_eq!(
            pending.last_update_source,
            shared_types::OrderUpdateSource::AdapterAck
        );
        assert_eq!(cancel_calls.load(Ordering::Relaxed), 1);

        let replay = engine.cancel(&intent.id).await.expect("terminal replay");
        assert_eq!(replay.state, LiveOrderState::CancelRequested);
        assert_eq!(cancel_calls.load(Ordering::Relaxed), 1);

        let finality = engine
            .journal()
            .apply_order_info(
                &intent.id,
                &order_info(shared_types::OrderStatus::Canceled),
                10,
            )
            .expect("query finality");
        assert_eq!(finality.state, LiveOrderState::Cancelled);
        assert_eq!(engine.journal().open_order_count(), 0);
    }

    #[tokio::test]
    async fn dry_run_cancel_keeps_immediate_finality() {
        let cancel_calls = Arc::new(AtomicUsize::new(0));
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(CountingCancelAdapter {
            cancel_calls: Arc::clone(&cancel_calls),
        });
        let engine = ExecutionEngine::new(
            adapter,
            RiskEngine::new(RiskConfig::default()),
            Arc::new(OrderJournal::new()),
        );
        let mut intent = order_intent();
        intent.id = "cancel-dry-run".into();
        intent.client_order_id = "cancel-dry-run-client".into();
        intent.order_type = OrderType::Limit;
        intent.price = Some(100.0);
        intent.reduce_only = false;

        let record = engine.submit(intent.clone()).await.expect("submitted");
        assert_eq!(record.state, LiveOrderState::Accepted);

        let cancelled = engine.cancel(&intent.id).await.expect("cancelled");
        assert_eq!(cancelled.state, LiveOrderState::Cancelled);
        assert_eq!(cancel_calls.load(Ordering::Relaxed), 1);
    }

    struct SpotOnlyBalanceAdapter;

    #[async_trait::async_trait]
    impl LiveTradingAdapter for SpotOnlyBalanceAdapter {
        fn name(&self) -> &'static str {
            "spot-only-test"
        }

        fn capabilities(&self) -> ExchangeCapabilities {
            test_capabilities()
        }

        async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
            Ok(ack(intent))
        }

        async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
            Ok(cancel_ack(request))
        }

        async fn get_order(
            &self,
            _symbol: &str,
            _client_order_id: &str,
        ) -> ExchangeResult<Option<shared_types::OrderInfo>> {
            Ok(None)
        }

        async fn get_open_orders(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::OrderInfo>> {
            Ok(Vec::new())
        }

        async fn get_exchange_balances(
            &self,
            _exchange: &str,
            _currency: Option<&str>,
        ) -> ExchangeResult<Vec<VenueBalanceInfo>> {
            Ok(vec![VenueBalanceInfo {
                venue: "hyperliquid:spot".into(),
                currency: "USDC".into(),
                total: 1_000.0,
                available: 1_000.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            }])
        }

        async fn get_positions(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::PositionInfo>> {
            Ok(Vec::new())
        }
    }

    struct BalanceAdapter {
        available: f64,
    }

    struct CountingCancelAdapter {
        cancel_calls: Arc<AtomicUsize>,
    }

    struct CancelFailAdapter;

    struct PlaceFailAdapter {
        error: fn() -> ExchangeError,
    }

    struct PendingPlaceAdapter {
        started: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl LiveTradingAdapter for BalanceAdapter {
        fn name(&self) -> &'static str {
            "balance-test"
        }

        fn capabilities(&self) -> ExchangeCapabilities {
            test_capabilities()
        }

        async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
            Ok(ack(intent))
        }

        async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
            Ok(cancel_ack(request))
        }

        async fn get_order(
            &self,
            _symbol: &str,
            _client_order_id: &str,
        ) -> ExchangeResult<Option<shared_types::OrderInfo>> {
            Ok(None)
        }

        async fn get_open_orders(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::OrderInfo>> {
            Ok(Vec::new())
        }

        async fn get_balances(
            &self,
            _currency: Option<&str>,
        ) -> ExchangeResult<Vec<VenueBalanceInfo>> {
            Ok(vec![VenueBalanceInfo {
                venue: "binance".into(),
                currency: "USDT".into(),
                total: self.available,
                available: self.available,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            }])
        }

        async fn get_positions(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::PositionInfo>> {
            Ok(Vec::new())
        }
    }

    #[async_trait::async_trait]
    impl LiveTradingAdapter for PlaceFailAdapter {
        fn name(&self) -> &'static str {
            "place-fail-test"
        }

        fn capabilities(&self) -> ExchangeCapabilities {
            test_capabilities()
        }

        async fn place_order(&self, _intent: &OrderIntent) -> ExchangeResult<OrderAck> {
            Err((self.error)())
        }

        async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
            Ok(cancel_ack(request))
        }

        async fn get_order(
            &self,
            _symbol: &str,
            _client_order_id: &str,
        ) -> ExchangeResult<Option<shared_types::OrderInfo>> {
            Ok(None)
        }

        async fn get_open_orders(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::OrderInfo>> {
            Ok(Vec::new())
        }

        async fn get_positions(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::PositionInfo>> {
            Ok(Vec::new())
        }
    }

    #[async_trait::async_trait]
    impl LiveTradingAdapter for PendingPlaceAdapter {
        fn name(&self) -> &'static str {
            "pending-place-test"
        }

        fn capabilities(&self) -> ExchangeCapabilities {
            test_capabilities()
        }

        async fn place_order(&self, _intent: &OrderIntent) -> ExchangeResult<OrderAck> {
            self.started.notify_one();
            std::future::pending().await
        }

        async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
            Ok(cancel_ack(request))
        }

        async fn get_order(
            &self,
            _symbol: &str,
            _client_order_id: &str,
        ) -> ExchangeResult<Option<shared_types::OrderInfo>> {
            Ok(None)
        }

        async fn get_open_orders(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::OrderInfo>> {
            Ok(Vec::new())
        }

        async fn get_positions(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::PositionInfo>> {
            Ok(Vec::new())
        }
    }

    #[async_trait::async_trait]
    impl LiveTradingAdapter for CountingCancelAdapter {
        fn name(&self) -> &'static str {
            "counting-cancel-test"
        }

        fn capabilities(&self) -> ExchangeCapabilities {
            test_capabilities()
        }

        async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
            Ok(ack(intent))
        }

        async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
            self.cancel_calls.fetch_add(1, Ordering::Relaxed);
            Ok(cancel_ack(request))
        }

        async fn get_order(
            &self,
            _symbol: &str,
            _client_order_id: &str,
        ) -> ExchangeResult<Option<shared_types::OrderInfo>> {
            Ok(None)
        }

        async fn get_open_orders(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::OrderInfo>> {
            Ok(Vec::new())
        }

        async fn get_balances(
            &self,
            _currency: Option<&str>,
        ) -> ExchangeResult<Vec<VenueBalanceInfo>> {
            Ok(vec![VenueBalanceInfo {
                venue: "binance".into(),
                currency: "USDT".into(),
                total: 150.0,
                available: 150.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            }])
        }

        async fn get_positions(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::PositionInfo>> {
            Ok(Vec::new())
        }
    }

    #[async_trait::async_trait]
    impl LiveTradingAdapter for CancelFailAdapter {
        fn name(&self) -> &'static str {
            "cancel-fail"
        }

        fn capabilities(&self) -> ExchangeCapabilities {
            test_capabilities()
        }

        async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
            Ok(ack(intent))
        }

        async fn cancel_order(&self, _request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
            Err(ExchangeError::Api {
                exchange: "cancel-fail".into(),
                code: "cancel_not_accepted".into(),
                message: "cancel rejected".into(),
            })
        }

        async fn get_order(
            &self,
            _symbol: &str,
            _client_order_id: &str,
        ) -> ExchangeResult<Option<shared_types::OrderInfo>> {
            Ok(None)
        }

        async fn get_open_orders(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::OrderInfo>> {
            Ok(Vec::new())
        }

        async fn get_balances(
            &self,
            _currency: Option<&str>,
        ) -> ExchangeResult<Vec<VenueBalanceInfo>> {
            Ok(Vec::new())
        }

        async fn get_positions(
            &self,
            _symbol: Option<&str>,
        ) -> ExchangeResult<Vec<shared_types::PositionInfo>> {
            Ok(Vec::new())
        }
    }

    fn test_capabilities() -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_testnet: false,
            supports_live: true,
            supports_spot: false,
            supports_perp: true,
            supports_limit_orders: true,
            supports_market_orders: true,
            supports_post_only: true,
            supports_reduce_only: true,
        }
    }

    #[tokio::test]
    async fn ambiguous_place_error_stays_open_for_read_side_reconciliation() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(PlaceFailAdapter {
            error: || ExchangeError::Timeout { seconds: 10 },
        });
        let engine = ExecutionEngine::new(
            adapter,
            RiskEngine::new(RiskConfig::default()),
            Arc::new(OrderJournal::new()),
        );
        let intent = order_intent();

        let error = engine.submit(intent.clone()).await.expect_err("timeout");

        assert!(matches!(
            error,
            TradingError::Exchange(ExchangeError::Timeout { .. })
        ));
        assert_eq!(
            engine.journal().get(&intent.id).map(|record| record.state),
            Some(LiveOrderState::Unknown)
        );
    }

    #[tokio::test]
    async fn cancelled_submit_future_becomes_unknown_for_identity_reconciliation() {
        let started = Arc::new(tokio::sync::Notify::new());
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(PendingPlaceAdapter {
            started: Arc::clone(&started),
        });
        let engine = Arc::new(ExecutionEngine::new(
            adapter,
            RiskEngine::new(RiskConfig::default()),
            Arc::new(OrderJournal::new()),
        ));
        let intent = order_intent();
        let task_engine = Arc::clone(&engine);
        let task_intent = intent.clone();
        let task = tokio::spawn(async move { task_engine.submit(task_intent).await });

        started.notified().await;
        task.abort();
        let _ = task.await;

        let record = engine.journal().get(&intent.id).expect("journal record");
        assert_eq!(record.state, LiveOrderState::Unknown);
        assert!(record
            .message
            .as_deref()
            .is_some_and(|message| message.contains("query by order identity")));
    }

    #[tokio::test]
    async fn deterministic_place_rejection_is_terminal_failed() {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(PlaceFailAdapter {
            error: || ExchangeError::Api {
                exchange: "mock".into(),
                code: "INVALID_ORDER".into(),
                message: "rejected".into(),
            },
        });
        let engine = ExecutionEngine::new(
            adapter,
            RiskEngine::new(RiskConfig::default()),
            Arc::new(OrderJournal::new()),
        );
        let intent = order_intent();

        let error = engine.submit(intent.clone()).await.expect_err("rejected");

        assert!(matches!(
            error,
            TradingError::Exchange(ExchangeError::Api { .. })
        ));
        assert_eq!(
            engine.journal().get(&intent.id).map(|record| record.state),
            Some(LiveOrderState::Failed)
        );
    }

    fn ack(intent: &OrderIntent) -> OrderAck {
        OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("balance-test-order".into()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: common::time::now_ms(),
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        }
    }

    fn cancel_ack(request: &CancelOrderRequest) -> OrderAck {
        OrderAck {
            internal_order_id: request.internal_order_id.clone(),
            exchange_order_id: request.exchange_order_id.clone(),
            client_order_id: request.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Cancelled,
            accepted_at_ms: common::time::now_ms(),
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        }
    }

    fn order_info(status: shared_types::OrderStatus) -> shared_types::OrderInfo {
        shared_types::OrderInfo {
            execution_style: None,
            venue_time_in_force: None,
            client_order_id: None,
            reduce_only: None,
            order_id: "balance-test-order".into(),
            symbol: "BTCUSDT".into(),
            exchange: "binance".into(),
            side: OrderSide::Sell,
            order_type: OrderType::Limit,
            status,
            quantity: 1.0,
            price: 100.0,
            filled_quantity: 0.0,
            filled_price: 0.0,
            fees: 0.0,
            created_at: chrono::Utc::now(),
        }
    }

    fn live_risk() -> RiskEngine {
        live_risk_for("binance")
    }

    fn live_risk_for(exchange: &str) -> RiskEngine {
        let mut allowed_exchanges = BTreeSet::new();
        allowed_exchanges.insert(exchange.into());
        RiskEngine::new(RiskConfig {
            live_trading_enabled: true,
            max_order_notional: 10_000.0,
            allowed_exchanges,
            ..RiskConfig::default()
        })
    }

    fn order_intent() -> OrderIntent {
        OrderIntent {
            id: "unwind".into(),
            source: OrderSource::ArbitragePreview,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "mock".into(),
            symbol: "BTC".into(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 0.1,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: true,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "unwind-client".into(),
            client_order_id_policy: None,
            created_at_ms: 1,
        }
    }

    fn live_margin_intent(
        id: &str,
        exchange: &str,
        symbol: &str,
        quantity: f64,
        price: f64,
    ) -> OrderIntent {
        let mut intent = order_intent();
        intent.id = id.into();
        intent.client_order_id = format!("{id}-client");
        intent.mode = ExecutionMode::Live;
        intent.exchange = exchange.into();
        intent.symbol = symbol.into();
        intent.quantity = quantity;
        intent.price = Some(price);
        intent.order_type = OrderType::Limit;
        intent.reduce_only = false;
        intent
    }
}
