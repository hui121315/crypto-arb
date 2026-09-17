#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
use shared_types::{
    CancelOrderRequest, ExecutionMode, LiveOrderState, OrderAck, OrderInfo, OrderIntent, OrderSide,
    OrderSource, OrderStatus, OrderType, RiskBlockReason, VenueBalanceInfo,
};
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::Arc;
use trading::{
    ExecutionEngine, MockLiveAdapter, OrderJournal, RiskConfig, RiskEngine, TradingError,
};

#[allow(clippy::panic)]
fn must_ok<T, E: Debug>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error:?}"),
    }
}

#[allow(clippy::panic)]
fn must_err<T: Debug, E>(result: Result<T, E>, context: &str) -> E {
    match result {
        Ok(value) => panic!("{context}: unexpected ok {value:?}"),
        Err(error) => error,
    }
}

#[allow(clippy::panic)]
fn must_some<T>(value: Option<T>, context: &str) -> T {
    match value {
        Some(value) => value,
        None => panic!("{context}: none"),
    }
}

#[allow(clippy::panic)]
fn risk_block_reasons(error: TradingError) -> Vec<RiskBlockReason> {
    match error {
        TradingError::RiskBlocked(reasons) => reasons,
        other => panic!("unexpected error: {other:?}"),
    }
}

