//! 订单队列状态机：REST seed + WS 流增量 upsert 成单一去重队列，区分 seed/stream problem。
//!
//! 从 `orders.rs` 拆出。投影（按 run 过滤 + problem memo）见 `projection.rs`，
//! feed 装配（REST/WS/兜底轮询）见 `orders.rs`。fail-closed：degraded/空 envelope
//! 绝不伪装成无订单，保留或合成 `ORDER_SEED_DEGRADED` problem。

use shared_types::{ApiProblem, ListEnvelope, ListStatus, OrderRecord, OrderStreamPayload};
use std::collections::HashMap;

const ORDER_SEED_DEGRADED: &str = "ORDER_SEED_DEGRADED";

#[derive(Clone, Default)]
pub(crate) struct OrderQueue {
    pub(in crate::panels::modules::execution::data::orders) rows: Vec<OrderRecord>,
    index: HashMap<String, usize>,
    pub(in crate::panels::modules::execution::data::orders) seed_problem: Option<ApiProblem>,
    pub(in crate::panels::modules::execution::data::orders) stream_problem: Option<ApiProblem>,
}

impl OrderQueue {
    pub(in crate::panels::modules::execution::data) fn runtime_problem(
        &self,
    ) -> Option<ApiProblem> {
        self.seed_problem
            .clone()
            .or_else(|| self.stream_problem.clone())
    }

    pub(in crate::panels::modules::execution::data::orders) fn seed(
        &mut self,
        result: Result<ListEnvelope<OrderRecord>, ApiProblem>,
    ) {
        match result {
            Ok(envelope) => {
                self.seed_problem = order_seed_problem(&envelope);
                for row in envelope.rows {
                    self.apply_receipt(row);
                }
            }
            Err(problem) => self.seed_problem = Some(problem),
        }
    }

    pub(in crate::panels::modules::execution::data::orders) fn apply_stream_payload(
        &mut self,
        event: OrderStreamPayload,
    ) {
        match event {
            OrderStreamPayload::Record(event) => self.upsert_stream(event.record),
            OrderStreamPayload::Reconcile(_) => self.note_reconcile_event(),
        }
    }

    pub(in crate::panels::modules::execution::data::orders) fn note_stream_problem(
        &mut self,
        problem: ApiProblem,
    ) {
        self.stream_problem = Some(problem);
    }

    fn upsert_stream(&mut self, record: OrderRecord) {
        self.stream_problem = None;
        self.upsert_row(record);
    }

    pub(in crate::panels::modules::execution) fn apply_receipt(&mut self, record: OrderRecord) {
        // HTTP carries no ordering within the same millisecond; retain the WS row on a tie.
        if self
            .index
            .get(&record.intent.id)
            .is_some_and(|index| self.rows[*index].updated_at_ms >= record.updated_at_ms)
        {
            return;
        }
        self.upsert_row(record);
    }

    fn note_reconcile_event(&mut self) {
        self.stream_problem = None;
    }

    fn upsert_row(&mut self, record: OrderRecord) {
        let id = record.intent.id.clone();
        if let Some(index) = self.index.get(&id).copied() {
            if !newer_order_should_replace(&self.rows[index], &record) {
                return;
            }
            self.rows[index] = record;
        } else {
            self.index.insert(id, self.rows.len());
            self.rows.push(record);
        }
    }

    #[cfg(test)]
    fn rows(&self) -> Vec<OrderRecord> {
        self.rows.clone()
    }

    #[cfg(test)]
    fn seed_problem(&self) -> Option<ApiProblem> {
        self.seed_problem.clone()
    }

    #[cfg(test)]
    fn stream_problem(&self) -> Option<ApiProblem> {
        self.stream_problem.clone()
    }
}

fn newer_order_should_replace(current: &OrderRecord, next: &OrderRecord) -> bool {
    next.updated_at_ms >= current.updated_at_ms
}

fn order_seed_problem(envelope: &ListEnvelope<OrderRecord>) -> Option<ApiProblem> {
    if envelope.status == ListStatus::Fresh && envelope.problems.is_empty() {
        return None;
    }
    let mut problem = envelope.problems.first().cloned().unwrap_or_else(|| {
        ApiProblem::new(
            ORDER_SEED_DEGRADED,
            "order seed envelope degraded without a backend problem",
        )
    });
    if problem.source.is_none() {
        problem.source = Some(envelope.source.clone());
    }
    let problem_details = problem.details.take();
    problem.details = Some(order_seed_details(envelope, problem_details.as_ref()));
    Some(problem)
}

