use super::*;

fn balance(currency: &str) -> BalanceInfo {
    BalanceInfo {
        currency: currency.into(),
        total: 1.0,
        available: 1.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }
}

#[test]
fn balance_probe_accepts_an_empty_account() {
    let balances = std::collections::HashMap::new();
    assert!(ensure_balance_probe_is_well_formed(&balances).is_ok());
}

#[test]
fn balance_probe_does_not_require_a_specific_asset() {
    let mut balances = std::collections::HashMap::new();
    balances.insert("USDC".into(), balance("USDC"));

    assert!(ensure_balance_probe_is_well_formed(&balances).is_ok());
}

#[test]
fn balance_probe_rejects_malformed_rows() {
    let mut balances = std::collections::HashMap::new();
    let mut malformed = balance("USDT");
    malformed.total = f64::NAN;
    balances.insert("USDT".into(), malformed);

    assert!(matches!(
        ensure_balance_probe_is_well_formed(&balances),
        Err(CredentialUpdateError::Validation(message))
            if message.contains("malformed row")
    ));
}

#[test]
fn optional_probe_auth_failure_is_failed_not_unknown() {
    let (status, message) = classify_optional_probe_error(
        "positions",
        &exchange::ExchangeError::Auth("bad key".into()),
    );
    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("venue rejected credentials"));
}

#[test]
fn optional_probe_forbidden_http_is_failed() {
    for status_code in [401_u16, 403] {
        let (status, _message) = classify_optional_probe_error(
            "open orders",
            &exchange::ExchangeError::Http {
                status: status_code,
                body: "denied".into(),
            },
        );
        assert_eq!(
            status,
            VenueCredentialProbeStatus::Failed,
            "http {status_code} must be a definitive failure"
        );
    }
}

#[test]
fn optional_probe_transient_errors_stay_unknown() {
    let cases = [
        exchange::ExchangeError::Timeout { seconds: 3 },
        exchange::ExchangeError::RateLimited {
            retry_after_secs: 5,
        },
        exchange::ExchangeError::NotImplemented("no reader"),
        exchange::ExchangeError::Http {
            status: 500,
            body: "err".into(),
        },
    ];
    for error in cases {
        let (status, _message) = classify_optional_probe_error("positions", &error);
        assert_eq!(
            status,
            VenueCredentialProbeStatus::Unknown,
            "{error:?} should stay unknown (retryable / not a rejection)"
        );
    }
}
