use super::*;

#[cfg(not(test))]
pub(super) fn read_only_evidence_with_order_permission_scopes(
    probes: Vec<VenueCredentialProbe>,
    scopes: &[VenueCredentialPermission],
) -> VenueCredentialValidationEvidence {
    evidence_with_order_permission_scopes(
        VenueCredentialValidationStatus::ReadOnlyOk,
        probes,
        scopes,
    )
}

#[cfg(not(test))]
pub(super) async fn validated_private_read_evidence<A>(
    venue: &str,
    adapter: &A,
    balance_currency: &str,
    mut probes: Vec<VenueCredentialProbe>,
    order_permission_scopes: &[VenueCredentialPermission],
) -> Result<VenueCredentialValidationEvidence, CredentialUpdateError>
where
    A: exchange::ExchangeAdapter + Sync,
{
    validate_balance(adapter, VALIDATION_TIMEOUT_SECS).await?;
    probes.push(balance_probe(balance_currency));
    probes.extend(optional_private_read_probes(adapter).await);
    if !probes.iter().any(|probe| probe.kind == "order_permission") {
        probes.push(order_permission_unproven_probe(venue));
    }
    Ok(read_only_evidence_with_order_permission_scopes(
        probes,
        order_permission_scopes,
    ))
}

#[cfg(not(test))]
pub(super) async fn optional_private_read_probes<A>(adapter: &A) -> Vec<VenueCredentialProbe>
where
    A: exchange::ExchangeAdapter + Sync,
{
    let positions = optional_exchange_probe(
        "positions_read",
        "private_read.positions",
        "exchange_adapter.get_positions",
        "positions",
        adapter.get_positions(None),
    );
    let open_orders = optional_exchange_probe(
        "open_orders_read",
        "private_read.open_orders",
        "exchange_adapter.get_open_orders",
        "open orders",
        adapter.get_open_orders(None),
    );
    let (positions, open_orders) = tokio::join!(positions, open_orders);
    vec![positions, open_orders]
}

#[cfg(test)]
pub(super) fn evidence(
    status: VenueCredentialValidationStatus,
    probes: Vec<VenueCredentialProbe>,
) -> VenueCredentialValidationEvidence {
    evidence_with_order_permission_scopes(status, probes, &[])
}

pub(super) fn evidence_with_order_permission_scopes(
    status: VenueCredentialValidationStatus,
    mut probes: Vec<VenueCredentialProbe>,
    order_permission_scopes: &[VenueCredentialPermission],
) -> VenueCredentialValidationEvidence {
    let checked_at_ms = common::time::now_ms();
    probes.iter_mut().for_each(|probe| {
        probe.checked_at_ms = checked_at_ms;
    });
    if !probes.iter().any(|probe| probe.kind == "order_permission") {
        probes.push(order_permission_probe(checked_at_ms));
    }
    if !probes.iter().any(|probe| probe.kind == "account_mode_read") {
        probes.push(account_mode_not_probed_probe(checked_at_ms));
    }
    VenueCredentialValidationEvidence {
        status,
        checked_at_ms,
        probes,
        permission_evidence: Vec::new(),
    }
    .with_order_permission_scopes(order_permission_scopes)
}

#[cfg(not(test))]
pub(super) fn balance_probe(currency: &str) -> VenueCredentialProbe {
    probe(
        "balance_read",
        VenueCredentialProbeStatus::Ok,
        currency,
        "exchange_adapter.get_balance",
        "read-only balance probe succeeded",
    )
}

#[cfg(not(test))]
pub(super) async fn optional_exchange_probe<F, T>(
    kind: &str,
    scope: &str,
    source: &str,
    label: &str,
    request: F,
) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<T>>,
{
    match tokio::time::timeout(
        std::time::Duration::from_secs(OPTIONAL_READ_PROBE_TIMEOUT_SECS),
        request,
    )
    .await
    {
        Ok(Ok(_)) => probe(
            kind,
            VenueCredentialProbeStatus::Ok,
            scope,
            source,
            &format!("read-only {label} probe succeeded"),
        ),
        Ok(Err(error)) => {
            let (status, message) = classify_optional_probe_error(label, &error);
            probe(kind, status, scope, source, &message)
        }
        Err(_) => probe(
            kind,
            VenueCredentialProbeStatus::Unknown,
            scope,
            source,
            &format!("read-only {label} probe timed out; save kept balance evidence"),
        ),
    }
}

/// Classify an optional read-probe failure into a fail-closed probe status.
///
/// A definitive credential/permission rejection (auth failure or HTTP 401/403)
/// becomes `Failed` so the save response never presents it as a neutral
/// `Unknown`; transient/unproven outcomes (timeout, rate limit, missing adapter
/// reader, other API errors) stay `Unknown` because a retry may yet prove the
/// scope.
pub(super) fn classify_optional_probe_error(
    label: &str,
    error: &exchange::ExchangeError,
) -> (VenueCredentialProbeStatus, String) {
    match error {
        exchange::ExchangeError::Auth(_) => (
            VenueCredentialProbeStatus::Failed,
            format!("read-only {label} probe failed: venue rejected credentials"),
        ),
        exchange::ExchangeError::Http { status, .. } if *status == 401 || *status == 403 => (
            VenueCredentialProbeStatus::Failed,
            format!("read-only {label} probe failed: venue rejected credentials (http {status})"),
        ),
        exchange::ExchangeError::NotImplemented(_) => (
            VenueCredentialProbeStatus::Unknown,
            format!("read-only {label} probe not proven: adapter does not expose this reader"),
        ),
        exchange::ExchangeError::RateLimited { retry_after_secs } => (
            VenueCredentialProbeStatus::Unknown,
            format!(
                "read-only {label} probe not proven: rate limited; retry after {retry_after_secs}s"
            ),
        ),
        exchange::ExchangeError::Timeout { seconds } => (
            VenueCredentialProbeStatus::Unknown,
            format!("read-only {label} probe not proven: timeout after {seconds}s"),
        ),
        _ => (
            VenueCredentialProbeStatus::Unknown,
            format!("read-only {label} probe not proven: {error}"),
        ),
    }
}

pub(super) fn probe(
    kind: &str,
    status: VenueCredentialProbeStatus,
    scope: &str,
    source: &str,
    message: &str,
) -> VenueCredentialProbe {
    VenueCredentialProbe {
        kind: kind.into(),
        status,
        scope: scope.into(),
        source: source.into(),
        message: message.into(),
        checked_at_ms: 0,
        request_id: common::request_id::current(),
    }
}

#[cfg(test)]
mod tests;
