use super::*;

pub(super) fn ambiguous_submit_result(error: &TradingError) -> bool {
    let TradingError::Exchange(error) = error else {
        return false;
    };
    matches!(
        error,
        exchange::ExchangeError::Timeout { .. }
            | exchange::ExchangeError::Network(_)
            | exchange::ExchangeError::WsClosed(_)
            | exchange::ExchangeError::Parse(_)
            | exchange::ExchangeError::Http {
                status: 500..=599,
                ..
            }
    )
}

pub(super) fn resolve_submit_recovery(
    internal_order_id: &str,
    submit_error: TradingError,
    query_result: TradingResult<Option<OrderRecord>>,
) -> TradingResult<OrderRecord> {
    match query_result {
        Ok(Some(record)) => Ok(recovered_submit_record(internal_order_id, record)),
        Ok(None) => Err(submit_error),
        Err(query_error) => {
            log_unconfirmed_submit(internal_order_id, &query_error);
            Err(submit_error)
        }
    }
}

fn recovered_submit_record(internal_order_id: &str, record: OrderRecord) -> OrderRecord {
    tracing::info!(
        internal_order_id,
        exchange = %record.intent.exchange,
        state = ?record.state,
        "ambiguous order submission recovered by client-order-id query"
    );
    record
}

fn log_unconfirmed_submit(internal_order_id: &str, query_error: &TradingError) {
    tracing::warn!(
        internal_order_id,
        error = %query_error,
        "ambiguous order submission could not be confirmed by read-side query"
    );
}
