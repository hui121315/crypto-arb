use crate::trading_service::private_ws_events::PrivateAccountDirty;
use shared_types::VenueOperationStatus;

#[derive(Debug)]
pub(super) struct PrivateWsRuntimeUpdate {
    pub(super) operation: &'static str,
    pub(super) status: VenueOperationStatus,
    pub(super) message: String,
    pub(super) request_id: Option<String>,
    pub(super) requested: Option<u64>,
    pub(super) rows: Option<u64>,
    pub(super) retry_after_ms: Option<u64>,
    pub(super) error: Option<String>,
    pub(super) account_dirty: Option<PrivateAccountDirty>,
}

impl PrivateWsRuntimeUpdate {
    pub(super) fn new(
        operation: &'static str,
        status: VenueOperationStatus,
        message: impl Into<String>,
    ) -> Self {
        let message = message.into();
        Self {
            operation,
            status,
            error: runtime_error(status, &message),
            message,
            request_id: common::request_id::current(),
            requested: None,
            rows: None,
            retry_after_ms: None,
            account_dirty: None,
        }
    }

    pub(super) fn ok(operation: &'static str, message: impl Into<String>) -> Self {
        Self::new(operation, VenueOperationStatus::Ok, message)
    }

    pub(super) fn warn(operation: &'static str, message: impl Into<String>) -> Self {
        Self::new(operation, VenueOperationStatus::Warn, message)
    }

    pub(super) fn blocked(operation: &'static str, message: impl Into<String>) -> Self {
        Self::new(operation, VenueOperationStatus::Blocked, message)
    }

    pub(super) fn with_requested(mut self, requested: usize) -> Self {
        self.requested = Some(requested as u64);
        self
    }

    pub(super) fn with_rows(mut self, rows: usize) -> Self {
        self.rows = Some(rows as u64);
        self
    }

    pub(super) fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub(super) fn with_request_id(mut self, request_id: Option<&str>) -> Self {
        if let Some(request_id) = request_id {
            self.request_id = Some(request_id.to_owned());
        }
        self
    }

    pub(super) fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub(super) fn with_account_dirty(mut self, dirty: PrivateAccountDirty) -> Self {
        self.account_dirty = Some(dirty);
        self
    }
}

fn runtime_error(status: VenueOperationStatus, message: &str) -> Option<String> {
    matches!(
        status,
        VenueOperationStatus::Blocked | VenueOperationStatus::Unsupported
    )
    .then(|| message.to_owned())
}
