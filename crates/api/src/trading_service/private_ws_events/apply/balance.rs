use super::*;

impl TradingService {
    pub(super) fn record_private_balance_events(
        &self,
        balance_kind: &str,
        rows: &[VenueBalanceInfo],
    ) -> bool {
        let observed_at_ms = common::time::now_ms();
        let mut updated = false;
        for row in rows {
            let event =
                match SqlBalanceLedgerEvent::from_balance_row(balance_kind, row, observed_at_ms) {
                    Ok(event) => event,
                    Err(error) => {
                        tracing::warn!(
                            venue = %row.venue,
                            currency = %row.currency,
                            balance_kind,
                            error = %error,
                            "failed to build private WS balance ledger event"
                        );
                        continue;
                    }
                };
            updated |= self.journal.append_balance_ledger_event(event);
        }
        updated
    }
}
