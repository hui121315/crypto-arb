use exchange::{ExchangeCapabilities, ExchangeResult, LiveTradingAdapter};
use parking_lot::RwLock;
use shared_types::{
    CancelOrderRequest, ExecutionMode, LiveOrderState, OrderAck, OrderInfo, OrderIntent,
    PositionInfo, VenueBalanceInfo,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const MOCK_TAKER_FEE_RATE: f64 = 0.0005;

#[derive(Debug)]
pub struct MockLiveAdapter {
    next_place_state: RwLock<LiveOrderState>,
    simulated_latency_ms: AtomicU64,
}

impl Default for MockLiveAdapter {
    fn default() -> Self {
        Self {
            next_place_state: RwLock::new(LiveOrderState::Accepted),
            simulated_latency_ms: AtomicU64::new(0),
        }
    }
}

impl MockLiveAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_next_place_state(&self, state: LiveOrderState) {
        *self.next_place_state.write() = state;
    }

    pub fn set_simulated_latency_ms(&self, latency_ms: u64) {
        self.simulated_latency_ms
            .store(latency_ms, Ordering::Relaxed);
    }
}

#[async_trait::async_trait]
impl LiveTradingAdapter for MockLiveAdapter {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities::testnet_limit_only()
    }

    fn order_margin_modes(&self) -> Vec<shared_types::MarginMode> {
        vec![
            shared_types::MarginMode::Cross,
            shared_types::MarginMode::Isolated,
        ]
    }

    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        sleep_simulated_latency(self.simulated_latency_ms.load(Ordering::Relaxed)).await;
        let state = mock_place_state(intent, *self.next_place_state.read());
        let (filled_quantity, filled_price, filled_fee) = mock_fill_evidence(intent, state);
        Ok(OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: match state {
                LiveOrderState::Rejected | LiveOrderState::Failed => None,
                _ => Some(mock_exchange_order_id(intent)),
            },
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state,
            accepted_at_ms: common::time::now_ms(),
            message: match state {
                LiveOrderState::Rejected => Some("mock rejected".into()),
                LiveOrderState::Failed => Some("mock failed".into()),
                _ => None,
            },
            filled_quantity,
            filled_price,
            filled_fee,
        })
    }

    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        Ok(OrderAck {
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
        })
    }

    async fn get_order(
        &self,
        _symbol: &str,
        _client_order_id: &str,
    ) -> ExchangeResult<Option<OrderInfo>> {
        Ok(None)
    }

    async fn get_open_orders(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        Ok(Vec::new())
    }

    async fn get_balances(&self, currency: Option<&str>) -> ExchangeResult<Vec<VenueBalanceInfo>> {
        let row = VenueBalanceInfo {
            venue: "mock".into(),
            currency: "USDT".into(),
            total: 1_000_000.0,
            available: 1_000_000.0,
            frozen: 0.0,
            unrealized_pnl: 0.0,
        };
        if currency.is_some_and(|want| !want.eq_ignore_ascii_case(&row.currency)) {
            Ok(Vec::new())
        } else {
            Ok(vec![row])
        }
    }

    async fn get_positions(&self, _symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        Ok(Vec::new())
    }
}

fn mock_place_state(intent: &OrderIntent, configured: LiveOrderState) -> LiveOrderState {
    if intent.mode == ExecutionMode::DryRun && configured == LiveOrderState::Accepted {
        LiveOrderState::Filled
    } else {
        configured
    }
}

fn mock_fill_evidence(
    intent: &OrderIntent,
    state: LiveOrderState,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    if intent.mode != ExecutionMode::DryRun || state != LiveOrderState::Filled {
        return (None, None, None);
    }
    let quantity = positive(intent.quantity);
    let price = intent.price.and_then(positive);
    let fee = quantity
        .zip(price)
        .map(|(quantity, price)| quantity * price * MOCK_TAKER_FEE_RATE);
    (quantity, price, fee)
}

fn mock_exchange_order_id(intent: &OrderIntent) -> String {
    format!("mock-{}", intent.id)
}

fn positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

async fn sleep_simulated_latency(latency_ms: u64) {
    if latency_ms > 0 {
        tokio::time::sleep(Duration::from_millis(latency_ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionMode, OrderSide, OrderSource, OrderType};

    #[tokio::test]
    async fn place_order_respects_simulated_latency() {
        let adapter = std::sync::Arc::new(MockLiveAdapter::new());
        adapter.set_simulated_latency_ms(20);
        let intent = order_intent();
        let worker = std::sync::Arc::clone(&adapter);
        let mut handle = tokio::spawn(async move { worker.place_order(&intent).await });

        assert!(tokio::time::timeout(Duration::from_millis(1), &mut handle)
            .await
            .is_err());
        assert!(handle.await.is_ok_and(|result| result.is_ok()));
    }

    #[tokio::test]
    async fn dry_run_orders_fill_immediately_by_default() {
        let adapter = MockLiveAdapter::new();
        let ack = adapter
            .place_order(&order_intent())
            .await
            .expect("mock dry-run ack");

        assert_eq!(ack.state, LiveOrderState::Filled);
        assert_eq!(ack.exchange_order_id.as_deref(), Some("mock-o"));
        assert_eq!(ack.filled_quantity, Some(1.0));
        assert_eq!(ack.filled_price, Some(1.0));
        assert_eq!(ack.filled_fee, Some(0.0005));
    }

    #[tokio::test]
    async fn exchange_order_identity_does_not_repeat_after_adapter_restart() {
        let first = MockLiveAdapter::new()
            .place_order(&order_intent_with_id("first"))
            .await
            .expect("first mock ack");
        let second = MockLiveAdapter::new()
            .place_order(&order_intent_with_id("second"))
            .await
            .expect("second mock ack");

        assert_eq!(first.exchange_order_id.as_deref(), Some("mock-first"));
        assert_eq!(second.exchange_order_id.as_deref(), Some("mock-second"));
        assert_ne!(first.exchange_order_id, second.exchange_order_id);
    }

    #[tokio::test]
    async fn explicit_mock_state_overrides_dry_run_default() {
        let adapter = MockLiveAdapter::new();
        adapter.set_next_place_state(LiveOrderState::Rejected);
        let ack = adapter
            .place_order(&order_intent())
            .await
            .expect("mock rejection ack");

        assert_eq!(ack.state, LiveOrderState::Rejected);
        assert!(ack.exchange_order_id.is_none());
        assert_eq!(ack.filled_quantity, None);
        assert_eq!(ack.filled_price, None);
        assert_eq!(ack.filled_fee, None);
    }

    #[tokio::test]
    async fn non_dry_run_fill_does_not_fabricate_fill_evidence() {
        let adapter = MockLiveAdapter::new();
        adapter.set_next_place_state(LiveOrderState::Filled);
        let mut intent = order_intent();
        intent.mode = ExecutionMode::Testnet;
        let ack = adapter
            .place_order(&intent)
            .await
            .expect("mock testnet ack");

        assert_eq!(ack.state, LiveOrderState::Filled);
        assert_eq!(ack.filled_quantity, None);
        assert_eq!(ack.filled_price, None);
        assert_eq!(ack.filled_fee, None);
    }

    fn order_intent() -> OrderIntent {
        order_intent_with_id("o")
    }

    fn order_intent_with_id(id: &str) -> OrderIntent {
        OrderIntent {
            id: id.into(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "mock".into(),
            symbol: "BTC".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(1.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: format!("c-{id}"),
            client_order_id_policy: None,
            created_at_ms: 1,
        }
    }
}
