use super::safe_order_permission_error::{
    api_error_looks_like_permission_denial, http_body_looks_like_permission_denial,
    hyperliquid_signer_is_not_registered,
};
use super::*;

const SAFE_ORDER_LIVE_WRITE_BOUNDARY: &str = "does not grant live-write readiness";

#[cfg(not(test))]
pub(super) async fn optional_safe_order_place_cancel_test_probe<F>(
    scope: &str,
    source: &str,
    request: F,
) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<()>>,
{
    match safe_order_timeout(request).await {
        Ok(Ok(())) => safe_order_place_cancel_test_probe(scope, source),
        Ok(Err(error)) => {
            let (status, message) = classify_safe_order_place_cancel_test_error(&error);
            probe("order_permission", status, scope, source, &message)
        }
        Err(_) => safe_order_unknown_probe(
            scope,
            source,
            "safe non-matching order-test/cancel probe timed out; private order stream and order finality remain unproven",
        ),
    }
}

#[cfg(not(test))]
pub(super) async fn optional_safe_order_pre_check_probe<F>(
    scope: &str,
    source: &str,
    request: F,
) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<()>>,
{
    match safe_order_timeout(request).await {
        Ok(Ok(())) => safe_order_pre_check_probe(scope, source),
        Ok(Err(error)) => {
            let (status, message) = classify_safe_order_pre_check_error(&error);
            probe("order_permission", status, scope, source, &message)
        }
        Err(_) => safe_order_unknown_probe(
            scope,
            source,
            "safe order pre-check probe timed out; cancel permission, private order stream, and order finality remain unproven",
        ),
    }
}

#[cfg(not(test))]
pub(super) async fn optional_safe_order_cancel_no_match_probe<F>(
    scope: &str,
    source: &str,
    request: F,
) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<()>>,
{
    match safe_order_timeout(request).await {
        Ok(Ok(())) => safe_order_cancel_no_match_probe(scope, source),
        Ok(Err(error)) => {
            let (status, message) = classify_safe_order_cancel_no_match_error(&error);
            probe("order_permission", status, scope, source, &message)
        }
        Err(_) => safe_order_unknown_probe(
            scope,
            source,
            "safe non-matching cancel probe timed out; place permission, private order stream, and order finality remain unproven",
        ),
    }
}

#[cfg(not(test))]
pub(super) async fn optional_safe_order_noop_probe<F>(
    scope: &str,
    source: &str,
    request: F,
) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<()>>,
{
    match safe_order_timeout(request).await {
        Ok(Ok(())) => safe_order_noop_probe(scope, source),
        Ok(Err(error)) => {
            let (status, message) = classify_safe_order_noop_error(&error);
            probe("order_permission", status, scope, source, &message)
        }
        Err(_) => safe_order_unknown_probe(
            scope,
            source,
            "safe noop action probe timed out; exchange-action signing permission remains unproven",
        ),
    }
}

#[cfg(not(test))]
async fn safe_order_timeout<F>(
    request: F,
) -> Result<exchange::ExchangeResult<()>, tokio::time::error::Elapsed>
where
    F: std::future::Future<Output = exchange::ExchangeResult<()>>,
{
    tokio::time::timeout(
        std::time::Duration::from_secs(OPTIONAL_READ_PROBE_TIMEOUT_SECS),
        request,
    )
    .await
}

#[cfg(test)]
pub(super) fn classify_safe_order_place_test_error(
    error: &exchange::ExchangeError,
) -> (VenueCredentialProbeStatus, String) {
    classify_safe_order_probe_error(error, "order-test", "order-test permission")
}

pub(super) fn classify_safe_order_place_cancel_test_error(
    error: &exchange::ExchangeError,
) -> (VenueCredentialProbeStatus, String) {
    classify_safe_order_probe_error(
        error,
        "order-test/cancel",
        "order-test or cancel permission",
    )
}

pub(super) fn classify_safe_order_pre_check_error(
    error: &exchange::ExchangeError,
) -> (VenueCredentialProbeStatus, String) {
    classify_safe_order_probe_error(error, "order pre-check", "pre-check permission")
}

pub(super) fn classify_safe_order_cancel_no_match_error(
    error: &exchange::ExchangeError,
) -> (VenueCredentialProbeStatus, String) {
    classify_safe_order_probe_error(error, "cancel-no-match", "cancel permission")
}