fn intent(id: &str) -> OrderIntent {
    OrderIntent {
        id: id.into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("client-{id}"),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn engine(config: RiskConfig) -> ExecutionEngine {
    ExecutionEngine::new(
        Arc::new(MockLiveAdapter::new()),
        RiskEngine::new(config),
        Arc::new(OrderJournal::new()),
    )
}

fn engine_with_adapter(adapter: Arc<MockLiveAdapter>, config: RiskConfig) -> ExecutionEngine {
    ExecutionEngine::new(
        adapter,
        RiskEngine::new(config),
        Arc::new(OrderJournal::new()),
    )
}

fn allowed_config() -> RiskConfig {
    RiskConfig {
        live_trading_enabled: false,
        kill_switch_active: false,
        max_order_notional: 1_000.0,
        max_open_orders: 10,
        max_hedge_imbalance_pct: 0.01,
        liquidation_warn_pct: 15.0,
        liquidation_danger_pct: 8.0,
        allowed_exchanges: BTreeSet::from(["mock".to_owned()]),
        allowed_symbols: BTreeSet::from(["BTC".to_owned()]),
        protected_positions: Vec::new(),
        auto_profit_close: Default::default(),
    }
}

#[tokio::test]
async fn submit_limit_order_records_accepted_state() {
    let engine = engine(allowed_config());

    let record = must_ok(engine.submit(intent("o1")).await, "accepted");

    assert_eq!(record.state, LiveOrderState::Accepted);
    assert_eq!(record.intent.client_order_id, "client-o1");
    assert!(record.exchange_order_id.is_some());
    assert!(record.risk.as_ref().is_some_and(|risk| risk.allowed));
}

#[tokio::test]
async fn kill_switch_blocks_order() {
    let mut cfg = allowed_config();
    cfg.kill_switch_active = true;
    let engine = engine(cfg);

    let err = must_err(engine.submit(intent("o2")).await, "blocked");

    let reasons = risk_block_reasons(err);
    assert!(reasons.contains(&RiskBlockReason::KillSwitchActive));
    let record = must_some(engine.journal().get("o2"), "journal record");
    assert_eq!(record.state, LiveOrderState::Rejected);
}

#[tokio::test]
async fn duplicate_client_order_id_is_idempotent() {
    let engine = engine(allowed_config());
    let first = intent("o3");
    let mut second = intent("o4");
    second.client_order_id = first.client_order_id.clone();

    let a = must_ok(engine.submit(first).await, "first");
    let b = must_ok(engine.submit(second).await, "duplicate");

    assert_eq!(a.intent.id, "o3");
    assert_eq!(b.intent.id, "o3");
    assert_eq!(engine.journal().open_order_count(), 1);
}

#[tokio::test]
async fn cancel_moves_order_to_cancel_requested_until_finality() {
    let engine = engine(allowed_config());
    must_ok(engine.submit(intent("o5")).await, "accepted");

    let cancelled = must_ok(engine.cancel("o5").await, "cancel requested");

    assert_eq!(cancelled.state, LiveOrderState::CancelRequested);
    assert_eq!(engine.journal().open_order_count(), 1);
}

#[tokio::test]
async fn cancel_returns_terminal_state_when_finality_beats_adapter_ack() {
    let journal = Arc::new(OrderJournal::new());
    let adapter = Arc::new(FinalityFirstCancelAdapter {
        journal: Arc::clone(&journal),
    });
    let mut config = allowed_config();
    config.live_trading_enabled = true;
    let engine = ExecutionEngine::new(adapter, RiskEngine::new(config), journal);
    let mut order = intent("cancel-finality-first");
    order.mode = ExecutionMode::Live;

    let accepted = must_ok(engine.submit(order).await, "accepted");
    assert_eq!(accepted.state, LiveOrderState::Accepted);

    let cancelled = must_ok(
        engine.cancel("cancel-finality-first").await,
        "terminal finality wins",
    );

    assert_eq!(cancelled.state, LiveOrderState::Cancelled);
    assert_eq!(engine.journal().open_order_count(), 0);
}

#[tokio::test]
async fn market_order_without_reference_price_is_blocked() {
    let engine = engine(allowed_config());
    let mut order = intent("o6");
    order.order_type = OrderType::Market;
    order.price = None;
    order.reduce_only = false;

    let err = must_err(engine.submit(order).await, "blocked");

    let reasons = risk_block_reasons(err);
    assert!(reasons.contains(&RiskBlockReason::NonPositivePrice));
}

#[tokio::test]
async fn mock_reject_records_rejected_state() {
    let adapter = Arc::new(MockLiveAdapter::new());
    adapter.set_next_place_state(LiveOrderState::Rejected);
    let engine = engine_with_adapter(adapter, allowed_config());

    let record = must_ok(engine.submit(intent("o7")).await, "mock reject ack");

    assert_eq!(record.state, LiveOrderState::Rejected);
    assert!(record.exchange_order_id.is_none());
    assert_eq!(record.message.as_deref(), Some("mock rejected"));
}

#[tokio::test]
async fn mock_partial_fill_records_partially_filled_state() {
    let adapter = Arc::new(MockLiveAdapter::new());
    adapter.set_next_place_state(LiveOrderState::PartiallyFilled);
    let engine = engine_with_adapter(adapter, allowed_config());

    let record = must_ok(engine.submit(intent("o8")).await, "partial fill");

    assert_eq!(record.state, LiveOrderState::PartiallyFilled);
    assert!(record.exchange_order_id.is_some());
    assert_eq!(engine.journal().open_order_count(), 1);
}

struct FinalityFirstCancelAdapter {
    journal: Arc<OrderJournal>,
}

#[async_trait::async_trait]
impl exchange::LiveTradingAdapter for FinalityFirstCancelAdapter {
    fn name(&self) -> &'static str {
        "finality-first-cancel"
    }

    fn capabilities(&self) -> exchange::ExchangeCapabilities {
        exchange::ExchangeCapabilities {
            supports_testnet: true,
            supports_live: true,
            supports_spot: false,
            supports_perp: true,
            supports_limit_orders: true,
            supports_market_orders: true,
            supports_post_only: true,
            supports_reduce_only: true,
        }
    }

    async fn place_order(&self, intent: &OrderIntent) -> exchange::ExchangeResult<OrderAck> {
        Ok(OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("finality-first-order".to_owned()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 1,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    async fn cancel_order(
        &self,
        request: &CancelOrderRequest,
    ) -> exchange::ExchangeResult<OrderAck> {
        let finality = OrderInfo {
            execution_style: None,
            venue_time_in_force: None,
            client_order_id: Some(request.client_order_id.clone()),
            reduce_only: None,
            order_id: "finality-first-order".to_owned(),
            symbol: request.symbol.clone(),
            exchange: request.exchange.clone(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            status: OrderStatus::Canceled,
            quantity: 0.01,
            price: 50_000.0,
            filled_quantity: 0.0,
            filled_price: 0.0,
            fees: 0.0,
            created_at: chrono::Utc::now(),
        };
        let _ = self
            .journal
            .apply_order_info(&request.internal_order_id, &finality, 2);
        Ok(OrderAck {
            internal_order_id: request.internal_order_id.clone(),
            exchange_order_id: request.exchange_order_id.clone(),
            client_order_id: request.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Cancelled,
            accepted_at_ms: 3,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
    }

    async fn get_order(
        &self,
        _symbol: &str,
        _client_order_id: &str,
    ) -> exchange::ExchangeResult<Option<OrderInfo>> {
        Ok(None)
    }

    async fn get_open_orders(
        &self,
        _symbol: Option<&str>,
    ) -> exchange::ExchangeResult<Vec<OrderInfo>> {
        Ok(Vec::new())
    }

    async fn get_balances(
        &self,
        _currency: Option<&str>,
    ) -> exchange::ExchangeResult<Vec<VenueBalanceInfo>> {
        Ok(vec![VenueBalanceInfo {
            venue: "mock".to_owned(),
            currency: "USDT".to_owned(),
            total: 1_000.0,
            available: 1_000.0,
            frozen: 0.0,
            unrealized_pnl: 0.0,
        }])
    }

    async fn get_positions(
        &self,
        _symbol: Option<&str>,
    ) -> exchange::ExchangeResult<Vec<shared_types::PositionInfo>> {
        Ok(Vec::new())
    }
}
