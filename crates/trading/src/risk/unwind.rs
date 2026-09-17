use super::evidence::{decision, f64_value, string_set_value, RiskEvidenceContext};
use super::protected_positions::record_block as record_protected_position_block;
use super::{exchange_allowed, symbol_allowed, RiskEngine, MARKET_NOTIONAL_BUFFER};
use shared_types::{
    normalized_venue_name, ExecutionMode, OrderIntent, RiskBlockReason, RiskDecision,
};

impl RiskEngine {
    pub fn check_unwind(&self, intent: &OrderIntent) -> RiskDecision {
        let config = self.config();
        let notional = computed_unwind_notional(intent);
        let mut reasons = Vec::new();
        let mut evidence = Vec::new();
        let evidence_context = RiskEvidenceContext::new(intent);

        record_protected_position_block(
            &config.protected_positions,
            intent,
            &mut reasons,
            &mut evidence,
            &evidence_context,
        );

        if intent.mode == ExecutionMode::Live && !config.live_trading_enabled {
            evidence_context.push(
                &mut reasons,
                &mut evidence,
                RiskBlockReason::LiveTradingDisabled,
                (
                    "live_trading_enabled",
                    Some(serde_json::Value::Bool(false)),
                    Some(serde_json::Value::Bool(true)),
                ),
            );
        }
        if !exchange_allowed(&config.allowed_exchanges, &intent.exchange) {
            evidence_context.push(
                &mut reasons,
                &mut evidence,
                RiskBlockReason::ExchangeNotAllowed,
                (
                    "exchange",
                    Some(serde_json::Value::String(normalized_venue_name(
                        &intent.exchange,
                    ))),
                    Some(string_set_value(&config.allowed_exchanges)),
                ),
            );
        }
        if !symbol_allowed(&config.allowed_symbols, &intent.symbol) {
            evidence_context.push(
                &mut reasons,
                &mut evidence,
                RiskBlockReason::SymbolNotAllowed,
                (
                    "symbol",
                    Some(serde_json::Value::String(intent.symbol.clone())),
                    Some(string_set_value(&config.allowed_symbols)),
                ),
            );
        }
        if !intent.quantity.is_finite() || intent.quantity <= 0.0 {
            evidence_context.push(
                &mut reasons,
                &mut evidence,
                RiskBlockReason::NonPositiveQuantity,
                (
                    "quantity",
                    Some(f64_value(intent.quantity)),
                    Some(serde_json::Value::String("finite > 0".to_owned())),
                ),
            );
        }
        if intent.order_type != shared_types::OrderType::Market || !intent.reduce_only {
            evidence_context.push(
                &mut reasons,
                &mut evidence,
                RiskBlockReason::MarketOrderNotReduceOnly,
                (
                    "unwind_policy",
                    Some(serde_json::json!({
                        "orderType": format!("{:?}", intent.order_type),
                        "reduceOnly": intent.reduce_only,
                    })),
                    Some(serde_json::Value::String(
                        "market reduce-only unwind".to_owned(),
                    )),
                ),
            );
        }
        if notional > config.max_order_notional {
            evidence_context.push(
                &mut reasons,
                &mut evidence,
                RiskBlockReason::MaxOrderNotionalExceeded,
                (
                    "computed_notional",
                    Some(f64_value(notional)),
                    Some(f64_value(config.max_order_notional)),
                ),
            );
        }

        decision(reasons, notional, evidence)
    }
}

fn computed_unwind_notional(intent: &OrderIntent) -> f64 {
    let quantity = intent.quantity.abs();
    let Some(price) = intent
        .price
        .filter(|price| price.is_finite() && *price > 0.0)
    else {
        return 0.0;
    };
    if !quantity.is_finite() || quantity <= 0.0 {
        return f64::INFINITY;
    }
    (quantity * price * MARKET_NOTIONAL_BUFFER).min(f64::MAX)
}
