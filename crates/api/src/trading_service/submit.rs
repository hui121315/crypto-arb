use super::*;
use crate::services::live_order_proof_health::LiveOrderProofProblemInput;

mod recovery;
mod submission_contract;
use recovery::{ambiguous_submit_result, resolve_submit_recovery};
use submission_contract::compile_submission_intent;

impl TradingService {
    pub(crate) fn get_order(&self, internal_order_id: &str) -> Option<OrderRecord> {
        self.journal.get(internal_order_id)
    }

    pub(crate) fn get_order_by_client_order_id(
        &self,
        client_order_id: &str,
    ) -> Option<OrderRecord> {
        self.journal.get_by_client_order_id(client_order_id)
    }

    pub(crate) fn get_order_by_exchange_order_id(
        &self,
        exchange_order_id: &str,
    ) -> Option<OrderRecord> {
        self.journal.get_by_exchange_order_id(exchange_order_id)
    }

    pub(crate) async fn refresh_order_state(
        &self,
        internal_order_id: &str,
    ) -> TradingResult<Option<OrderRecord>> {
        let record = self
            .journal
            .get(internal_order_id)
            .ok_or_else(|| trading::TradingError::OrderNotFound(internal_order_id.to_owned()))?;
        let identity = record.identity_snapshot();
        let context = OrderSubmissionContext {
            product: identity.product,
            ..OrderSubmissionContext::default()
        };
        let remote = if let Some(exchange_order_id) = identity.exchange_order_id.as_deref() {
            self.engine
                .adapter()
                .get_exchange_order_by_exchange_order_id_with_context(
                    &record.intent.exchange,
                    &record.intent.symbol,
                    exchange_order_id,
                    &context,
                )
                .await?
        } else {
            let mut remote = None;
            for client_order_id in refresh_order_query_client_ids(&record) {
                remote = self
                    .engine
                    .adapter()
                    .get_exchange_order_with_context(
                        &record.intent.exchange,
                        &record.intent.symbol,
                        &client_order_id,
                        &context,
                    )
                    .await?;
                if remote.is_some() {
                    break;
                }
            }
            remote
        };
        let Some(remote) = remote else {
            return Ok(None);
        };
        let updated = self
            .journal
            .apply_order_info(internal_order_id, &remote, common::time::now_ms())
            .ok_or_else(|| trading::TradingError::OrderNotFound(internal_order_id.to_owned()))?;
        self.apply_open_order_cache(&remote);
        if filled_quantity_changes_account_state(updated.filled_quantity) {
            self.mark_filled_account_cache_stale(
                &updated.intent.exchange,
                "order_query_terminal_update",
            );
        }
        self.live_order_proof_health
            .record_order_query_cancel_finality_from_record(&updated);
        Ok(Some(updated))
    }

    pub(crate) async fn submit(
        &self,
        intent: shared_types::OrderIntent,
    ) -> TradingResult<OrderRecord> {
        self.journal.clear_execution_ledger_context(&intent.id);
        self.submit_inner(intent, OrderSubmissionContext::default())
            .await
    }

    pub(crate) async fn submit_with_ledger_context_and_context(
        &self,
        intent: OrderIntent,
        ledger_context: ExecutionLedgerOrderContext,
        context: OrderSubmissionContext,
    ) -> TradingResult<OrderRecord> {
        self.journal
            .attach_execution_ledger_context(&intent.id, ledger_context);
        self.submit_inner(intent, context).await
    }

    pub(super) async fn submit_inner(
        &self,
        intent: OrderIntent,
        context: OrderSubmissionContext,
    ) -> TradingResult<OrderRecord> {
        let intent = compile_submission_intent(intent, &context)?;
        ensure_live_order_mutation_audit_trail(intent.mode)?;
        let internal_order_id = intent.id.clone();
        let proof_mode = intent.mode;
        let proof_venue = intent.exchange.clone();
        let result = self.engine.submit_with_context(intent, context).await;
        let result = self
            .recover_ambiguous_submit_result(&internal_order_id, result)
            .await;
        if let Ok(record) = result.as_ref() {
            self.live_order_proof_health
                .record_submit_ack_from_record(record);
            self.mark_open_order_cache_stale(&record.intent.exchange);
        } else if let Err(error) = result.as_ref() {
            self.record_live_order_problem(proof_mode, &proof_venue, "submit_order", error);
        }
        result
    }

