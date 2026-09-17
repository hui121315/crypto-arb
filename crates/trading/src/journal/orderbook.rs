use super::*;
use crate::ledger::OrderbookDepthLedgerInput;

impl OrderJournal {
    pub fn record_orderbook_evidence(
        &self,
        internal_order_id: &str,
        input: &OrderbookDepthLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        let record = self.get(internal_order_id)?;
        let context = self.ledger_context_for(&record);
        let event = self
            .execution_ledger
            .record_orderbook_evidence_with_context(
                &record,
                input,
                source,
                captured_at_ms,
                context.as_ref(),
            )?;
        self.append_ledger_event(&event);
        Some(event)
    }
}
