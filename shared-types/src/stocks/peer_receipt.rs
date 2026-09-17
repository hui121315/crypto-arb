use super::{StockCexOrderPhase, StockChainDirection, StockPeerOrderDraft, StockTradeFee};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub const MAX_STOCK_PEER_FILLS: usize = 512;
mod stored;
mod submission;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerOrderAck {
    pub accepted: bool,
    pub request_id: u64,
    pub received_at_ms: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerCancelAck {
    pub client_order_id: String,
    pub accepted: Option<bool>,
    pub received_at_ms: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerFill {
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trade_id: Option<u64>,
    pub quantity: String,
    pub price: String,
    pub cost: Option<String>,
    pub fees: Option<Vec<StockTradeFee>>,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerOrderReceipt {
    pub draft: StockPeerOrderDraft,
    pub client_order_id: String,
    pub order_id: Option<String>,
    pub phase: StockCexOrderPhase,
    pub cumulative_quantity: Option<String>,
    pub cumulative_cost: Option<String>,
    pub fills: Vec<StockPeerFill>,
    pub updated_at_ms: Option<i64>,
    pub evidence_conflict: bool,
    pub problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_ack: Option<StockPeerOrderAck>,
}

// Rebased xStock quantities are equity shares, not withdrawable token units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockPeerCashSettlement {
    pub base_asset: String,
    pub equity_shares_change: String,
    pub quote_asset: String,
    pub quote_change: String,
}

#[derive(Debug, Clone)]
pub struct StockPeerExecutionPatch {
    pub order_id: String,
    pub client_order_id: Option<String>,
    pub native_symbol: Option<String>,
    pub side: Option<String>,
    pub order_quantity: Option<String>,
    pub phase: Option<StockCexOrderPhase>,
    pub cumulative_quantity: Option<String>,
    pub cumulative_cost: Option<String>,
    pub fill: Option<StockPeerFill>,
    pub occurred_at_ms: i64,
}

pub fn stock_exact_decimal(value: &str) -> Result<Decimal, &'static str> {
    if value.len() > 128 {
        return Err("stock decimal exceeds length limit");
    }
    let parsed = if let Some((base, _)) = value.split_once(['e', 'E']) {
        Decimal::from_str_exact(base).and_then(|_| Decimal::from_scientific(value))
    } else {
        Decimal::from_str_exact(value)
    };
    parsed.map_err(|_| "stock decimal is invalid or exceeds exact precision")
}

fn exact_add(a: Decimal, b: Decimal) -> Option<Decimal> {
    let sum = a.checked_add(b)?;
    (sum.checked_sub(a) == Some(b) && sum.checked_sub(b) == Some(a)).then_some(sum)
}

impl StockPeerOrderReceipt {
    pub fn pending(
        draft: StockPeerOrderDraft,
        client_order_id: String,
    ) -> Result<Self, &'static str> {
        // Validates identity and amounts without treating an old restored draft as executable.
        draft.kraken_validation("identity-only", 1, draft.prepared_at_ms)?;
        stock_exact_decimal(&draft.quantity)?;
        stock_exact_decimal(&draft.limit_price)?;
        if client_order_id.is_empty()
            || client_order_id.len() > 18
            || !client_order_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("stock client order ID is invalid");
        }
        Ok(Self {
            draft,
            client_order_id,
            order_id: None,
            phase: StockCexOrderPhase::SubmissionUnknown,
            cumulative_quantity: None,
            cumulative_cost: None,
            fills: Vec::new(),
            updated_at_ms: None,
            evidence_conflict: false,
            problem: None,
            submission_ack: None,
        })
    }

    pub fn mark_conflict(&mut self, message: &str) {
        self.evidence_conflict = true;
        self.problem = Some(message.into());
    }

    pub fn apply(&mut self, patch: StockPeerExecutionPatch) -> Result<(), &'static str> {
        let mut next = self.clone();
        match next.apply_checked(patch) {
            Ok(()) => {
                *self = next;
                Ok(())
            }
            Err(error) => {
                self.mark_conflict(error);
                Err(error)
            }
        }
    }

    fn apply_checked(&mut self, p: StockPeerExecutionPatch) -> Result<(), &'static str> {
        if self
            .submission_ack
            .as_ref()
            .is_some_and(|ack| !ack.accepted)
        {
            return Err("stock execution contradicts an explicit submission rejection");
        }
        let native = &self.draft.request.selection.native_symbol;
        let side = if self.draft.request.direction == StockChainDirection::Buy {
            "sell"
        } else {
            "buy"
        };
        let quantity = stock_exact_decimal(&self.draft.quantity)?;
        for value in [&p.cumulative_quantity, &p.cumulative_cost]
            .into_iter()
            .flatten()
        {
            if stock_exact_decimal(value)? < Decimal::ZERO {
                return Err("negative stock cumulative receipt");
            }
        }
        if p.order_id.is_empty()
            || p.order_id.len() > 128
            || self.order_id.as_ref().is_some_and(|id| id != &p.order_id)
            || p.client_order_id
                .as_ref()
                .is_some_and(|id| id != &self.client_order_id)
            || (self.order_id.is_none()
                && p.client_order_id.as_deref() != Some(&self.client_order_id))
            || p.native_symbol
                .as_ref()
                .is_some_and(|symbol| symbol != native)
            || p.side.as_deref().is_some_and(|s| s != side)
            || p.order_quantity
                .as_deref()
                .map(stock_exact_decimal)
                .transpose()?
                .is_some_and(|n| n != quantity)
            || p.occurred_at_ms < self.draft.prepared_at_ms
        {
            return Err("stock execution identity or original quantity conflicts");
        }
        self.order_id = Some(p.order_id);
        if let Some(fill) = p.fill {
            validate_fill(&fill, &self.draft)?;
            if let Some(old) = self.fills.iter_mut().find(|f| {
                f.execution_id == fill.execution_id
                    || f.trade_id.is_some() && f.trade_id == fill.trade_id
            }) {
                if (old.trade_id.is_some()
                    && fill.trade_id.is_some()
                    && old.trade_id != fill.trade_id)
                    || old.quantity != fill.quantity
                    || old.price != fill.price
                    || old.occurred_at_ms != fill.occurred_at_ms
                    || (old.cost.is_some() && fill.cost.is_some() && old.cost != fill.cost)
                    || (old.fees.is_some() && fill.fees.is_some() && old.fees != fill.fees)
                {
                    return Err("stock execution ID has conflicting receipt data");
                }
                if old.cost.is_none() {
                    old.cost = fill.cost;
                }
                if old.fees.is_none() {
                    old.fees = fill.fees;
                }
                if old.trade_id.is_none() {
                    old.trade_id = fill.trade_id;
                }
            } else {
                if self.fills.len() >= MAX_STOCK_PEER_FILLS {
                    return Err("stock fill receipt capacity reached; original history required");
                }
                self.fills.push(fill);
                self.fills
                    .sort_by(|a, b| a.execution_id.cmp(&b.execution_id));
            }
        }
        let lower_cumulative = self
            .cumulative_quantity
            .as_deref()
            .zip(p.cumulative_quantity.as_deref())
            .map(|(a, b)| Ok::<_, &'static str>(stock_exact_decimal(b)? < stock_exact_decimal(a)?))
            .transpose()?
            .unwrap_or(false);
        let fresh = self.updated_at_ms.is_none_or(|at| {
            p.occurred_at_ms > at || (p.occurred_at_ms == at && !lower_cumulative)
        });
        if fresh {
            for (known, incoming) in [
                (&mut self.cumulative_quantity, p.cumulative_quantity),
                (&mut self.cumulative_cost, p.cumulative_cost),
            ] {
                if let Some(value) = incoming {
                    let value = stock_exact_decimal(&value)?;
                    if value < Decimal::ZERO
                        || known
                            .as_deref()
                            .map(stock_exact_decimal)
                            .transpose()?
                            .is_some_and(|old| value < old)
                    {
                        return Err("stock cumulative receipt regressed");
                    }
                    *known = Some(value.normalize().to_string());
                }
            }
            if let Some(phase) = p.phase {
                if terminal(self.phase) && terminal(phase) && phase != self.phase {
                    return Err("stock terminal receipt conflicts");
                }
                if !terminal(self.phase) {
                    self.phase = phase;
                }
            }
            self.updated_at_ms = Some(p.occurred_at_ms);
        }
        let (filled, cost) = self.fill_totals()?;
        let cumulative = self
            .cumulative_quantity
            .as_deref()
            .map(stock_exact_decimal)
            .transpose()?;
        if filled > quantity
            || cumulative.is_some_and(|n| n > quantity || filled > n)
            || cost.is_some_and(|cost| {
                self.cumulative_cost
                    .as_deref()
                    .and_then(|n| stock_exact_decimal(n).ok())
                    .is_some_and(|n| cost > n)
            })
        {
            return Err("stock fills exceed the original order or cumulative receipt");
        }
        if !self.evidence_conflict {
            self.problem = None;
        }
        Ok(())
    }

    fn fill_totals(&self) -> Result<(Decimal, Option<Decimal>), &'static str> {
        let mut qty = Decimal::ZERO;
        let mut cost = Some(Decimal::ZERO);
        for fill in &self.fills {
            qty = exact_add(qty, stock_exact_decimal(&fill.quantity)?)
                .ok_or("stock quantity overflow or precision loss")?;
            cost = match (cost, fill.cost.as_deref()) {
                (Some(a), Some(b)) => Some(
                    exact_add(a, stock_exact_decimal(b)?)
                        .ok_or("stock cost overflow or precision loss")?,
                ),
                _ => None,
            };
        }
        Ok((qty, cost))
    }

    pub fn receipt_complete(&self) -> bool {
        if self.phase == StockCexOrderPhase::Rejected {
            return self.rejection_proven();
        }
        if self.evidence_conflict || !terminal(self.phase) || self.order_id.is_none() {
            return false;
        }
        if self.fills.len() > MAX_STOCK_PEER_FILLS
            || self
                .fills
                .iter()
                .any(|f| validate_fill(f, &self.draft).is_err())
        {
            return false;
        }
        if !self.unique_fills() {
            return false;
        }
        let Ok((qty, cost)) = self.fill_totals() else {
            return false;
        };
        let expected_qty = self
            .cumulative_quantity
            .as_deref()
            .and_then(|s| stock_exact_decimal(s).ok());
        let expected_cost = self
            .cumulative_cost
            .as_deref()
            .and_then(|s| stock_exact_decimal(s).ok());
        expected_qty == Some(qty)
            && expected_cost.is_some()
            && cost == expected_cost
            && (self.phase != StockCexOrderPhase::Filled
                || stock_exact_decimal(&self.draft.quantity).ok() == Some(qty))
            && self
                .fills
                .iter()
                .all(|f| f.fees.as_ref().is_some_and(|fees| !fees.is_empty()))
    }

    pub fn cash_settlement(&self) -> Option<StockPeerCashSettlement> {
        if !self.receipt_complete() {
            return None;
        }
        let (qty, cost) = self.fill_totals().ok()?;
        let mut fees = Decimal::ZERO;
        for f in &self.fills {
            for fee in f.fees.as_ref()? {
                // Preserve unexpected native fees, but do not invent a share/FX conversion for them.
                if fee.asset != self.draft.quote_asset {
                    return None;
                }
                fees = exact_add(fees, stock_exact_decimal(&fee.quantity).ok()?)?;
            }
        }
        let selling = self.draft.request.direction == StockChainDirection::Buy;
        Some(StockPeerCashSettlement {
            base_asset: self
                .draft
                .request
                .selection
                .native_symbol
                .split_once('/')?
                .0
                .into(),
            equity_shares_change: (if selling { -qty } else { qty }).normalize().to_string(),
            quote_asset: self.draft.quote_asset.clone(),
            quote_change: exact_add(if selling { cost? } else { -cost? }, -fees)?
                .normalize()
                .to_string(),
        })
    }

    fn unique_fills(&self) -> bool {
        let mut executions = std::collections::BTreeSet::new();
        let mut trades = std::collections::BTreeSet::new();
        self.fills.iter().all(|f| {
            executions.insert(&f.execution_id) && f.trade_id.is_none_or(|id| trades.insert(id))
        })
    }
}