fn order_seed_details(
    envelope: &ListEnvelope<OrderRecord>,
    problem_details: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "envelopeStatus": envelope.status,
        "envelopeSource": envelope.source.clone(),
        "observedAtMs": envelope.observed_at_ms,
        "returnedCount": envelope.page.returned_count,
        "totalRows": envelope.page.total_rows,
        "problemCount": envelope.problems.len(),
        "problems": envelope.problems.clone(),
        "problemDetails": problem_details,
    })
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{envelope, envelope_with, order, order_at};
    use super::*;

    #[test]
    fn seed_upserts_rows_and_clears_problem() {
        let mut queue = OrderQueue::default();
        queue.seed(Err(ApiProblem::new("TIMEOUT", "slow")));

        queue.seed(Ok(envelope(vec![order("a"), order("b")])));
        queue.upsert_stream(order("a"));

        assert_eq!(queue.rows().len(), 2);
        assert!(queue.seed_problem().is_none());
    }

    #[test]
    fn seed_degraded_envelope_keeps_rows_and_problem() {
        let mut queue = OrderQueue::default();
        let mut backend_problem = ApiProblem::new("LIST_FILTER_INVALID", "bad state");
        backend_problem.details = Some(serde_json::json!({ "field": "state" }));

        queue.seed(Ok(envelope_with(
            vec![order("a")],
            ListStatus::Degraded,
            vec![backend_problem],
        )));

        let problem = queue.seed_problem();
        assert_eq!(queue.rows().len(), 1);
        assert_eq!(
            problem.as_ref().map(|problem| problem.code.as_str()),
            Some("LIST_FILTER_INVALID")
        );
        assert_eq!(
            problem
                .as_ref()
                .and_then(|problem| problem.details.as_ref())
                .and_then(|details| details.get("envelopeStatus"))
                .and_then(serde_json::Value::as_str),
            Some("degraded")
        );
        assert_eq!(
            problem
                .as_ref()
                .and_then(|problem| problem.details.as_ref())
                .and_then(|details| details.get("problems"))
                .and_then(|problems| problems.get(0))
                .and_then(|problem| problem.get("code"))
                .and_then(serde_json::Value::as_str),
            Some("LIST_FILTER_INVALID")
        );
    }

    #[test]
    fn seed_degraded_empty_envelope_is_not_silent_empty_orders() {
        let mut queue = OrderQueue::default();

        queue.seed(Ok(envelope_with(
            Vec::new(),
            ListStatus::Degraded,
            Vec::new(),
        )));

        assert!(queue.rows().is_empty());
        assert_eq!(
            queue.seed_problem().map(|problem| problem.code).as_deref(),
            Some(ORDER_SEED_DEGRADED)
        );
    }

    #[test]
    fn seed_does_not_regress_newer_stream_order() {
        let mut queue = OrderQueue::default();
        queue.upsert_stream(order_at("a", 10));
        queue.seed(Ok(envelope(vec![order_at("a", 5)])));

        assert_eq!(queue.rows()[0].updated_at_ms, 10);
    }

    #[test]
    fn stream_problem_is_distinct_from_seed_problem() {
        let mut queue = OrderQueue::default();
        queue.note_stream_problem(ApiProblem::new("WS_DECODE", "bad payload"));
        assert_eq!(
            queue.stream_problem().map(|p| p.code).as_deref(),
            Some("WS_DECODE")
        );
        assert!(queue.seed_problem().is_none());

        queue.seed(Err(ApiProblem::new("REST_SEED", "seed failed")));
        assert_eq!(
            queue.stream_problem().map(|p| p.code).as_deref(),
            Some("WS_DECODE")
        );

        queue.upsert_stream(order("a"));
        assert!(queue.stream_problem().is_none());
        assert!(queue.seed_problem().is_some());
    }

    #[test]
    fn reconcile_stream_payload_is_not_decode_failure_or_order_row() {
        let mut queue = OrderQueue::default();
        queue.note_stream_problem(ApiProblem::new("WS_PAYLOAD_DECODE", "old"));

        queue.apply_stream_payload(OrderStreamPayload::Reconcile(
            shared_types::OrderStreamReconcileEvent {
                event: "order_reconcile_diff".to_owned(),
                diffs: Vec::new(),
                diff_count: 0,
                timestamp_ms: 10,
            },
        ));

        assert!(queue.stream_problem().is_none());
        assert!(queue.rows().is_empty());
    }
}