    async fn recover_ambiguous_submit_result(
        &self,
        internal_order_id: &str,
        result: TradingResult<OrderRecord>,
    ) -> TradingResult<OrderRecord> {
        let error = match result {
            Ok(record) => return Ok(record),
            Err(error) => error,
        };
        if !ambiguous_submit_result(&error) {
            return Err(error);
        }
        resolve_submit_recovery(
            internal_order_id,
            error,
            self.refresh_order_state(internal_order_id).await,
        )
    }

    pub(crate) async fn submit_unwind_with_ledger_context(
        &self,
        intent: OrderIntent,
        context: ExecutionLedgerOrderContext,
    ) -> TradingResult<OrderRecord> {
        self.journal
            .attach_execution_ledger_context(&intent.id, context);
        self.submit_unwind_inner(intent).await
    }

    pub(super) async fn submit_unwind_inner(
        &self,
        intent: OrderIntent,
    ) -> TradingResult<OrderRecord> {
        ensure_live_order_mutation_audit_trail(intent.mode)?;
        let internal_order_id = intent.id.clone();
        let proof_mode = intent.mode;
        let proof_venue = intent.exchange.clone();
        let result = self.engine.submit_unwind(intent).await;
        let result = self
            .recover_ambiguous_submit_result(&internal_order_id, result)
            .await;
        if let Ok(record) = result.as_ref() {
            self.live_order_proof_health
                .record_submit_ack_from_record(record);
            self.mark_open_order_cache_stale(&record.intent.exchange);
        } else if let Err(error) = result.as_ref() {
            self.record_live_order_problem(proof_mode, &proof_venue, "submit_unwind_order", error);
        }
        result
    }

    pub(crate) async fn cancel(&self, internal_order_id: &str) -> TradingResult<OrderRecord> {
        let existing = self
            .journal
            .get(internal_order_id)
            .ok_or_else(|| trading::TradingError::OrderNotFound(internal_order_id.to_owned()))?;
        ensure_remote_cancel_audit_trail(&existing)?;
        let record = match self.engine.cancel(internal_order_id).await {
            Ok(record) => record,
            Err(error) => {
                self.record_live_order_problem(
                    existing.intent.mode,
                    &existing.intent.exchange,
                    "cancel_order",
                    &error,
                );
                return Err(error);
            }
        };
        self.live_order_proof_health
            .record_cancel_ack_from_record(&record);
        let finality_query_required = record.state == LiveOrderState::CancelRequested;
        self.mark_open_order_cache_stale(&record.intent.exchange);
        let record = self.refresh_pending_cancel_finality(record).await;
        if !finality_query_required && filled_quantity_changes_account_state(record.filled_quantity)
        {
            self.mark_filled_account_cache_stale(
                &record.intent.exchange,
                "cancel_fill_terminal_update",
            );
        }
        Ok(record)
    }

    pub(crate) async fn refresh_pending_cancel_finality(&self, record: OrderRecord) -> OrderRecord {
        if record.state != LiveOrderState::CancelRequested {
            return record;
        }
        match self.refresh_order_state(&record.intent.id).await {
            Ok(Some(updated)) => updated,
            Ok(None) => record,
            Err(error) => {
                tracing::warn!(
                    internal_order_id = %record.intent.id,
                    exchange = %record.intent.exchange,
                    error = %error,
                    "cancel finality refresh failed"
                );
                record
            }
        }
    }

    fn record_live_order_problem(
        &self,
        mode: ExecutionMode,
        venue: &str,
        source: &str,
        error: &trading::TradingError,
    ) {
        if mode != ExecutionMode::Live {
            return;
        }
        let problem = crate::trading_errors::trading_error_problem(error, venue, source);
        let message = problem.message.clone();
        self.live_order_proof_health
            .record_problem(LiveOrderProofProblemInput {
                venue,
                source,
                message: &message,
                request_id: problem.request_id,
                retry_after_ms: problem.retry_after_ms,
                status: problem.status,
            });
    }

    pub(crate) fn check_hedge(
        &self,
        long_leg: &OrderIntent,
        short_leg: &OrderIntent,
    ) -> (RiskDecision, RiskDecision) {
        self.risk
            .check_hedge(long_leg, short_leg, self.open_order_count())
    }

    pub(crate) fn check_hedge_with_open_orders(
        &self,
        long_leg: &OrderIntent,
        short_leg: &OrderIntent,
        open_orders: usize,
    ) -> (RiskDecision, RiskDecision) {
        self.risk.check_hedge(long_leg, short_leg, open_orders)
    }
}
