use super::*;
use trading::FundingLedgerInput;

impl TradingService {
    pub(super) fn apply_private_ws_funding(
        &self,
        delta: &PrivateFundingDelta,
    ) -> PrivateWsApplyOutcome {
        self.apply_funding_delta(delta, OrderUpdateSource::PrivateWs)
    }

    pub(in crate::trading_service) fn apply_funding_delta(
        &self,
        delta: &PrivateFundingDelta,
        source: OrderUpdateSource,
    ) -> PrivateWsApplyOutcome {
        let (ledger_events, funding_skip_reason) = match self.record_funding_delta(delta, source) {
            Ok(event) => (vec![event], None),
            Err(reason) => (Vec::new(), Some(reason)),
        };
        let dirty = self.mark_private_event_account_dirty(PrivateAccountDirty::new(
            &delta.venue,
            PrivateAccountScope::All,
            "funding_event",
        ));
        PrivateWsApplyOutcome {
            ledger_updated: !ledger_events.is_empty(),
            ledger_events,
            funding_skip_reason,
            account_cache_dirty: Some(dirty),
            ..PrivateWsApplyOutcome::default()
        }
    }

    fn record_funding_delta(
        &self,
        delta: &PrivateFundingDelta,
        source: OrderUpdateSource,
    ) -> Result<ExecutionLedgerEvent, shared_types::FundingPaymentIngestSkipReason> {
        let input = FundingLedgerInput {
            venue_event_id: delta.venue_event_id.clone(),
            amount: delta.amount,
            currency: delta.currency.clone(),
            funding_time_ms: delta.occurred_at_ms,
        };
        self.journal
            .record_funding_by_venue_symbol_reported_deferred_sql(
                &delta.venue,
                &delta.coin,
                &input,
                source,
                common::time::now_ms(),
            )
    }
}
