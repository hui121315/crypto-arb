use shared_types::{LiveOrderState, OrderLifecycleEvent};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OrderTransitionError {
    #[error("illegal order transition from {from:?} with event {event:?}")]
    Illegal {
        from: LiveOrderState,
        event: OrderLifecycleEvent,
    },
}

pub fn transition(
    current: LiveOrderState,
    event: OrderLifecycleEvent,
) -> Result<LiveOrderState, OrderTransitionError> {
    Ok(match (current, event) {
        (LiveOrderState::Created, OrderLifecycleEvent::RiskApproved) => LiveOrderState::RiskChecked,
        (LiveOrderState::Created, OrderLifecycleEvent::RiskRejected) => LiveOrderState::Rejected,
        (LiveOrderState::RiskChecked, OrderLifecycleEvent::Submitted) => LiveOrderState::Submitted,
        (LiveOrderState::RiskChecked, OrderLifecycleEvent::AdapterFailed) => LiveOrderState::Failed,
        (LiveOrderState::Submitted, OrderLifecycleEvent::AdapterAccepted) => {
            LiveOrderState::Accepted
        }
        // Private WS can publish OPEN/PARTIALLY_FILLED/FILLED before the request-response ACK
        // reaches the journal. Merging that late identity ACK must never regress finality.
        (
            state @ (LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::Filled),
            OrderLifecycleEvent::AdapterAccepted,
        ) => state,
        (LiveOrderState::Submitted, OrderLifecycleEvent::AdapterRejected) => {
            LiveOrderState::Rejected
        }
        (LiveOrderState::Submitted, OrderLifecycleEvent::AdapterFailed) => LiveOrderState::Failed,
        (LiveOrderState::Submitted, OrderLifecycleEvent::PartialFill) => {
            LiveOrderState::PartiallyFilled
        }
        (LiveOrderState::Submitted, OrderLifecycleEvent::FullFill) => LiveOrderState::Filled,
        (LiveOrderState::Accepted, OrderLifecycleEvent::PartialFill) => {
            LiveOrderState::PartiallyFilled
        }
        (LiveOrderState::PartiallyFilled, OrderLifecycleEvent::PartialFill) => {
            LiveOrderState::PartiallyFilled
        }
        (
            LiveOrderState::Accepted | LiveOrderState::PartiallyFilled,
            OrderLifecycleEvent::FullFill,
        ) => LiveOrderState::Filled,
        (
            LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::Submitted
            | LiveOrderState::Unknown,
            OrderLifecycleEvent::CancelRequest,
        ) => LiveOrderState::CancelRequested,
        (LiveOrderState::CancelRequested, OrderLifecycleEvent::CancelAck) => {
            LiveOrderState::Cancelled
        }
        // 撤单挂起期间交易所仍可能推送部分成交（cancel 与 fill 竞态）：保持
        // 撤单挂起语义、仅让调用方更新成交字段。缺此转移时 transition 报
        // Illegal，record_fill_snapshot 被短路，撤单窗口内的已成交数量在本地
        // 完全不可见（对只推 order-update 的 venue 尤甚）。
        (LiveOrderState::CancelRequested, OrderLifecycleEvent::PartialFill) => {
            LiveOrderState::CancelRequested
        }
        (LiveOrderState::CancelRequested, OrderLifecycleEvent::FullFill) => LiveOrderState::Filled,
        (
            LiveOrderState::Submitted
            | LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::Unknown,
            OrderLifecycleEvent::CancelAck,
        ) => LiveOrderState::Cancelled,
        (
            LiveOrderState::Accepted | LiveOrderState::PartiallyFilled | LiveOrderState::Unknown,
            OrderLifecycleEvent::AdapterRejected,
        ) => LiveOrderState::Rejected,
        (
            LiveOrderState::Accepted | LiveOrderState::PartiallyFilled | LiveOrderState::Unknown,
            OrderLifecycleEvent::AdapterFailed,
        ) => LiveOrderState::Failed,
        (LiveOrderState::Unknown, OrderLifecycleEvent::AdapterAccepted) => LiveOrderState::Accepted,
        (LiveOrderState::Unknown, OrderLifecycleEvent::PartialFill) => {
            LiveOrderState::PartiallyFilled
        }
        (LiveOrderState::Unknown, OrderLifecycleEvent::FullFill) => LiveOrderState::Filled,
        (
            LiveOrderState::Submitted
            | LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::CancelRequested,
            OrderLifecycleEvent::Timeout,
        ) => LiveOrderState::Unknown,
        (any, OrderLifecycleEvent::ExchangeUnknown) if !is_terminal(any) => LiveOrderState::Unknown,
        _ => {
            return Err(OrderTransitionError::Illegal {
                from: current,
                event,
            })
        }
    })
}

