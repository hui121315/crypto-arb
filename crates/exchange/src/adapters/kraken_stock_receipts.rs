//! Exact, opt-in stock receipts on the existing Kraken executions connection.
//! https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/executions

use crate::error::{ExchangeError, ExchangeResult};
use serde_json::Value;
use shared_types::stocks::{
    stock_exact_decimal, StockCexOrderPhase, StockPeerExecutionPatch, StockPeerFill,
    StockPeerOrderReceipt, StockTradeFee,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use tokio::sync::broadcast;

const MAX_TRACKED: usize = 256;

#[derive(Debug)]
pub(super) struct StockReceipts {
    rows: Mutex<BTreeMap<String, StockPeerOrderReceipt>>,
    retired: Mutex<BTreeSet<String>>,
    submitting: Mutex<BTreeSet<String>>,
    updates: broadcast::Sender<StockPeerOrderReceipt>,
}

impl Default for StockReceipts {
    fn default() -> Self {
        Self {
            rows: Mutex::new(BTreeMap::new()),
            retired: Mutex::new(BTreeSet::new()),
            submitting: Mutex::new(BTreeSet::new()),
            updates: broadcast::channel(256).0,
        }
    }
}

impl StockReceipts {
    // The caller must persist the original intent/receipt before registering it.
    // Registration restores observation only: it never submits or resubmits orders.
    pub(super) fn track(&self, receipt: StockPeerOrderReceipt) -> ExchangeResult<()> {
        self.insert(receipt).map(|_| ())
    }

    pub(super) fn claim_submission(
        &self,
        receipt: StockPeerOrderReceipt,
    ) -> ExchangeResult<Option<StockSubmissionGuard<'_>>> {
        let client = receipt.client_order_id.clone();
        if !self.insert(receipt)? {
            return Ok(None);
        }
        self.submitting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(client.clone());
        Ok(Some(StockSubmissionGuard {
            cache: self,
            client,
        }))
    }

    fn insert(&self, receipt: StockPeerOrderReceipt) -> ExchangeResult<bool> {
        validate_restore(&receipt).map_err(|e| ExchangeError::Parse(e.into()))?;
        let mut rows = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self
            .retired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&receipt.client_order_id)
        {
            return Err(ExchangeError::Parse(
                "stock client order ID was already retired; use its durable receipt".into(),
            ));
        }
        if let Some(existing) = rows.get(&receipt.client_order_id) {
            if existing.draft != receipt.draft
                || (receipt.order_id.is_some()
                    && existing.order_id.is_some()
                    && receipt.order_id != existing.order_id)
            {
                return Err(ExchangeError::Parse(
                    "stock receipt identity is already in use".into(),
                ));
            }
            return Ok(false);
        }
        if rows.len() >= MAX_TRACKED {
            return Err(ExchangeError::Parse(
                "stock receipt capacity reached; persist and release settled receipts first".into(),
            ));
        }
        if receipt
            .order_id
            .as_ref()
            .is_some_and(|id| rows.values().any(|r| r.order_id.as_ref() == Some(id)))
        {
            return Err(ExchangeError::Parse(
                "stock order is already tracked under a different client ID".into(),
            ));
        }
        rows.insert(receipt.client_order_id.clone(), receipt);
        Ok(true)
    }

    pub(super) fn update(
        &self,
        client: &str,
        change: impl FnOnce(&mut StockPeerOrderReceipt),
    ) -> ExchangeResult<StockPeerOrderReceipt> {
        let mut rows = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let row = rows
            .get_mut(client)
            .ok_or_else(|| ExchangeError::Parse("original stock receipt is not tracked".into()))?;
        let old = row.clone();
        change(row);
        if old != *row {
            let _ = self.updates.send(row.clone());
        }
        Ok(row.clone())
    }

    pub(super) fn get(&self, client: &str) -> Option<StockPeerOrderReceipt> {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(client)
            .cloned()
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<StockPeerOrderReceipt> {
        self.updates.subscribe()
    }

    pub(super) fn release(&self, expected: &StockPeerOrderReceipt) -> ExchangeResult<()> {
        let mut rows = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let existing = rows
            .get(&expected.client_order_id)
            .ok_or_else(|| ExchangeError::Parse("stock receipt is not tracked".into()))?;
        if existing != expected || !existing.receipt_complete() {
            return Err(ExchangeError::Parse(
                "stock receipt changed or is not completely settled".into(),
            ));
        }
        if self
            .submitting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&expected.client_order_id)
        {
            return Err(ExchangeError::Parse(
                "stock submission response is still in flight".into(),
            ));
        }
        let mut retired = self
            .retired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if retired.len() >= 4096 {
            return Err(ExchangeError::Parse(
                "stock receipt retirement capacity reached".into(),
            ));
        }
        retired.insert(expected.client_order_id.clone());
        rows.remove(&expected.client_order_id);
        Ok(())
    }

    pub(super) fn apply(&self, text: &str) {
        let mut rows = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if rows.is_empty() {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let Some(events) = v["data"].as_array() else {
            return;
        };
        for event in events {
            let client = event["cl_ord_id"].as_str();
            let order = event["order_id"].as_str();
            let keys: Vec<_> = rows
                .values()
                .filter(|r| {
                    client == Some(r.client_order_id.as_str())
                        || order.is_some_and(|id| r.order_id.as_deref() == Some(id))
                })
                .map(|r| r.client_order_id.clone())
                .collect();
            for key in &keys {
                let row = rows.get_mut(key).expect("selected tracked stock receipt");
                let before = row.clone();
                if keys.len() > 1 {
                    row.mark_conflict("stock execution matches conflicting original identities");
                } else {
                    match parse_execution(event) {
                        Ok(patch) => {
                            let _ = row.apply(patch);
                        }
                        Err(_) => row.mark_conflict(
                            "stock execution fields are invalid; original history required",
                        ),
                    }
                }
                if *row != before {
                    let _ = self.updates.send(row.clone());
                }
            }
        }
    }
}

