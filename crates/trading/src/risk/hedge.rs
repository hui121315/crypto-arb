use super::evidence::{f64_value, RiskEvidenceContext};
use super::RiskEngine;
use shared_types::{OrderIntent, RiskBlockReason, RiskDecision};

impl RiskEngine {
    pub fn check_hedge(
        &self,
        long_leg: &OrderIntent,
        short_leg: &OrderIntent,
        open_orders: usize,
    ) -> (RiskDecision, RiskDecision) {
        let mut long = self.check_order(long_leg, open_orders);
        let mut short = self.check_order(short_leg, open_orders.saturating_add(1));
        let long_notional = long.computed_notional;
        let short_notional = short.computed_notional;
        let long_quantity = long_leg.quantity.abs();
        let short_quantity = short_leg.quantity.abs();
        let larger = long_quantity.max(short_quantity);

        if larger > 0.0 {
            let imbalance = (long_quantity - short_quantity).abs() / larger;
            let max_hedge_imbalance_pct = self.config().max_hedge_imbalance_pct;
            if imbalance > max_hedge_imbalance_pct {
                record_hedge_imbalance(
                    &HedgeImbalanceCheck {
                        long_leg,
                        short_leg,
                        long_quantity,
                        short_quantity,
                        long_notional,
                        short_notional,
                        imbalance,
                        max_hedge_imbalance_pct,
                    },
                    &mut HedgeDecisions {
                        long: &mut long,
                        short: &mut short,
                    },
                );
            }
        }

        (long, short)
    }
}

struct HedgeImbalanceCheck<'a> {
    long_leg: &'a OrderIntent,
    short_leg: &'a OrderIntent,
    long_quantity: f64,
    short_quantity: f64,
    long_notional: f64,
    short_notional: f64,
    imbalance: f64,
    max_hedge_imbalance_pct: f64,
}

struct HedgeDecisions<'a> {
    long: &'a mut RiskDecision,
    short: &'a mut RiskDecision,
}

fn record_hedge_imbalance(check: &HedgeImbalanceCheck<'_>, decisions: &mut HedgeDecisions<'_>) {
    decisions.long.allowed = false;
    decisions.short.allowed = false;
    let actual = serde_json::json!({
        "imbalance": f64_value(check.imbalance),
        "longQuantity": f64_value(check.long_quantity),
        "shortQuantity": f64_value(check.short_quantity),
        "longNotional": f64_value(check.long_notional),
        "shortNotional": f64_value(check.short_notional),
    });
    let limit = Some(f64_value(check.max_hedge_imbalance_pct));

    push_hedge_block(
        decisions.long,
        check.long_leg,
        actual.clone(),
        limit.clone(),
    );
    push_hedge_block(decisions.short, check.short_leg, actual, limit);
}

fn push_hedge_block(
    decision: &mut RiskDecision,
    intent: &OrderIntent,
    actual: serde_json::Value,
    limit: Option<serde_json::Value>,
) {
    if !decision
        .reasons
        .contains(&RiskBlockReason::HedgeImbalanceExceeded)
    {
        decision
            .reasons
            .push(RiskBlockReason::HedgeImbalanceExceeded);
    }
    decision
        .evidence
        .push(RiskEvidenceContext::new(intent).build(
            RiskBlockReason::HedgeImbalanceExceeded,
            "hedge_imbalance_ratio",
            Some(actual),
            limit,
        ));
}
