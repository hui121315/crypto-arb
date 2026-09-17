use super::*;

mod probe_support;

use probe_support::{
    credential_probe_status_name, hyperliquid_account_side_probe, merged_probe_status,
    relation_message, validate_relation,
};

pub(super) type HyperliquidAccountRelation = exchange::HyperliquidCredentialRelation;
pub(super) type HyperliquidAbstractionState = exchange::HyperliquidAccountAbstraction;

/// Convert complete account/signer/vault evidence into a fail-closed probe.
/// A sub-account can be owned by a master account, while the signing agent can
/// be approved by either the selected read account or that master.
pub(super) fn hyperliquid_account_relation_probe(
    checked_at_ms: i64,
    outcome: Option<Result<HyperliquidAccountRelation, exchange::ExchangeError>>,
) -> VenueCredentialProbe {
    let (status, message) = match outcome {
        Some(Ok(relation)) => match validate_relation(&relation) {
            Ok(()) => (VenueCredentialProbeStatus::Ok, relation_message(&relation)),
            Err(message) => (VenueCredentialProbeStatus::Failed, message),
        },
        Some(Err(error)) => classify_optional_probe_error("account signer vault relation", &error),
        None => (
            VenueCredentialProbeStatus::Unknown,
            "account, derived signer, and vault relation not proven by credential save".to_owned(),
        ),
    };
    VenueCredentialProbe {
        kind: "account_signer_vault_relation".into(),
        status,
        scope: "hyperliquid_account_signer_vault".into(),
        source: "userRole(account)+userRole(signer)+vaultDetails".into(),
        message,
        checked_at_ms,
        request_id: common::request_id::current(),
    }
}

#[cfg(not(test))]
pub(super) async fn optional_hyperliquid_account_relation_probe<F>(
    request: F,
) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<HyperliquidAccountRelation>>,
{
    let outcome = optional_outcome(request).await;
    hyperliquid_account_relation_probe(common::time::now_ms(), outcome)
}

pub(super) fn hyperliquid_abstraction_probe(
    checked_at_ms: i64,
    outcome: Option<Result<HyperliquidAbstractionState, exchange::ExchangeError>>,
) -> VenueCredentialProbe {
    let (status, scope, message) = match outcome {
        Some(Ok(state)) => (
            VenueCredentialProbeStatus::Ok,
            state.user_abstraction.clone(),
            format!(
                "account={} userAbstraction={} userDexAbstraction={}",
                state.account_address,
                state.user_abstraction,
                state.user_dex_abstraction.as_deref().unwrap_or("none")
            ),
        ),
        Some(Err(error)) => {
            let (status, message) = classify_optional_probe_error("account abstraction", &error);
            (status, "hyperliquid_account".to_owned(), message)
        }
        None => (
            VenueCredentialProbeStatus::Unknown,
            "hyperliquid_account".to_owned(),
            "userAbstraction and userDexAbstraction were not probed".to_owned(),
        ),
    };
    VenueCredentialProbe {
        kind: "account_abstraction".into(),
        status,
        scope,
        source: "userAbstraction(account)+userDexAbstraction(account)".into(),
        message,
        checked_at_ms,
        request_id: common::request_id::current(),
    }
}

#[cfg(not(test))]
pub(super) async fn optional_hyperliquid_abstraction_probe<F>(request: F) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<HyperliquidAbstractionState>>,
{
    let outcome = optional_outcome(request).await;
    hyperliquid_abstraction_probe(common::time::now_ms(), outcome)
}

/// The generic readiness matrix has one account-mode link. Hyperliquid fills
/// it from the independently visible role and abstraction probes so neither
/// missing relation can accidentally grant live readiness.
pub(super) fn hyperliquid_account_mode_probe(
    relation: &VenueCredentialProbe,
    abstraction: &VenueCredentialProbe,
) -> VenueCredentialProbe {
    let status = merged_probe_status(relation.status, abstraction.status);
    let scope = if abstraction.status == VenueCredentialProbeStatus::Ok {
        abstraction.scope.as_str()
    } else {
        "hyperliquid_account_role_abstraction"
    };
    probe(
        "account_mode_read",
        status,
        scope,
        "userRole(account)+userRole(signer)+userAbstraction(account)+userDexAbstraction(account)",
        &format!(
            "role relation={}；abstraction={}",
            credential_probe_status_name(relation.status),
            credential_probe_status_name(abstraction.status)
        ),
    )
}