pub(super) struct StockSubmissionGuard<'a> {
    cache: &'a StockReceipts,
    client: String,
}

impl Drop for StockSubmissionGuard<'_> {
    fn drop(&mut self) {
        self.cache
            .submitting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.client);
    }
}

fn validate_restore(receipt: &StockPeerOrderReceipt) -> Result<(), &'static str> {
    let mut replay =
        StockPeerOrderReceipt::pending(receipt.draft.clone(), receipt.client_order_id.clone())?;
    if receipt.phase == StockCexOrderPhase::Rejected {
        let mut rejection = receipt.clone();
        // A contradictory response must keep the original hold after restart.
        // This copy validates the rejection fields, not permission to settle.
        rejection.evidence_conflict = false;
        if rejection.rejection_proven() {
            return Ok(());
        }
    }
    if let Some(ack) = &receipt.submission_ack {
        if !ack.accepted
            || ack.received_at_ms < receipt.draft.prepared_at_ms
            || receipt.order_id.is_none()
        {
            return Err("stock submission acknowledgement is invalid");
        }
    }
    if receipt.updated_at_ms.is_none()
        && receipt.fills.is_empty()
        && receipt.cumulative_quantity.is_none()
        && receipt.cumulative_cost.is_none()
        && receipt.phase == StockCexOrderPhase::SubmissionUnknown
    {
        return if receipt
            .order_id
            .as_deref()
            .is_none_or(|id| !id.is_empty() && id.len() <= 128)
        {
            Ok(())
        } else {
            Err("invalid original stock order ID")
        };
    }
    if receipt.order_id.is_none() {
        if *receipt != replay {
            return Err("stock receipt without original order ID cannot be restored");
        }
        return Ok(());
    }
    let id = receipt.order_id.clone().unwrap();
    let mut fills = receipt.fills.clone();
    fills.sort_by_key(|f| f.occurred_at_ms);
    if fills
        .windows(2)
        .any(|w| w[0].execution_id == w[1].execution_id)
        || fills
            .iter()
            .map(|f| &f.execution_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != fills.len()
    {
        return Err("stock receipt has duplicate executions");
    }
    for fill in fills {
        replay.apply(StockPeerExecutionPatch {
            order_id: id.clone(),
            client_order_id: Some(receipt.client_order_id.clone()),
            native_symbol: None,
            side: None,
            order_quantity: None,
            phase: None,
            cumulative_quantity: None,
            cumulative_cost: None,
            occurred_at_ms: fill.occurred_at_ms,
            fill: Some(fill),
        })?;
    }
    let at = receipt
        .updated_at_ms
        .ok_or("stock receipt has no original event time")?;
    if replay.updated_at_ms.is_some_and(|t| t > at) {
        return Err("stock receipt event time regressed");
    }
    replay.apply(StockPeerExecutionPatch {
        order_id: id,
        client_order_id: Some(receipt.client_order_id.clone()),
        native_symbol: None,
        side: None,
        order_quantity: None,
        phase: Some(receipt.phase),
        cumulative_quantity: receipt.cumulative_quantity.clone(),
        cumulative_cost: receipt.cumulative_cost.clone(),
        occurred_at_ms: at,
        fill: None,
    })?;
    Ok(())
}

