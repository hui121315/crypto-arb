use super::private_ws_mapper::funding_payment_delta;
use super::*;
use shared_types::{
    FundingPaymentData, FundingPaymentIngestReport, FundingPaymentIngestRouteFailure,
    FundingPaymentIngestSkipReason,
};

const FUNDING_PAYMENT_ROUTE_OPERATION: &str = "funding_payments";

pub(crate) type PrivateFundingPaymentIngestReport = FundingPaymentIngestReport;

pub(crate) struct PrivateFundingPaymentIngestBatch {
    pub(crate) report: PrivateFundingPaymentIngestReport,
    pub(crate) ledger_events: Vec<ExecutionLedgerEvent>,
}

mod kucoin_symbols;

impl TradingService {
    pub(crate) async fn ingest_configured_private_funding_payments(
        &self,
        credentials: AdapterCredentials,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> PrivateFundingPaymentIngestBatch {
        if credentials.available_count() == 0 {
            return self.publish_private_funding_payment_ingest_batch(
                PrivateFundingPaymentIngestReport {
                    observed_at_ms: common::time::now_ms(),
                    window_start_ms: start_time_ms,
                    window_end_ms: end_time_ms,
                    unsupported: true,
                    ..PrivateFundingPaymentIngestReport::default()
                },
            );
        }
        let kucoin_symbols = match self.kucoin_funding_symbols(start_time_ms, end_time_ms) {
            Ok(symbols) => symbols,
            Err(error) => {
                return self.publish_private_funding_payment_ingest_batch(
                    PrivateFundingPaymentIngestReport {
                        observed_at_ms: common::time::now_ms(),
                        window_start_ms: start_time_ms,
                        window_end_ms: end_time_ms,
                        fetch_error: Some(error.to_string()),
                        ..PrivateFundingPaymentIngestReport::default()
                    },
                );
            }
        };
        let rows = match live_adapters::funding_payments_from_credentials(
            credentials,
            Arc::clone(&self.route_failures),
            &kucoin_symbols,
            start_time_ms,
            end_time_ms,
        )
        .await
        {
            Ok(rows) => rows,
            Err(error) if unsupported_funding_payment_error(&error) => {
                return self.publish_private_funding_payment_ingest_batch(
                    PrivateFundingPaymentIngestReport {
                        observed_at_ms: common::time::now_ms(),
                        window_start_ms: start_time_ms,
                        window_end_ms: end_time_ms,
                        unsupported: true,
                        ..PrivateFundingPaymentIngestReport::default()
                    },
                );
            }
            Err(error) => {
                return self.publish_private_funding_payment_ingest_batch(
                    PrivateFundingPaymentIngestReport {
                        observed_at_ms: common::time::now_ms(),
                        window_start_ms: start_time_ms,
                        window_end_ms: end_time_ms,
                        fetch_error: Some(error.to_string()),
                        ..PrivateFundingPaymentIngestReport::default()
                    },
                );
            }
        };
        let mut batch = self.ingest_private_funding_payment_rows(rows, start_time_ms, end_time_ms);
        attach_route_failures(
            &mut batch.report,
            self.take_route_failures(FUNDING_PAYMENT_ROUTE_OPERATION),
        );
        self.publish_private_funding_payment_batch(batch)
    }

    #[cfg(test)]
    pub(crate) async fn ingest_private_funding_payments(
        &self,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> PrivateFundingPaymentIngestReport {
        let rows = match self
            .engine
            .adapter()
            .get_funding_payments(None, start_time_ms, end_time_ms)
            .await
        {
            Ok(rows) => rows,
            Err(error) if unsupported_funding_payment_error(&error) => {
                return self.publish_private_funding_payment_ingest_report(
                    PrivateFundingPaymentIngestReport {
                        observed_at_ms: common::time::now_ms(),
                        window_start_ms: start_time_ms,
                        window_end_ms: end_time_ms,
                        unsupported: true,
                        ..PrivateFundingPaymentIngestReport::default()
                    },
                );
            }
            Err(error) => {
                return self.publish_private_funding_payment_ingest_report(
                    PrivateFundingPaymentIngestReport {
                        observed_at_ms: common::time::now_ms(),
                        window_start_ms: start_time_ms,
                        window_end_ms: end_time_ms,
                        fetch_error: Some(error.to_string()),
                        ..PrivateFundingPaymentIngestReport::default()
                    },
                );
            }
        };
        let mut batch = self.ingest_private_funding_payment_rows(rows, start_time_ms, end_time_ms);
        attach_route_failures(
            &mut batch.report,
            self.take_route_failures(FUNDING_PAYMENT_ROUTE_OPERATION),
        );
        self.publish_private_funding_payment_batch(batch).report
    }

    fn ingest_private_funding_payment_rows(
        &self,
        rows: Vec<FundingPaymentData>,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
    ) -> PrivateFundingPaymentIngestBatch {
        let mut report = PrivateFundingPaymentIngestReport {
            observed_at_ms: common::time::now_ms(),
            window_start_ms: start_time_ms,
            window_end_ms: end_time_ms,
            fetched: rows.len(),
            ..PrivateFundingPaymentIngestReport::default()
        };
        let mut ledger_events = Vec::new();
        for row in rows {
            let Some(delta) = funding_payment_delta(row) else {
                report.invalid = report.invalid.saturating_add(1);
                continue;
            };
            report.mapped = report.mapped.saturating_add(1);
            let outcome = self.apply_funding_delta(&delta, OrderUpdateSource::FundingPoller);
            if outcome.ledger_updated {
                report.ledger_events = report
                    .ledger_events
                    .saturating_add(outcome.ledger_events.len());
                ledger_events.extend(outcome.ledger_events);
            } else {
                report.skipped = report.skipped.saturating_add(1);
                count_skip_reason(
                    &mut report,
                    outcome
                        .funding_skip_reason
                        .unwrap_or(FundingPaymentIngestSkipReason::UnmatchedOrAmbiguousOrder),
                );
            }
        }
        report.refresh_skip_reasons();
        PrivateFundingPaymentIngestBatch {
            report,
            ledger_events,
        }
    }

    pub(crate) fn latest_private_funding_payment_ingest_report(
        &self,
    ) -> Option<PrivateFundingPaymentIngestReport> {
        self.latest_funding_payment_ingest
            .load_full()
            .map(|report| (*report).clone())
    }

    pub(crate) fn record_private_funding_payment_storage_error(
        &self,
        mut report: PrivateFundingPaymentIngestReport,
        error: &str,
    ) -> PrivateFundingPaymentIngestReport {
        report.ledger_events = 0;
        report.fetch_error = Some(format!("funding ledger durability failed: {error}"));
        self.publish_private_funding_payment_ingest_report(report)
    }

    fn publish_private_funding_payment_ingest_report(
        &self,
        mut report: PrivateFundingPaymentIngestReport,
    ) -> PrivateFundingPaymentIngestReport {
        report.refresh_skip_reasons();
        self.latest_funding_payment_ingest
            .store(Some(Arc::new(report.clone())));
        report
    }

    fn publish_private_funding_payment_ingest_batch(
        &self,
        report: PrivateFundingPaymentIngestReport,
    ) -> PrivateFundingPaymentIngestBatch {
        PrivateFundingPaymentIngestBatch {
            report: self.publish_private_funding_payment_ingest_report(report),
            ledger_events: Vec::new(),
        }
    }

    fn publish_private_funding_payment_batch(
        &self,
        mut batch: PrivateFundingPaymentIngestBatch,
    ) -> PrivateFundingPaymentIngestBatch {
        batch.report = self.publish_private_funding_payment_ingest_report(batch.report);
        batch
    }
}

fn count_skip_reason(
    report: &mut PrivateFundingPaymentIngestReport,
    reason: FundingPaymentIngestSkipReason,
) {
    match reason {
        FundingPaymentIngestSkipReason::InvalidRow => {
            report.invalid = report.invalid.saturating_add(1);
        }
        FundingPaymentIngestSkipReason::DuplicateOrAlreadyRecorded => {
            report.duplicate_or_already_recorded =
                report.duplicate_or_already_recorded.saturating_add(1);
        }
        FundingPaymentIngestSkipReason::InvalidMatchKey => {
            report.invalid_match_key = report.invalid_match_key.saturating_add(1);
            report.unmatched_or_ambiguous_order =
                report.unmatched_or_ambiguous_order.saturating_add(1);
        }
        FundingPaymentIngestSkipReason::NoMatchingOrder => {
            report.no_matching_order = report.no_matching_order.saturating_add(1);
            report.unmatched_or_ambiguous_order =
                report.unmatched_or_ambiguous_order.saturating_add(1);
        }
        FundingPaymentIngestSkipReason::NoFilledAnchor => {
            report.no_filled_anchor = report.no_filled_anchor.saturating_add(1);
            report.unmatched_or_ambiguous_order =
                report.unmatched_or_ambiguous_order.saturating_add(1);
        }
        FundingPaymentIngestSkipReason::AmbiguousOrderGroup => {
            report.ambiguous_order_group = report.ambiguous_order_group.saturating_add(1);
            report.unmatched_or_ambiguous_order =
                report.unmatched_or_ambiguous_order.saturating_add(1);
        }
        FundingPaymentIngestSkipReason::UnmatchedOrAmbiguousOrder => {
            report.unmatched_or_ambiguous_order =
                report.unmatched_or_ambiguous_order.saturating_add(1);
        }
    }
}

fn attach_route_failures(
    report: &mut PrivateFundingPaymentIngestReport,
    failures: Vec<RouteFailure>,
) {
    report.route_failures = failures.len();
    report.route_failure_details = failures
        .into_iter()
        .map(|failure| FundingPaymentIngestRouteFailure {
            venue: failure.venue,
            operation: failure.operation.to_owned(),
            message: failure.error.to_string(),
        })
        .collect();
}

fn unsupported_funding_payment_error(error: &exchange::ExchangeError) -> bool {
    matches!(
        error,
        exchange::ExchangeError::NotImplemented(feature)
            | exchange::ExchangeError::UnsupportedCapability(feature)
            if *feature == "get_funding_payments"
    )
}

#[cfg(test)]
mod tests;
