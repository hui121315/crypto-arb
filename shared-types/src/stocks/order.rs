use super::*;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StockCexOrderPhase {
    SubmissionUnknown,
    Open,
    Filled,
    Cancelled,
    Expired,
    Rejected,
}

impl StockCexOrderPhase {
    pub fn terminal(self) -> bool {
        !matches!(self, Self::SubmissionUnknown | Self::Open)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockTradeFee {
    pub asset: String,
    pub quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockCexFill {
    pub trade_id: String,
    pub quantity: String,
    pub price: String,
    // None is unproven; an explicit zero fee remains a real receipt.
    pub fee: Option<StockTradeFee>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockOrderRecheck {
    pub attempts: u8,
    pub next_at_ms: Option<i64>,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockCexOrder {
    pub order_id: Option<String>,
    pub phase: StockCexOrderPhase,
    pub executed_quantity: Option<String>,
    pub executed_quote_quantity: Option<String>,
    pub fills: Vec<StockCexFill>,
    pub submitted_at_ms: i64,
    pub updated_at_ms: i64,
    pub recheck: StockOrderRecheck,
    pub problem: Option<String>,
    #[serde(default)]
    pub evidence_conflict: bool,
}

impl StockCexOrder {
    pub fn intent(now: i64) -> Self {
        Self {
            order_id: None,
            phase: StockCexOrderPhase::SubmissionUnknown,
            executed_quantity: None,
            executed_quote_quantity: None,
            fills: vec![],
            submitted_at_ms: now,
            updated_at_ms: now,
            recheck: StockOrderRecheck {
                next_at_ms: Some(now.saturating_add(5_000)),
                ..Default::default()
            },
            problem: None,
            evidence_conflict: false,
        }
    }

    pub fn fill_totals(&self) -> Option<(Decimal, Decimal)> {
        self.fills
            .iter()
            .try_fold((Decimal::ZERO, Decimal::ZERO), |(q, v), f| {
                let size = decimal(&f.quantity)?;
                Some((
                    q.checked_add(size)?,
                    v.checked_add(size.checked_mul(decimal(&f.price)?)?)?,
                ))
            })
    }

    pub fn receipt_complete(&self) -> bool {
        !self.evidence_conflict && self.phase.terminal()
            && self
                .fill_totals()
                .zip(
                    self.executed_quantity
                        .as_deref()
                        .and_then(decimal)
                        .zip(self.executed_quote_quantity.as_deref().and_then(decimal)),
                )
                .is_some_and(|(fills, total)| fills == total)
            && self.fills.iter().all(|f| f.fee.is_some())
    }

    pub fn needs_follow_up(&self) -> bool {
        !self.receipt_complete() && !self.recheck.paused
    }

    /// Native asset movements on this leg only, not total arbitrage profit.
    pub fn net_asset_changes(
        &self,
        instruction: &StockCexInstruction,
    ) -> Option<BTreeMap<String, String>> {
        if !self.receipt_complete() {
            return None;
        }
        let StockCexInstruction::OrderBook { symbol, side, .. } = instruction else {
            return None;
        };
        let (base, quote) = symbol.rsplit_once('_')?;
        let (q, v) = self.fill_totals()?;
        let mut changes = BTreeMap::new();
        let sign = if *side == StockRfqSide::Bid {
            Decimal::ONE
        } else {
            -Decimal::ONE
        };
        changes.insert(base.to_owned(), q.checked_mul(sign)?);
        changes.insert(quote.to_owned(), v.checked_mul(-sign)?);
        for fill in &self.fills {
            let fee = fill.fee.as_ref()?;
            let entry = changes.entry(fee.asset.clone()).or_insert(Decimal::ZERO);
            *entry = entry.checked_sub(decimal(&fee.quantity)?)?;
        }
        Some(
            changes
                .into_iter()
                .map(|(a, n)| (a, n.normalize().to_string()))
                .collect(),
        )
    }
}

fn decimal(s: &str) -> Option<Decimal> {
    Decimal::from_str_exact(s).ok()
}