/// A successful signed action proves permission only for the account relation
/// verified by `userRole`; an agent owned by another account must stay blocked.
pub(super) fn hyperliquid_order_permission_probe(
    relation: &VenueCredentialProbe,
    mut signed_action: VenueCredentialProbe,
) -> VenueCredentialProbe {
    if signed_action.status == VenueCredentialProbeStatus::Ok
        && relation.status != VenueCredentialProbeStatus::Ok
    {
        signed_action.status = relation.status;
        signed_action.message = format!(
            "official noop exchange action succeeded, but permission for the configured account is not proven because account/signer relation={}",
            credential_probe_status_name(relation.status)
        );
    }
    signed_action
}

/// Build aggregate and per-source balance probes from a partial account read.
/// A failed source remains visible without hiding rows from the successful one.
pub(super) fn hyperliquid_account_read_probes(
    checked_at_ms: i64,
    outcome: Option<Result<exchange::VenueAccountRead, exchange::ExchangeError>>,
) -> Vec<VenueCredentialProbe> {
    let (perp, spot) = match outcome {
        Some(Ok(read)) => {
            let perp_rows = read
                .balances
                .iter()
                .filter(|row| row.venue != "hyperliquid:spot")
                .count();
            let spot_rows = read
                .balances
                .iter()
                .filter(|row| row.venue == "hyperliquid:spot")
                .count();
            let perp_errors = read
                .issues
                .iter()
                .filter(|issue| issue.operation == "perp_margin")
                .map(|issue| &issue.error)
                .collect::<Vec<_>>();
            let spot_errors = read
                .issues
                .iter()
                .filter(|issue| issue.operation == "spot_truth")
                .map(|issue| &issue.error)
                .collect::<Vec<_>>();
            (
                hyperliquid_account_side_probe(
                    "perp_margin_read",
                    "perp_margin.USDC",
                    "hyperliquid.POST /info type=clearinghouseState",
                    "perp margin",
                    perp_rows,
                    &perp_errors,
                ),
                hyperliquid_account_side_probe(
                    "spot_truth_read",
                    "spot_truth.USDC",
                    "hyperliquid.POST /info type=spotClearinghouseState",
                    "spot truth",
                    spot_rows,
                    &spot_errors,
                ),
            )
        }
        Some(Err(error)) => {
            let (status, message) = classify_optional_probe_error("account balance", &error);
            (
                probe(
                    "perp_margin_read",
                    status,
                    "perp_margin.USDC",
                    "hyperliquid.POST /info type=clearinghouseState",
                    &message,
                ),
                probe(
                    "spot_truth_read",
                    status,
                    "spot_truth.USDC",
                    "hyperliquid.POST /info type=spotClearinghouseState",
                    &message,
                ),
            )
        }
        None => (
            probe(
                "perp_margin_read",
                VenueCredentialProbeStatus::Unknown,
                "perp_margin.USDC",
                "not_probed",
                "perp margin truth was not probed",
            ),
            probe(
                "spot_truth_read",
                VenueCredentialProbeStatus::Unknown,
                "spot_truth.USDC",
                "not_probed",
                "spot truth was not probed",
            ),
        ),
    };
    let balance = VenueCredentialProbe {
        kind: "balance_read".into(),
        status: merged_probe_status(perp.status, spot.status),
        scope: "hyperliquid_spot_truth_and_perp_margin".into(),
        source: "clearinghouseState(account)+spotClearinghouseState(account)".into(),
        message: format!(
            "perp_margin={}；spot_truth={}; each source is retained independently",
            credential_probe_status_name(perp.status),
            credential_probe_status_name(spot.status)
        ),
        checked_at_ms,
        request_id: common::request_id::current(),
    };
    vec![balance, perp, spot]
}

#[cfg(not(test))]
pub(super) async fn optional_hyperliquid_account_read_probes<F>(
    request: F,
) -> Vec<VenueCredentialProbe>
where
    F: std::future::Future<Output = exchange::ExchangeResult<exchange::VenueAccountRead>>,
{
    let outcome = optional_outcome(request).await;
    hyperliquid_account_read_probes(common::time::now_ms(), outcome)
}

#[cfg(not(test))]
async fn optional_outcome<F, T>(request: F) -> Option<Result<T, exchange::ExchangeError>>
where
    F: std::future::Future<Output = exchange::ExchangeResult<T>>,
{
    match tokio::time::timeout(
        std::time::Duration::from_secs(OPTIONAL_READ_PROBE_TIMEOUT_SECS),
        request,
    )
    .await
    {
        Ok(result) => Some(result),
        Err(_) => Some(Err(exchange::ExchangeError::Timeout {
            seconds: OPTIONAL_READ_PROBE_TIMEOUT_SECS,
        })),
    }
}
