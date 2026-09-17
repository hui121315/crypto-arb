use crate::api::rest::ApiError;
use shared_types::{problem::codes, KillSwitchRequest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KillSwitchReplaySlot {
    pub(crate) fingerprint: String,
    pub(crate) key: String,
}

pub(crate) fn replay_slot(
    existing: Option<KillSwitchReplaySlot>,
    request: &KillSwitchRequest,
) -> KillSwitchReplaySlot {
    let fingerprint = request_fingerprint(request);
    KillSwitchReplaySlot {
        key: replay_key(existing, &fingerprint),
        fingerprint,
    }
}

pub(crate) fn should_reuse_replay_key(error: &ApiError) -> bool {
    matches!(
        error.problem.code.as_str(),
        "NETWORK" | "TIMEOUT" | codes::ACTION_RUN_IN_FLIGHT
    )
}

fn replay_key(existing: Option<KillSwitchReplaySlot>, fingerprint: &str) -> String {
    existing
        .filter(|slot| slot.fingerprint == fingerprint)
        .map_or_else(next_replay_key, |slot| slot.key)
}

fn request_fingerprint(request: &KillSwitchRequest) -> String {
    format!(
        "active={};expected_active={};expected_open_order_count={};reason={}",
        request.active,
        option_bool_key(request.expected_active),
        option_usize_key(request.expected_open_order_count),
        request.reason.trim()
    )
}

fn option_bool_key(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "-",
    }
}

fn option_usize_key(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn next_replay_key() -> String {
    format!("kill-switch-{}", replay_key_entropy())
}

#[cfg(target_arch = "wasm32")]
fn replay_key_entropy() -> String {
    let now = js_sys::Date::now().to_bits();
    let random = js_sys::Math::random().to_bits();
    format!("{now:016x}{random:016x}")
}

#[cfg(not(target_arch = "wasm32"))]
fn replay_key_entropy() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{sequence:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_slot_reuses_only_same_request_fingerprint() {
        let request = kill_switch_request(true, Some(false), Some(0), " reason ");
        let first = replay_slot(None, &request);
        let same = replay_slot(
            Some(first.clone()),
            &kill_switch_request(true, Some(false), Some(0), "reason"),
        );
        let different = replay_slot(
            Some(first.clone()),
            &kill_switch_request(false, Some(true), Some(0), "reason"),
        );

        assert_eq!(same.key, first.key);
        assert_ne!(different.key, first.key);
    }

    #[test]
    fn replay_key_kept_only_for_transport_or_in_flight_errors() {
        assert!(should_reuse_replay_key(&ApiError::client(
            "NETWORK", "down"
        )));
        assert!(should_reuse_replay_key(&ApiError::client(
            "TIMEOUT", "slow"
        )));
        assert!(should_reuse_replay_key(&ApiError::client(
            codes::ACTION_RUN_IN_FLIGHT,
            "busy",
        )));
        assert!(!should_reuse_replay_key(&ApiError::client(
            codes::ACTION_RUN_REPLAY_FAILED,
            "failed",
        )));
    }

    fn kill_switch_request(
        active: bool,
        expected_active: Option<bool>,
        expected_open_order_count: Option<usize>,
        reason: &str,
    ) -> KillSwitchRequest {
        KillSwitchRequest {
            active,
            expected_active,
            expected_open_order_count,
            reason: reason.to_owned(),
        }
    }
}