pub(super) fn classify_safe_order_noop_error(
    error: &exchange::ExchangeError,
) -> (VenueCredentialProbeStatus, String) {
    if hyperliquid_signer_is_not_registered(error) {
        return (
            VenueCredentialProbeStatus::Failed,
            "safe noop action probe failed: Hyperliquid API/Agent Wallet is not approved for the configured account; approve the derived signer, then save credentials again".to_owned(),
        );
    }
    classify_safe_order_probe_error(error, "noop action", "signed action permission")
}

fn classify_safe_order_probe_error(
    error: &exchange::ExchangeError,
    probe_label: &str,
    permission_label: &str,
) -> (VenueCredentialProbeStatus, String) {
    match error {
        exchange::ExchangeError::Auth(_) => (
            VenueCredentialProbeStatus::Failed,
            format!(
                "safe non-matching {probe_label} probe failed: venue rejected credentials or {permission_label}"
            ),
        ),
        exchange::ExchangeError::Http { status, body }
            if *status == 401
                || *status == 403
                || http_body_looks_like_permission_denial(body) =>
        {
            (
                VenueCredentialProbeStatus::Failed,
                format!(
                    "safe non-matching {probe_label} probe failed: venue rejected credentials or {permission_label} (http {status})"
                ),
            )
        }
        exchange::ExchangeError::Api { code, message, .. }
            if code == "api_trading_disabled"
                || code == "safe_cancel_collision"
                || code == "unified_account_api_ordering_unsupported"
                || code == "unknown_account_type"
                || api_error_looks_like_permission_denial(code, message) =>
        {
            (
                VenueCredentialProbeStatus::Failed,
                format!("safe non-matching {probe_label} probe failed: {message}"),
            )
        }
        exchange::ExchangeError::RateLimited { retry_after_secs } => (
            VenueCredentialProbeStatus::Unknown,
            format!(
                "safe non-matching {probe_label} probe not proven: rate limited; retry after {retry_after_secs}s; {SAFE_ORDER_LIVE_WRITE_BOUNDARY}"
            ),
        ),
        exchange::ExchangeError::Timeout { seconds } => (
            VenueCredentialProbeStatus::Unknown,
            format!(
                "safe non-matching {probe_label} probe not proven: timeout after {seconds}s; {SAFE_ORDER_LIVE_WRITE_BOUNDARY}"
            ),
        ),
        exchange::ExchangeError::NotImplemented(_) => (
            VenueCredentialProbeStatus::Unknown,
            format!(
                "safe non-matching {probe_label} probe not proven: adapter does not expose this probe; {SAFE_ORDER_LIVE_WRITE_BOUNDARY}"
            ),
        ),
        _ => (
            VenueCredentialProbeStatus::Unknown,
            format!(
                "safe non-matching {probe_label} probe not proven: {error}; {SAFE_ORDER_LIVE_WRITE_BOUNDARY}"
            ),
        ),
    }
}

#[cfg(test)]
pub(super) fn safe_order_place_test_probe(scope: &str, source: &str) -> VenueCredentialProbe {
    safe_order_unknown_probe(
        scope,
        source,
        "safe non-matching order-test probe succeeded; cancel permission, private order stream, and order finality remain unproven by credential save",
    )
}

pub(super) fn safe_order_place_cancel_test_probe(
    scope: &str,
    source: &str,
) -> VenueCredentialProbe {
    safe_order_unknown_probe(
        scope,
        source,
        "safe non-matching order-test and cancel probe succeeded; private order stream and order finality remain unproven by credential save",
    )
}

pub(super) fn safe_order_pre_check_probe(scope: &str, source: &str) -> VenueCredentialProbe {
    safe_order_unknown_probe(
        scope,
        source,
        "safe order pre-check probe succeeded; cancel permission, private order stream, and order finality remain unproven by credential save",
    )
}

pub(super) fn safe_order_cancel_no_match_probe(scope: &str, source: &str) -> VenueCredentialProbe {
    safe_order_unknown_probe(
        scope,
        source,
        "safe non-matching cancel probe succeeded; place permission, private order stream, and order finality remain unproven by credential save",
    )
}

pub(super) fn safe_order_noop_probe(scope: &str, source: &str) -> VenueCredentialProbe {
    probe(
        "order_permission",
        VenueCredentialProbeStatus::Ok,
        scope,
        source,
        "official noop exchange action succeeded and consumed one nonce; API/Agent Wallet exchange-action signing permission verified; private order stream and order finality remain separate runtime gates",
    )
}

fn safe_order_unknown_probe(scope: &str, source: &str, message: &str) -> VenueCredentialProbe {
    let message = format!("{message}; {SAFE_ORDER_LIVE_WRITE_BOUNDARY}");
    probe(
        "order_permission",
        VenueCredentialProbeStatus::Unknown,
        scope,
        source,
        &message,
    )
}