pub fn is_terminal(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{LiveOrderState as S, OrderLifecycleEvent as E};

    #[test]
    fn covers_happy_path_to_accepted() {
        assert_eq!(transition(S::Created, E::RiskApproved), Ok(S::RiskChecked));
        assert_eq!(transition(S::RiskChecked, E::Submitted), Ok(S::Submitted));
        assert_eq!(
            transition(S::Submitted, E::AdapterAccepted),
            Ok(S::Accepted)
        );
    }

    #[test]
    fn late_adapter_ack_preserves_private_stream_finality() {
        assert_eq!(transition(S::Accepted, E::AdapterAccepted), Ok(S::Accepted));
        assert_eq!(
            transition(S::PartiallyFilled, E::AdapterAccepted),
            Ok(S::PartiallyFilled)
        );
        assert_eq!(transition(S::Filled, E::AdapterAccepted), Ok(S::Filled));
    }

    #[test]
    fn covers_rejections_and_failures() {
        assert_eq!(transition(S::Created, E::RiskRejected), Ok(S::Rejected));
        assert_eq!(
            transition(S::Submitted, E::AdapterRejected),
            Ok(S::Rejected)
        );
        assert_eq!(transition(S::RiskChecked, E::AdapterFailed), Ok(S::Failed));
        assert_eq!(transition(S::Submitted, E::AdapterFailed), Ok(S::Failed));
    }

    #[test]
    fn covers_fill_lifecycle() {
        assert_eq!(
            transition(S::Submitted, E::PartialFill),
            Ok(S::PartiallyFilled)
        );
        assert_eq!(
            transition(S::Accepted, E::PartialFill),
            Ok(S::PartiallyFilled)
        );
        assert_eq!(
            transition(S::PartiallyFilled, E::PartialFill),
            Ok(S::PartiallyFilled)
        );
        assert_eq!(transition(S::Accepted, E::FullFill), Ok(S::Filled));
        assert_eq!(transition(S::PartiallyFilled, E::FullFill), Ok(S::Filled));
        assert_eq!(transition(S::Submitted, E::FullFill), Ok(S::Filled));
    }

    #[test]
    fn covers_cancel_lifecycle() {
        assert_eq!(
            transition(S::Submitted, E::CancelRequest),
            Ok(S::CancelRequested)
        );
        assert_eq!(
            transition(S::Accepted, E::CancelRequest),
            Ok(S::CancelRequested)
        );
        assert_eq!(
            transition(S::PartiallyFilled, E::CancelRequest),
            Ok(S::CancelRequested)
        );
        assert_eq!(
            transition(S::Unknown, E::CancelRequest),
            Ok(S::CancelRequested)
        );
        assert_eq!(
            transition(S::CancelRequested, E::CancelAck),
            Ok(S::Cancelled)
        );
        assert_eq!(transition(S::CancelRequested, E::FullFill), Ok(S::Filled));
        // 撤单窗口内的部分成交保持挂起态（成交字段由调用方更新），不得判 Illegal。
        assert_eq!(
            transition(S::CancelRequested, E::PartialFill),
            Ok(S::CancelRequested)
        );
    }

    #[test]
    fn covers_exchange_unknown_for_non_terminal_states() {
        assert_eq!(transition(S::Submitted, E::ExchangeUnknown), Ok(S::Unknown));
        assert_eq!(transition(S::Accepted, E::ExchangeUnknown), Ok(S::Unknown));
        assert_eq!(
            transition(S::CancelRequested, E::ExchangeUnknown),
            Ok(S::Unknown)
        );
    }

    #[test]
    fn covers_timeout_as_unknown_for_open_states() {
        assert_eq!(transition(S::Submitted, E::Timeout), Ok(S::Unknown));
        assert_eq!(transition(S::Accepted, E::Timeout), Ok(S::Unknown));
        assert_eq!(transition(S::PartiallyFilled, E::Timeout), Ok(S::Unknown));
        assert_eq!(transition(S::CancelRequested, E::Timeout), Ok(S::Unknown));
        assert!(transition(S::Filled, E::Timeout).is_err());
    }

    #[test]
    fn covers_remote_terminal_backfill_for_open_states() {
        assert_eq!(transition(S::Accepted, E::CancelAck), Ok(S::Cancelled));
        assert_eq!(
            transition(S::PartiallyFilled, E::CancelAck),
            Ok(S::Cancelled)
        );
        assert_eq!(transition(S::Unknown, E::AdapterRejected), Ok(S::Rejected));
        assert_eq!(transition(S::Unknown, E::AdapterFailed), Ok(S::Failed));
        assert_eq!(transition(S::Unknown, E::FullFill), Ok(S::Filled));
    }

    #[test]
    fn rejects_illegal_transitions() {
        assert!(transition(S::Created, E::Submitted).is_err());
        assert!(transition(S::Filled, E::CancelRequest).is_err());
        assert!(transition(S::Cancelled, E::ExchangeUnknown).is_err());
    }
}
