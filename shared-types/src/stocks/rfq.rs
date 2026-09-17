use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StockRfqSide {
    Bid,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockRfqPhase {
    SubmissionUnknown,
    NotSent,
    Rejected,
    AwaitingQuotes,
    Candidate,
    AcceptedBinding,
    Filled,
    Cancelled,
    Expired,
}

impl StockRfqPhase {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::NotSent | Self::Rejected | Self::Filled | Self::Cancelled | Self::Expired
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockRfqRequest {
    pub request_id: String,
    pub asset: String,
    pub side: StockRfqSide,
    pub quantity: String,
}

/// The UI and server use the same session limits; the server still refreshes them before sending.
pub fn validate_rfq_quantity(
    request: &StockRfqRequest,
    snapshot: &super::StockMarketSnapshot,
    now: i64,
) -> Result<String, String> {
    use super::comparison::positive;
    let security = snapshot
        .security
        .as_ref()
        .filter(|s| s.asset == request.asset)
        .ok_or("请重新选择股票")?;
    let route = snapshot
        .trading_route
        .as_ref()
        .filter(|r| {
            r.kind == super::StockRouteKind::Rfq
                && r.valid_until_ms > now
                && r.symbol.as_deref() == Some(&security.rfq_symbol)
        })
        .ok_or("当前不是已核实的股票 RFQ 时段")?;
    let session = route.session.as_ref().ok_or("股票 RFQ 数量约束缺失")?;
    let quantity = positive(&request.quantity).ok_or("询价股数必须大于 0")?;
    let step = positive(&session.step_size).ok_or("股票步长无效")?;
    let min = positive(&session.min_quantity).ok_or("股票最小股数无效")?;
    if quantity < min || quantity.checked_rem(step).is_none_or(|r| !r.is_zero()) {
        return Err(format!("当前时段至少 {min} 股，股数须按 {step} 对齐"));
    }
    if session
        .max_quantity
        .as_ref()
        .is_some_and(|max| positive(max).is_none_or(|m| quantity > m))
    {
        return Err("询价股数超过当前时段上限".into());
    }
    Ok(quantity.normalize().to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockRfqActionRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRfqCandidate {
    pub quote_id: String,
    // Backpack documents rfqCandidate.p as taker price including the quote fee.
    pub taker_price: String,
    pub source_at_us: i64,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRfqFill {
    pub quote_id: String,
    pub quantity: String,
    pub quote_quantity: String,
    pub price: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRfqSettlement {
    pub attempts: u8,
    pub next_at_ms: Option<i64>,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRfqAcceptance {
    pub plan_id: String,
    pub quote_id: String,
    pub taker_price: String,
    pub submitted_at_ms: i64,
    pub acknowledged: bool,
    pub rejected: bool,
    pub evidence_conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRfq {
    pub request: StockRfqRequest,
    pub client_id: u32,
    pub account_fingerprint: String,
    pub symbol: String,
    pub rfq_id: Option<String>,
    pub phase: StockRfqPhase,
    pub candidate: Option<StockRfqCandidate>,
    pub submission_time_ms: Option<i64>,
    pub expiry_time_ms: Option<i64>,
    pub source_at_us: Option<i64>,
    pub fill_price: Option<String>,
    pub executed_quantity: Option<String>,
    pub executed_quote_quantity: Option<String>,
    #[serde(default)]
    pub fills: Vec<StockRfqFill>,
    #[serde(default)]
    pub settlement: StockRfqSettlement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<StockRfqAcceptance>,
    pub needs_recheck: bool,
    pub cancel_requested: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub problem: Option<String>,
}

impl StockRfq {
    pub fn settlement_pending(&self) -> bool {
        use super::comparison::positive;
        use rust_decimal::Decimal;
        if self.phase != StockRfqPhase::Filled {
            return false;
        }
        let totals =
            self.fills
                .iter()
                .try_fold((Decimal::ZERO, Decimal::ZERO), |(q, usd), fill| {
                    Some((
                        q.checked_add(positive(&fill.quantity)?)?,
                        usd.checked_add(positive(&fill.quote_quantity)?)?,
                    ))
                });
        self.fills.is_empty()
            || totals.is_none_or(|(q, usd)| {
                Some(q) != positive(&self.request.quantity)
                    || Some(q) != self.executed_quantity.as_deref().and_then(positive)
                    || Some(usd) != self.executed_quote_quantity.as_deref().and_then(positive)
            })
    }

    pub fn unresolved(&self) -> bool {
        !self.phase.terminal() || self.settlement_pending()
    }

    pub fn needs_follow_up(&self) -> bool {
        self.unresolved()
            && !((self.acceptance.is_some() || self.phase == StockRfqPhase::Filled)
                && self.settlement.paused)
            && !self
                .acceptance
                .as_ref()
                .is_some_and(|a| a.evidence_conflict)
    }

    pub fn current_candidate(&self, connected: bool, now: i64) -> Option<&StockRfqCandidate> {
        if !connected
            || self.acceptance.is_some()
            || self.needs_recheck
            || self.cancel_requested
            || self.phase != StockRfqPhase::Candidate
            || self.expiry_time_ms.is_none_or(|t| now >= t)
            || self.submission_time_ms.is_none_or(|t| now < t)
        {
            return None;
        }
        self.candidate.as_ref().filter(|q| q.received_at_ms <= now)
    }
}

#[cfg(test)]
mod quantity_tests {
    use super::*;
    use crate::stocks::*;

    #[test]
    fn stock_rfq_quantity_uses_live_session_limits_and_exact_decimals() {
        let mut snapshot = StockMarketSnapshot {
            security: Some(StockSecurity {
                asset: "MU.US".into(),
                ticker: "MU".into(),
                name: "Micron".into(),
                cusip: None,
                sessions: vec![],
                order_books: vec![],
                rfq_symbol: "MU.US_USDC_RFQ".into(),
            }),
            trading_route: Some(StockTradingRoute {
                kind: StockRouteKind::Rfq,
                session: Some(StockSession {
                    name: "Regular".into(),
                    min_quantity: "0.01".into(),
                    max_quantity: Some("10".into()),
                    step_size: "0.01".into(),
                }),
                symbol: Some("MU.US_USDC_RFQ".into()),
                reason: "fixture".into(),
                timezone: Some("America/New_York".into()),
                calendar_at_ms: Some(1),
                valid_until_ms: 100,
            }),
            ..Default::default()
        };
        let mut request = StockRfqRequest {
            request_id: "local".into(),
            asset: "MU.US".into(),
            side: StockRfqSide::Ask,
            quantity: "1.00".into(),
        };
        assert_eq!(validate_rfq_quantity(&request, &snapshot, 99).unwrap(), "1");
        for q in ["0", "-1", "NaN", "0.001", "0.019", "10.01"] {
            request.quantity = q.into();
            assert!(
                validate_rfq_quantity(&request, &snapshot, 99).is_err(),
                "{q}"
            );
        }
        request.quantity = "1".into();
        assert!(validate_rfq_quantity(&request, &snapshot, 100).is_err());
        request.asset = "SNDK.US".into();
        assert!(validate_rfq_quantity(&request, &snapshot, 99).is_err());
        request.asset = "MU.US".into();
        snapshot.trading_route.as_mut().unwrap().session = None;
        assert!(validate_rfq_quantity(&request, &snapshot, 99).is_err());
    }
}
