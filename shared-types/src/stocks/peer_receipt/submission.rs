use super::*;
use serde_json::{json, Value};

impl StockPeerOrderReceipt {
    /// Compile only a new, durably owned intent. Validation-only API requests
    /// never call this compiler and cannot select `validate=false` themselves.
    pub fn kraken_submission(
        &self,
        token: &str,
        request_id: u64,
        now: i64,
    ) -> Result<Value, &'static str> {
        let pending = Self::pending(self.draft.clone(), self.client_order_id.clone())?;
        if self != &pending {
            return Err("stock submission must use a new original intent");
        }
        for (at, limit) in [
            (self.draft.prepared_at_ms, 3_000),
            (self.draft.source_at_ms, 3_000),
            (self.draft.metadata_at_ms, self.draft.metadata_max_age_ms()),
        ] {
            if now.checked_sub(at).is_none_or(|age| age < 0 || age > limit) {
                return Err("stock executable quote or instrument evidence expired");
            }
        }
        let mut request = self.draft.kraken_validation(token, request_id, now)?;
        let expiry = self
            .draft
            .prepared_at_ms
            .checked_add(3_000)
            .ok_or("stock deadline overflow")?
            .min(
                self.draft
                    .source_at_ms
                    .checked_add(3_000)
                    .ok_or("stock deadline overflow")?,
            )
            .min(now.checked_add(2_000).ok_or("stock deadline overflow")?);
        if expiry.checked_sub(now).is_none_or(|left| left < 500) {
            return Err("stock quote has insufficient matching time remaining");
        }
        request["params"]["deadline"] = json!(chrono::DateTime::from_timestamp_millis(expiry)
            .ok_or("stock deadline invalid")?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        request["params"]["validate"] = json!(false);
        request["params"]["cl_ord_id"] = json!(self.client_order_id);
        Ok(request)
    }

    pub fn record_submission_ack(
        &mut self,
        ack: StockPeerOrderAck,
        order_id: Option<String>,
    ) -> Result<(), &'static str> {
        let conflict = ack.received_at_ms < self.draft.prepared_at_ms
            || self
                .submission_ack
                .as_ref()
                .is_some_and(|old| old.accepted != ack.accepted)
            || if ack.accepted {
                order_id
                    .as_deref()
                    .is_none_or(|id| id.is_empty() || id.len() > 128)
                    || self
                        .order_id
                        .as_ref()
                        .zip(order_id.as_ref())
                        .is_some_and(|(old, new)| old != new)
                    || self.phase == StockCexOrderPhase::Rejected
            } else {
                order_id.is_some()
                    || self.order_id.is_some()
                    || !self.fills.is_empty()
                    || !matches!(
                        self.phase,
                        StockCexOrderPhase::SubmissionUnknown | StockCexOrderPhase::Rejected
                    )
            };
        if conflict {
            self.mark_conflict("stock submission acknowledgement conflicts with original receipt");
            return Err("stock submission acknowledgement conflicts with original receipt");
        }
        if ack.accepted {
            self.order_id = order_id;
        } else {
            self.phase = StockCexOrderPhase::Rejected;
            self.cumulative_quantity = Some("0".into());
            self.cumulative_cost = Some("0".into());
            self.updated_at_ms = Some(ack.received_at_ms);
        }
        if !self.evidence_conflict {
            self.problem = None;
        }
        self.submission_ack = Some(ack);
        Ok(())
    }

    pub fn rejection_proven(&self) -> bool {
        !self.evidence_conflict
            && self.phase == StockCexOrderPhase::Rejected
            && self.order_id.is_none()
            && self.fills.is_empty()
            && self.cumulative_quantity.as_deref() == Some("0")
            && self.cumulative_cost.as_deref() == Some("0")
            && self
                .submission_ack
                .as_ref()
                .is_some_and(|a| !a.accepted && a.received_at_ms >= self.draft.prepared_at_ms)
    }
}
