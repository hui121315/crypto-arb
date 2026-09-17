use shared_types::{OrderIntent, RiskBlockEvidence, RiskBlockReason, RiskDecision};
use std::collections::BTreeSet;

const RISK_EVIDENCE_SOURCE: &str = "trading.risk_engine";

pub(super) fn decision(
    reasons: Vec<RiskBlockReason>,
    notional: f64,
    evidence: Vec<RiskBlockEvidence>,
) -> RiskDecision {
    if reasons.is_empty() {
        RiskDecision::allow(notional)
    } else {
        RiskDecision::block_with_evidence(reasons, notional, evidence)
    }
}

pub(super) struct RiskEvidenceContext<'a> {
    intent: &'a OrderIntent,
    checked_at_ms: i64,
}

impl<'a> RiskEvidenceContext<'a> {
    pub(super) fn new(intent: &'a OrderIntent) -> Self {
        Self {
            intent,
            checked_at_ms: common::time::now_ms(),
        }
    }

    pub(super) fn push(
        &self,
        reasons: &mut Vec<RiskBlockReason>,
        evidence: &mut Vec<RiskBlockEvidence>,
        code: RiskBlockReason,
        details: (&str, Option<serde_json::Value>, Option<serde_json::Value>),
    ) {
        let (field, actual, limit) = details;
        reasons.push(code);
        evidence.push(self.build(code, field, actual, limit));
    }

    pub(super) fn build(
        &self,
        code: RiskBlockReason,
        field: &str,
        actual: Option<serde_json::Value>,
        limit: Option<serde_json::Value>,
    ) -> RiskBlockEvidence {
        RiskBlockEvidence {
            code,
            field: field.to_owned(),
            actual,
            limit,
            venue: Some(self.intent.exchange.clone()),
            symbol: Some(self.intent.symbol.clone()),
            source: RISK_EVIDENCE_SOURCE.to_owned(),
            checked_at_ms: self.checked_at_ms,
        }
    }
}

pub(super) fn f64_value(value: f64) -> serde_json::Value {
    serde_json::Number::from_f64(value)
        .map(serde_json::Value::Number)
        .unwrap_or_else(|| serde_json::Value::String(value.to_string()))
}

pub(super) fn string_set_value(values: &BTreeSet<String>) -> serde_json::Value {
    serde_json::Value::Array(
        values
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    )
}
