use super::*;

#[cfg(not(test))]
pub(super) async fn optional_order_permission_probe<F>(
    scope: &str,
    source: &str,
    request: F,
) -> VenueCredentialProbe
where
    F: std::future::Future<Output = exchange::ExchangeResult<()>>,
{
    match tokio::time::timeout(
        std::time::Duration::from_secs(OPTIONAL_READ_PROBE_TIMEOUT_SECS),
        request,
    )
    .await
    {
        Ok(Ok(())) => probe(
            "order_permission",
            VenueCredentialProbeStatus::Ok,
            scope,
            source,
            "read-only order-permission probe succeeded",
        ),
        Ok(Err(error)) => {
            let (status, message) = classify_order_permission_probe_error(&error);
            probe("order_permission", status, scope, source, &message)
        }
        Err(_) => probe(
            "order_permission",
            VenueCredentialProbeStatus::Unknown,
            scope,
            source,
            "read-only order-permission probe timed out; save kept read-only evidence",
        ),
    }
}

pub(super) fn classify_order_permission_probe_error(
    error: &exchange::ExchangeError,
) -> (VenueCredentialProbeStatus, String) {
    match error {
        exchange::ExchangeError::Api { code, message, .. }
            if code == "api_trading_disabled"
                || code == "unified_account_api_ordering_unsupported"
                || code == "unknown_account_type" =>
        {
            (
                VenueCredentialProbeStatus::Failed,
                format!("read-only order-permission probe failed: {message}"),
            )
        }
        _ => {
            let (status, message) = classify_optional_probe_error("order permission", error);
            (status, message)
        }
    }
}

pub(super) fn order_permission_probe(checked_at_ms: i64) -> VenueCredentialProbe {
    VenueCredentialProbe {
        kind: "order_permission".into(),
        status: VenueCredentialProbeStatus::Unknown,
        scope: "place_cancel_order_stream".into(),
        source: "not_probed".into(),
        message: "order placement, cancellation, and order-stream permissions are not proven by credential save".into(),
        checked_at_ms,
        request_id: common::request_id::current(),
    }
}

pub(super) fn order_permission_unproven_probe(venue: &str) -> VenueCredentialProbe {
    let venue = normalized_probe_venue(venue);
    probe(
        "order_permission",
        VenueCredentialProbeStatus::Unknown,
        &format!("{venue}_place_cancel_order_stream"),
        &format!("credential_save.order_permission_unproven.{venue}"),
        "order placement, cancellation, private order stream, and order finality are not proven by credential save",
    )
}

fn normalized_probe_venue(venue: &str) -> String {
    let trimmed = venue.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed
    }
}
