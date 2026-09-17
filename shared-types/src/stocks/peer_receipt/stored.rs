use super::*;

impl StockPeerOrderReceipt {
    /// Validate a durable snapshot using the same exact event reducer as WS.
    pub fn validate_stored(&self) -> Result<(), &'static str> {
        let mut replay = Self::pending(self.draft.clone(), self.client_order_id.clone())?;
        if self.problem.as_ref().is_some_and(|s| s.len() > 2048)
            || self
                .submission_ack
                .as_ref()
                .is_some_and(|a| a.message.len() > 2048)
            || self.fills.len() > MAX_STOCK_PEER_FILLS
        {
            return Err("stock stored receipt exceeds bounds");
        }
        if let Some(ack) = &self.submission_ack {
            replay.record_submission_ack(ack.clone(), self.order_id.clone())?;
        }
        if !self.unique_fills() {
            return Err("stock stored receipt duplicates an execution");
        }
        let mut fills = self.fills.clone();
        fills.sort_by_key(|f| f.occurred_at_ms);
        for fill in fills {
            replay.apply(self.patch(Some(fill.clone()), fill.occurred_at_ms, false)?)?;
        }
        if self.phase != StockCexOrderPhase::Rejected {
            if let Some(at) = self.updated_at_ms {
                if replay.updated_at_ms.is_some_and(|t| t > at) {
                    return Err("stock stored receipt event time regressed");
                }
                replay.apply(self.patch(None, at, true)?)?;
            }
        }
        if replay.order_id != self.order_id
            || replay.phase != self.phase
            || replay.cumulative_quantity != self.cumulative_quantity
            || replay.cumulative_cost != self.cumulative_cost
            || replay.updated_at_ms != self.updated_at_ms
            || replay.fills != self.fills
        {
            return Err("stock stored receipt cannot be reproduced from original events");
        }
        Ok(())
    }

    /// Delayed ACKs/snapshots can enrich receipts, never remove fills or rewind them.
    pub fn merge_snapshot(&mut self, incoming: &Self) -> Result<(), &'static str> {
        incoming.validate_stored()?;
        if self.draft != incoming.draft || self.client_order_id != incoming.client_order_id {
            return Err("stock snapshot belongs to another original intent");
        }
        let mut next = self.clone();
        let result = (|| {
            if let Some(ack) = &incoming.submission_ack {
                if next.submission_ack.is_none() {
                    next.record_submission_ack(ack.clone(), incoming.order_id.clone())?;
                } else if next.submission_ack != incoming.submission_ack {
                    return Err("stock acknowledgement changed after persistence");
                }
            }
            // Advance the original order totals before adding new fills. Otherwise
            // an older partial cumulative quantity would incorrectly reject them.
            if incoming.phase != StockCexOrderPhase::Rejected {
                if let Some(at) = incoming.updated_at_ms {
                    next.apply(incoming.patch(None, at, true)?)?;
                }
            }
            for fill in &incoming.fills {
                next.apply(incoming.patch(Some(fill.clone()), fill.occurred_at_ms, false)?)?;
            }
            if incoming.evidence_conflict {
                next.mark_conflict(
                    incoming
                        .problem
                        .as_deref()
                        .unwrap_or("stock source reported conflicting original evidence"),
                );
            }
            if !next.evidence_conflict && next.updated_at_ms.is_none() {
                next.problem = incoming.problem.clone();
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                *self = next;
                Ok(())
            }
            Err(e) => {
                self.mark_conflict(e);
                Err(e)
            }
        }
    }

    fn patch(
        &self,
        fill: Option<StockPeerFill>,
        at: i64,
        totals: bool,
    ) -> Result<StockPeerExecutionPatch, &'static str> {
        Ok(StockPeerExecutionPatch {
            order_id: self
                .order_id
                .clone()
                .ok_or("stock original order ID missing")?,
            client_order_id: Some(self.client_order_id.clone()),
            native_symbol: Some(self.draft.request.selection.native_symbol.clone()),
            side: None,
            order_quantity: Some(self.draft.quantity.clone()),
            phase: totals.then_some(self.phase),
            cumulative_quantity: totals.then(|| self.cumulative_quantity.clone()).flatten(),
            cumulative_cost: totals.then(|| self.cumulative_cost.clone()).flatten(),
            fill,
            occurred_at_ms: at,
        })
    }
}
