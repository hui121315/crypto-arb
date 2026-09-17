//! 订单名义金额推导：成交回报优先，其次风控 evidence，最后 intent 兜底；
//! 缺证据绝不渲染 0，区分「待成交回报」与「名义缺证据」。文案标签见 `labels.rs`。

use shared_types::{LiveOrderState, OrderRecord};

pub(super) fn notional_label(order: &OrderRecord) -> String {
    notional_usd(order)
        .map(money)
        .unwrap_or_else(|| missing_notional_label(order).into())
}

fn notional_usd(order: &OrderRecord) -> Option<f64> {
    filled_notional_usd(order)
        .or_else(|| {
            order
                .risk
                .as_ref()
                .and_then(|risk| finite_positive(risk.computed_notional))
        })
        .or_else(|| intent_notional_usd(order))
}

fn filled_notional_usd(order: &OrderRecord) -> Option<f64> {
    Some(finite_positive(order.filled_quantity?)? * finite_positive(order.filled_price?)?)
}

fn intent_notional_usd(order: &OrderRecord) -> Option<f64> {
    Some(finite_positive(order.intent.quantity)? * finite_positive(order.intent.price?)?)
}

fn finite_positive(value: f64) -> Option<f64> {
    if value.is_finite() && value > 0.0 {
        Some(value)
    } else {
        None
    }
}

fn missing_notional_label(order: &OrderRecord) -> &'static str {
    if has_fill_evidence(order) {
        "名义缺证据"
    } else {
        "待成交回报"
    }
}

fn has_fill_evidence(order: &OrderRecord) -> bool {
    matches!(
        order.state,
        LiveOrderState::PartiallyFilled | LiveOrderState::Filled
    ) || order.filled_quantity.is_some()
        || order.filled_price.is_some()
}

fn money(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("${:.0}K", value / 1_000.0)
    } else {
        format!("${value:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionMode, OrderSide, OrderUpdateSource};

    #[test]
    fn order_notional_uses_filled_price_when_available() {
        let mut order = order();
        order.risk = Some(shared_types::RiskDecision::allow(10.0));
        order.filled_quantity = Some(2.0);
        order.filled_price = Some(12.0);

        assert_eq!(notional_usd(&order), Some(24.0));
        assert_eq!(notional_label(&order), "$24");
    }

    #[test]
    fn order_notional_uses_risk_evidence_before_intent_fallback() {
        let mut order = order();
        order.risk = Some(shared_types::RiskDecision::allow(88.0));
        order.intent.price = Some(10.0);
        order.intent.quantity = 2.0;

        assert_eq!(notional_usd(&order), Some(88.0));
        assert_eq!(notional_label(&order), "$88");
    }

    #[test]
    fn order_notional_missing_price_does_not_render_zero() {
        let mut order = order();
        order.intent.price = None;
        order.risk = None;

        assert_eq!(notional_usd(&order), None);
        assert_eq!(notional_label(&order), "待成交回报");

        order.state = LiveOrderState::Filled;
        order.filled_quantity = Some(1.0);
        assert_eq!(notional_label(&order), "名义缺证据");
    }

    fn order() -> OrderRecord {
        OrderRecord {
            intent: shared_types::OrderIntent {
                id: "order-1".into(),
                source: shared_types::OrderSource::Manual,
                strategy: None,
                mode: ExecutionMode::DryRun,
                client_order_id: "client-1".into(),
                client_order_id_policy: None,
                exchange: "paper".into(),
                symbol: "BTC-USDT".into(),
                side: OrderSide::Buy,
                order_type: shared_types::OrderType::Limit,
                quantity: 1.0,
                price: Some(100.0),
                slippage_tolerance_bps: None,
                time_in_force: shared_types::TimeInForce::Gtc,
                post_only: false,
                reduce_only: false,
                leverage: 1.0,
                margin_mode: shared_types::MarginMode::Cross,
                created_at_ms: 1,
            },
            state: LiveOrderState::Accepted,
            risk: None,
            identity: Default::default(),
            last_update_source: OrderUpdateSource::AdapterAck,
            exchange_order_id: None,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
            updated_at_ms: 1,
        }
    }
}