fn validate_fill(fill: &StockPeerFill, draft: &StockPeerOrderDraft) -> Result<(), &'static str> {
    let qty = stock_exact_decimal(&fill.quantity)?;
    let price = stock_exact_decimal(&fill.price)?;
    let limit = stock_exact_decimal(&draft.limit_price)?;
    if fill.execution_id.is_empty()
        || fill.execution_id.len() > 128
        || qty <= Decimal::ZERO
        || price <= Decimal::ZERO
        || fill.occurred_at_ms < draft.prepared_at_ms
        || (draft.request.direction == StockChainDirection::Buy && price < limit)
        || (draft.request.direction == StockChainDirection::Sell && price > limit)
        || fill
            .cost
            .as_deref()
            .map(stock_exact_decimal)
            .transpose()?
            .is_some_and(|n| n <= Decimal::ZERO)
    {
        return Err("stock fill quantity, price or identity is invalid");
    }
    if let Some(fees) = &fill.fees {
        if fees.len() > 16 {
            return Err("stock native fee capacity exceeded");
        }
        for fee in fees {
            if fee.asset.is_empty() || fee.asset.len() > 64 {
                return Err("stock native fee asset is missing");
            }
            stock_exact_decimal(&fee.quantity)?;
        }
    }
    Ok(())
}

fn terminal(p: StockCexOrderPhase) -> bool {
    matches!(
        p,
        StockCexOrderPhase::Filled
            | StockCexOrderPhase::Cancelled
            | StockCexOrderPhase::Expired
            | StockCexOrderPhase::Rejected
    )
}
