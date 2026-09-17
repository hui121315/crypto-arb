use super::*;

pub(super) fn hyperliquid_account_side_probe(
    kind: &str,
    scope: &str,
    source: &str,
    label: &str,
    rows: usize,
    errors: &[&exchange::ExchangeError],
) -> VenueCredentialProbe {
    if errors.is_empty() {
        return probe(
            kind,
            VenueCredentialProbeStatus::Ok,
            scope,
            source,
            &format!("read-only {label} probe succeeded; rows={rows}"),
        );
    }
    let (status, message) = errors.iter().fold(
        (
            VenueCredentialProbeStatus::Unknown,
            format!("read-only {label} probe not proven"),
        ),
        |(status, message), error| {
            let (next_status, next_message) = classify_optional_probe_error(label, error);
            (
                merged_probe_status(status, next_status),
                format!("{message}; {next_message}"),
            )
        },
    );
    probe(kind, status, scope, source, &message)
}

pub(super) fn merged_probe_status(
    left: VenueCredentialProbeStatus,
    right: VenueCredentialProbeStatus,
) -> VenueCredentialProbeStatus {
    if matches!(left, VenueCredentialProbeStatus::Failed)
        || matches!(right, VenueCredentialProbeStatus::Failed)
    {
        VenueCredentialProbeStatus::Failed
    } else if matches!(left, VenueCredentialProbeStatus::Unknown)
        || matches!(right, VenueCredentialProbeStatus::Unknown)
    {
        VenueCredentialProbeStatus::Unknown
    } else {
        VenueCredentialProbeStatus::Ok
    }
}

pub(super) fn credential_probe_status_name(status: VenueCredentialProbeStatus) -> &'static str {
    match status {
        VenueCredentialProbeStatus::Ok => "verified",
        VenueCredentialProbeStatus::Failed => "failed",
        VenueCredentialProbeStatus::Unknown => "unknown",
    }
}

pub(super) fn validate_relation(relation: &HyperliquidAccountRelation) -> Result<(), String> {
    if !is_evm_address(&relation.main_account) {
        return Err("configured Hyperliquid read account is not a valid EVM address".into());
    }
    if !is_evm_address(&relation.signer) {
        return Err("derived Hyperliquid signer is not a valid EVM address".into());
    }
    if !matches!(relation.main_account_role.as_str(), "user" | "subAccount") {
        return Err("configured Hyperliquid read address is neither a main nor sub-account".into());
    }
    let signer_is_direct = addresses_equal(&relation.signer, &relation.main_account)
        && relation.signer_role == relation.main_account_role;
    let signer_is_agent = relation.signer_role == "agent"
        && relation.signer_owner.as_deref().is_some_and(|owner| {
            addresses_equal(owner, &relation.main_account)
                || relation
                    .main_account_owner
                    .as_deref()
                    .is_some_and(|master| addresses_equal(owner, master))
        });
    if !signer_is_direct && !signer_is_agent {
        return Err(match relation.signer_role.as_str() {
            "missing" => format!(
                "derived signer {} is not registered; approve it as an API/Agent Wallet for the configured Hyperliquid account, then save credentials again",
                relation.signer
            ),
            "agent" => format!(
                "derived API/Agent Wallet {} is approved for a different Hyperliquid account",
                relation.signer
            ),
            _ => format!(
                "private key resolves to {} ({}) instead of the configured Hyperliquid account or its approved API/Agent Wallet",
                relation.signer, relation.signer_role
            ),
        });
    }
    match (
        &relation.vault_address,
        &relation.vault_role,
        &relation.vault_leader,
    ) {
        (None, None, None) => Ok(()),
        (Some(vault), Some(role), Some(leader))
            if is_evm_address(vault)
                && role == "vault"
                && is_evm_address(leader)
                && (addresses_equal(leader, &relation.main_account)
                    || relation
                        .main_account_owner
                        .as_deref()
                        .is_some_and(|master| addresses_equal(leader, master))) =>
        {
            Ok(())
        }
        (Some(_), Some(_), Some(_)) => {
            Err("configured Hyperliquid vault is not led by the selected account owner".into())
        }
        (Some(_), _, _) => {
            Err("configured Hyperliquid vault has no verified vaultDetails leader evidence".into())
        }
        (None, Some(_), _) | (None, _, Some(_)) => {
            Err("unexpected Hyperliquid vault leader without a vault address".into())
        }
    }
}

pub(super) fn relation_message(relation: &HyperliquidAccountRelation) -> String {
    format!(
        "account={} role={} owner={}; derived signer={} role={} owner={}; vault={} role={} leader={}",
        relation.main_account,
        relation.main_account_role,
        relation.main_account_owner.as_deref().unwrap_or("self"),
        relation.signer,
        relation.signer_role,
        relation.signer_owner.as_deref().unwrap_or("self"),
        relation.vault_address.as_deref().unwrap_or("none"),
        relation.vault_role.as_deref().unwrap_or("none"),
        relation.vault_leader.as_deref().unwrap_or("none"),
    )
}

fn is_evm_address(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() == 42
        && trimmed.starts_with("0x")
        && trimmed[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn addresses_equal(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}
