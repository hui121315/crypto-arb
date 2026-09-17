use super::evidence::{decision, f64_value, string_set_value, RiskEvidenceContext};
use super::protected_positions::record_block as record_protected_position_block;
use super::{exchange_allowed, symbol_allowed, RiskEngine, MARKET_NOTIONAL_BUFFER};
use shared_types::{
    normalized_venue_name, ExecutionMode, OrderIntent, OrderSource, RiskBlockEvidence,
    RiskBlockReason, RiskDecision,
};

impl RiskEngine {
    pub fn check_order(&self, intent: &OrderIntent, open_orders: usize) -> RiskDecision {
        let config = self.config();
        let notional = computed_notional(intent);
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

        if config.kill_switch_active && !intent.reduce_only {
            evidence_context.push(
                &mut reasons,
                &mut evidence,
                RiskBlockReason::KillSwitchActive,
                (
                    "kill_switch_active",
                    Some(serde_json::Value::Bool(true)),
                    Some(serde_json::Value::Bool(false)),
                ),
            );
        }
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

        record_price_policy(intent, &mut reasons, &mut evidence, &evidence_context);
        record_notional_limits(
            LimitCheck {
                intent,
                open_orders,
                max_order_notional: config.max_order_notional,
                max_open_orders: config.max_open_orders,
                notional,
            },
            &mut reasons,
            &mut evidence,
            &evidence_context,
        );

        decision(reasons, notional, evidence)
    }
}

fn record_price_policy(
    intent: &OrderIntent,
    reasons: &mut Vec<RiskBlockReason>,
    evidence: &mut Vec<RiskBlockEvidence>,
    evidence_context: &RiskEvidenceContext<'_>,
) {
    match intent.order_type {
        shared_types::OrderType::Limit | shared_types::OrderType::PostOnly => match intent.price {
            Some(p) if p.is_finite() && p > 0.0 => {}
            Some(price) => evidence_context.push(
                reasons,
                evidence,
                RiskBlockReason::NonPositivePrice,
                (
                    "price",
                    Some(f64_value(price)),
                    Some(serde_json::Value::String("finite > 0".to_owned())),
                ),
            ),
            None => evidence_context.push(
                reasons,
                evidence,
                RiskBlockReason::MissingLimitPrice,
                (
                    "price",
                    None,
                    Some(serde_json::Value::String("required limit price".to_owned())),
                ),
            ),
        },
        shared_types::OrderType::Market => {
            match intent.price {
                Some(price) if price.is_finite() && price > 0.0 => {}
                price => evidence_context.push(
                    reasons,
                    evidence,
                    RiskBlockReason::NonPositivePrice,
                    (
                        "reference_price",
                        price.map(f64_value),
                        Some(serde_json::Value::String("finite > 0".to_owned())),
                    ),
                ),
            }
            if !intent.reduce_only && !market_open_allowed(intent) {
                evidence_context.push(
                    reasons,
                    evidence,
                    RiskBlockReason::MarketOrderNotReduceOnly,
                    (
                        "market_open_policy",
                        Some(serde_json::json!({
                            "orderType": "market",
                            "reduceOnly": intent.reduce_only,
                            "source": format!("{:?}", intent.source),
                        })),
                        Some(serde_json::Value::String(
                            "reduce-only or arbitrage preview source".to_owned(),
                        )),
                    ),
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
struct LimitCheck<'a> {
    intent: &'a OrderIntent,
    open_orders: usize,
    max_order_notional: f64,
    max_open_orders: usize,
    notional: f64,
}

fn record_notional_limits(
    check: LimitCheck<'_>,
    reasons: &mut Vec<RiskBlockReason>,
    evidence: &mut Vec<RiskBlockEvidence>,
    evidence_context: &RiskEvidenceContext<'_>,
) {
    if check.notional > check.max_order_notional && !check.intent.reduce_only {
        evidence_context.push(
            reasons,
            evidence,
            RiskBlockReason::MaxOrderNotionalExceeded,
            (
                "computed_notional",
                Some(f64_value(check.notional)),
                Some(f64_value(check.max_order_notional)),
            ),
        );
    }
    if check.open_orders >= check.max_open_orders && !check.intent.reduce_only {
        evidence_context.push(
            reasons,
            evidence,
            RiskBlockReason::MaxOpenOrdersExceeded,
            (
                "open_orders_before_submit",
                Some(serde_json::json!(check.open_orders)),
                Some(serde_json::json!({
                    "maxOpenOrders": check.max_open_orders,
                    "rule": "open_orders < max_open_orders",
                })),
            ),
        );
    }
}

fn market_open_allowed(intent: &OrderIntent) -> bool {
    matches!(intent.source, OrderSource::ArbitragePreview)
}

fn computed_notional(intent: &OrderIntent) -> f64 {
    let quantity = intent.quantity.abs();
    let price = intent.price.unwrap_or(0.0);
    if !quantity.is_finite() || !price.is_finite() || quantity <= 0.0 || price <= 0.0 {
        return f64::INFINITY;
    }
    let buffer = match intent.order_type {
        shared_types::OrderType::Market => MARKET_NOTIONAL_BUFFER,
        shared_types::OrderType::Limit | shared_types::OrderType::PostOnly => 1.0,
    };
    (quantity * price * buffer).min(f64::MAX)
}
