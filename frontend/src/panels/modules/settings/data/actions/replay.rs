//! Stable replay identity for settings mutations.

use crate::api::rest::ApiError;
use shared_types::{problem::codes, RiskConfigPatch};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::panels::modules::settings) struct CredentialSaveReplay {
    pub(in crate::panels::modules::settings) fingerprint: String,
    pub(in crate::panels::modules::settings) key: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::panels::modules::settings) struct RiskConfigReplay {
    pub(in crate::panels::modules::settings) patch: RiskConfigPatch,
    pub(in crate::panels::modules::settings) key: String,
}

pub(in crate::panels::modules::settings) fn risk_config_replay_slot(
    existing: Option<RiskConfigReplay>,
    patch: &RiskConfigPatch,
) -> RiskConfigReplay {
    existing
        .filter(|slot| slot.patch == *patch)
        .unwrap_or_else(|| RiskConfigReplay {
            patch: patch.clone(),
            key: format!("settings-risk-config-{}", credential_key_entropy()),
        })
}

pub(in crate::panels::modules::settings) fn credential_save_replay_key(
    existing: Option<CredentialSaveReplay>,
    fingerprint: &str,
) -> String {
    existing
        .filter(|slot| slot.fingerprint == fingerprint)
        .map_or_else(next_credential_save_key, |slot| slot.key)
}

pub(in crate::panels::modules::settings) fn credential_save_fingerprint(
    venue: &str,
    fields: &[(String, String)],
) -> String {
    let mut fields = fields
        .iter()
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect::<Vec<_>>();
    fields.sort_unstable_by(|left, right| left.0.cmp(right.0).then_with(|| left.1.cmp(right.1)));
    let mut text = venue.trim().to_ascii_lowercase();
    for (key, value) in fields {
        text.push('\n');
        text.push_str(key);
        text.push('=');
        text.push_str(value);
    }
    text
}

pub(in crate::panels::modules::settings) fn should_reuse_credential_replay_key(
    error: &ApiError,
) -> bool {
    matches!(
        error.problem.code.as_str(),
        "NETWORK" | "TIMEOUT" | codes::ACTION_RUN_IN_FLIGHT
    )
}

fn next_credential_save_key() -> String {
    format!("settings-credentials-{}", credential_key_entropy())
}

#[cfg(target_arch = "wasm32")]
fn credential_key_entropy() -> String {
    let now = js_sys::Date::now().to_bits();
    let random = js_sys::Math::random().to_bits();
    format!("{now:016x}{random:016x}")
}

#[cfg(not(target_arch = "wasm32"))]
fn credential_key_entropy() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{sequence:016x}")
}
