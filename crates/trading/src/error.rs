use shared_types::RiskBlockReason;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TradingError {
    #[error("risk blocked order: {0:?}")]
    RiskBlocked(Vec<RiskBlockReason>),

    #[error("exchange error: {0}")]
    Exchange(#[from] exchange::ExchangeError),

    #[error("insufficient margin on {exchange}: required {required:.4}, available {available:.4}")]
    InsufficientMargin {
        exchange: String,
        required: f64,
        available: f64,
    },

    #[error("order not found: {0}")]
    OrderNotFound(String),

    #[error("submission already in flight for client_order_id {0}; retry after it settles")]
    SubmissionInFlight(String),

    #[error("live audit trail unavailable: {reason}")]
    AuditLogUnavailable { reason: String },
}

pub type TradingResult<T> = Result<T, TradingError>;
