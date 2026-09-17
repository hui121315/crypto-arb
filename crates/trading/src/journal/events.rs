use shared_types::{LiveOrderState, OrderEventRecord, OrderLifecycleEvent};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub(super) fn append_jsonl(path: &PathBuf, event: &OrderEventRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub(super) fn event_from_ack_state(state: LiveOrderState) -> OrderLifecycleEvent {
    match state {
        LiveOrderState::Accepted => OrderLifecycleEvent::AdapterAccepted,
        LiveOrderState::PartiallyFilled => OrderLifecycleEvent::PartialFill,
        LiveOrderState::Filled => OrderLifecycleEvent::FullFill,
        LiveOrderState::Cancelled => OrderLifecycleEvent::CancelAck,
        LiveOrderState::Rejected => OrderLifecycleEvent::AdapterRejected,
        LiveOrderState::Failed => OrderLifecycleEvent::AdapterFailed,
        LiveOrderState::Unknown => OrderLifecycleEvent::ExchangeUnknown,
        LiveOrderState::CancelRequested => OrderLifecycleEvent::CancelRequest,
        LiveOrderState::Created | LiveOrderState::RiskChecked | LiveOrderState::Submitted => {
            OrderLifecycleEvent::ExchangeUnknown
        }
    }
}

pub(super) fn event_from_target_state(state: LiveOrderState) -> Option<OrderLifecycleEvent> {
    Some(match state {
        LiveOrderState::RiskChecked => OrderLifecycleEvent::RiskApproved,
        LiveOrderState::Submitted => OrderLifecycleEvent::Submitted,
        LiveOrderState::Accepted => OrderLifecycleEvent::AdapterAccepted,
        LiveOrderState::PartiallyFilled => OrderLifecycleEvent::PartialFill,
        LiveOrderState::Filled => OrderLifecycleEvent::FullFill,
        LiveOrderState::CancelRequested => OrderLifecycleEvent::CancelRequest,
        LiveOrderState::Cancelled => OrderLifecycleEvent::CancelAck,
        LiveOrderState::Rejected => OrderLifecycleEvent::AdapterRejected,
        LiveOrderState::Failed => OrderLifecycleEvent::AdapterFailed,
        LiveOrderState::Unknown => OrderLifecycleEvent::ExchangeUnknown,
        LiveOrderState::Created => return None,
    })
}