fn decimal(v: &Value) -> Result<String, &'static str> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err("missing stock decimal"),
    };
    Ok(stock_exact_decimal(&s)?.normalize().to_string())
}

fn optional_decimal(row: &Value, key: &str) -> Result<Option<String>, &'static str> {
    row.get(key).map(decimal).transpose()
}

fn optional_text(row: &Value, key: &str) -> Result<Option<String>, &'static str> {
    row.get(key)
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .map(str::to_owned)
                .ok_or("invalid stock identity")
        })
        .transpose()
}

fn parse_execution(row: &Value) -> Result<StockPeerExecutionPatch, &'static str> {
    let kind = row["exec_type"]
        .as_str()
        .ok_or("stock event type missing")?;
    let phase = match row
        .get("order_status")
        .and_then(Value::as_str)
        .unwrap_or(kind)
    {
        "pending_new" => Some(StockCexOrderPhase::SubmissionUnknown),
        "new" | "partially_filled" => Some(StockCexOrderPhase::Open),
        "filled" => Some(StockCexOrderPhase::Filled),
        "canceled" => Some(StockCexOrderPhase::Cancelled),
        "expired" => Some(StockCexOrderPhase::Expired),
        "trade" | "status" | "amended" | "restated" | "iceberg_refill"
            if row.get("order_status").is_none() =>
        {
            None
        }
        _ => return Err("unrecognized stock execution state"),
    };
    let at = chrono::DateTime::parse_from_rfc3339(
        row["timestamp"]
            .as_str()
            .ok_or("stock event time missing")?,
    )
    .map_err(|_| "invalid stock event time")?
    .timestamp_millis();
    let fill = if kind == "trade" {
        let fees = match row.get("fees") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let entries = v
                    .as_array()
                    .filter(|a| a.len() <= 16)
                    .ok_or("invalid stock native fees")?;
                if entries.is_empty() {
                    None
                } else {
                    let mut fees: Vec<_> = entries
                        .iter()
                        .map(|f| {
                            Ok(StockTradeFee {
                                asset: optional_text(f, "asset")?.ok_or("fee asset missing")?,
                                quantity: decimal(&f["qty"])?,
                            })
                        })
                        .collect::<Result<_, &'static str>>()?;
                    fees.sort_by(|a, b| (&a.asset, &a.quantity).cmp(&(&b.asset, &b.quantity)));
                    Some(fees)
                }
            }
        };
        Some(StockPeerFill {
            execution_id: optional_text(row, "exec_id")?.ok_or("stock execution ID missing")?,
            trade_id: row
                .get("trade_id")
                .map(|v| v.as_u64().ok_or("invalid stock trade ID"))
                .transpose()?,
            quantity: decimal(&row["last_qty"])?,
            price: decimal(&row["last_price"])?,
            cost: optional_decimal(row, "cost")?,
            fees,
            occurred_at_ms: at,
        })
    } else {
        None
    };
    Ok(StockPeerExecutionPatch {
        order_id: optional_text(row, "order_id")?.ok_or("stock order ID missing")?,
        client_order_id: optional_text(row, "cl_ord_id")?,
        native_symbol: optional_text(row, "symbol")?,
        side: optional_text(row, "side")?,
        order_quantity: optional_decimal(row, "order_qty")?,
        phase,
        cumulative_quantity: optional_decimal(row, "cum_qty")?,
        cumulative_cost: optional_decimal(row, "cum_cost")?,
        fill,
        occurred_at_ms: at,
    })
}

#[cfg(test)]
pub(super) mod tests;
