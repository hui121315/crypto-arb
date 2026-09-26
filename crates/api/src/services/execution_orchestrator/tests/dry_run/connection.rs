use super::*;
use exchange::{ExchangeCapabilities, ExchangeResult, LiveTradingAdapter};
use shared_types::{CancelOrderRequest, OrderAck, OrderInfo, PositionInfo};
use std::sync::Arc;

struct SwitchingAdapter {
    state: AppState,
    mock: trading::MockLiveAdapter,
    calls: parking_lot::Mutex<Vec<OrderIntent>>,
}

#[async_trait::async_trait]
impl LiveTradingAdapter for SwitchingAdapter {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn capabilities(&self) -> ExchangeCapabilities {
        self.mock.capabilities()
    }
    async fn place_order(&self, intent: &OrderIntent) -> ExchangeResult<OrderAck> {
        self.calls.lock().push(intent.clone());
        let ack = self.mock.place_order(intent).await?;
        if !intent.reduce_only {
            let _config = self
                .state
                .trading_runtime_config_mutation_lock()
                .lock()
                .await;
            self.state.trading_service().select_mock_adapter();
        }
        Ok(ack)
    }
    async fn cancel_order(&self, request: &CancelOrderRequest) -> ExchangeResult<OrderAck> {
        self.mock.cancel_order(request).await
    }
    async fn get_order(&self, symbol: &str, client: &str) -> ExchangeResult<Option<OrderInfo>> {
        self.mock.get_order(symbol, client).await
    }
    async fn get_open_orders(&self, symbol: Option<&str>) -> ExchangeResult<Vec<OrderInfo>> {
        self.mock.get_open_orders(symbol).await
    }
    async fn get_positions(&self, symbol: Option<&str>) -> ExchangeResult<Vec<PositionInfo>> {
        self.mock.get_positions(symbol).await
    }
}

#[tokio::test]
async fn context_switch_blocks_second_leg_and_unwinds_on_original_connection() -> anyhow::Result<()>
{
    let state = test_state().await?;
    let now = common::time::now_ms();
    seed_spot_book(&state, "BTC-LONG", 100.0, now);
    seed_spot_book(&state, "BTC-SHORT", 101.0, now);
    let mut preview = preview(now)?;
    preview.execution_binding =
        Some(crate::services::hedge_preview::runtime::capture(&state).await);
    let original_binding = preview.execution_binding.clone();
    let engine = crate::services::hedge_preview::runtime::bind_engine(&state, &preview).await?;
    let adapter = Arc::new(SwitchingAdapter {
        state: state.clone(),
        mock: trading::MockLiveAdapter::new(),
        calls: Default::default(),
    });
    engine.set_adapter(adapter.clone());
    let response =
        confirm_preview(&state, preview.clone(), "switch-after-fill".into(), &engine).await;
    assert_eq!(
        response.status,
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindAttempted
    );
    assert!(response
        .error
        .as_deref()
        .is_some_and(|text| text.contains("执行账户或环境已改变")));
    assert!(response.short_record.is_none());
    let first = response.long_record.as_ref().expect("original first leg");
    let unwind = response
        .unwind_record
        .as_ref()
        .expect("original adapter unwind");
    assert_eq!(first.state, LiveOrderState::Filled);
    assert_eq!(unwind.state, LiveOrderState::Filled);
    assert_eq!(unwind.filled_quantity, first.filled_quantity);
    assert!(unwind.intent.reduce_only);
    let calls = adapter.calls.lock();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].symbol, calls[1].symbol);
    assert_ne!(calls[0].side, calls[1].side);
    drop(calls);
    assert_ne!(
        original_binding,
        Some(crate::services::hedge_preview::runtime::capture(&state).await)
    );
    assert!(
        crate::services::hedge_preview::runtime::bind_engine(&state, &preview)
            .await
            .is_err()
    );
    preview.execution_binding = None;
    assert!(
        crate::services::hedge_preview::runtime::bind_engine(&state, &preview)
            .await
            .is_err()
    );
    Ok(())
}
