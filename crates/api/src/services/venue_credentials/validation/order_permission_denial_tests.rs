use super::*;

struct SafeProbeDenialCase {
    label: &'static str,
    classify: fn(&exchange::ExchangeError) -> (VenueCredentialProbeStatus, String),
    error: exchange::ExchangeError,
}

#[test]
fn safe_order_probe_common_auth_denials_fail_closed() {
    let cases = [
        SafeProbeDenialCase {
            label: "binance api key text",
            classify: classify_safe_order_place_cancel_test_error,
            error: exchange::ExchangeError::Http {
                status: 400,
                body: r#"{"code":-2015,"msg":"Invalid API key, IP, or permissions for action."}"#
                    .into(),
            },
        },
        SafeProbeDenialCase {
            label: "okx signature code",
            classify: classify_safe_order_pre_check_error,
            error: exchange::ExchangeError::Api {
                exchange: "okx".into(),
                code: "50113".into(),
                message: "Invalid signature".into(),
            },
        },
        SafeProbeDenialCase {
            label: "bybit permission code",
            classify: classify_safe_order_pre_check_error,
            error: exchange::ExchangeError::Api {
                exchange: "bybit".into(),
                code: "10005".into(),
                message: "not authorized for order pre-check".into(),
            },
        },
        SafeProbeDenialCase {
            label: "bitget access denied code",
            classify: classify_safe_order_cancel_no_match_error,
            error: exchange::ExchangeError::Api {
                exchange: "bitget".into(),
                code: "40037".into(),
                message: "access denied".into(),
            },
        },
        SafeProbeDenialCase {
            label: "gate forbidden body",
            classify: classify_safe_order_cancel_no_match_error,
            error: exchange::ExchangeError::Http {
                status: 400,
                body: r#"{"label":"FORBIDDEN","message":"forbidden"}"#.into(),
            },
        },
        SafeProbeDenialCase {
            label: "kucoin api key header",
            classify: classify_safe_order_place_cancel_test_error,
            error: exchange::ExchangeError::Http {
                status: 400,
                body: r#"{"code":"400003","msg":"KC-API-KEY not exists"}"#.into(),
            },
        },
        SafeProbeDenialCase {
            label: "hyperliquid unauthorized noop",
            classify: classify_safe_order_noop_error,
            error: exchange::ExchangeError::Api {
                exchange: "hyperliquid".into(),
                code: "unauthorized".into(),
                message: "not authorized".into(),
            },
        },
    ];

    for case in cases {
        let (status, message) = (case.classify)(&case.error);
        assert_eq!(
            status,
            VenueCredentialProbeStatus::Failed,
            "{} should fail closed: {}",
            case.label,
            message
        );
        assert!(message.contains("safe non-matching"), "{}", case.label);
    }
}

#[test]
fn safe_order_probe_non_auth_failures_remain_unknown() {
    let cases = [
        exchange::ExchangeError::Api {
            exchange: "okx".into(),
            code: "51000".into(),
            message: "parameter instId error".into(),
        },
        exchange::ExchangeError::RateLimited {
            retry_after_secs: 2,
        },
        exchange::ExchangeError::Timeout { seconds: 3 },
    ];

    for error in cases {
        let (status, message) = classify_safe_order_pre_check_error(&error);
        assert_eq!(
            status,
            VenueCredentialProbeStatus::Unknown,
            "non-auth error should stay Unknown: {message}"
        );
        assert!(message.contains("not proven"));
    }
}

#[test]
fn hyperliquid_missing_agent_is_an_actionable_failure() {
    let (status, message) = classify_safe_order_noop_error(&exchange::ExchangeError::Api {
        exchange: "hyperliquid".into(),
        code: "noop".into(),
        message: r#"{"response":"User or API Wallet 0xabc does not exist.","status":"err"}"#.into(),
    });

    assert_eq!(status, VenueCredentialProbeStatus::Failed);
    assert!(message.contains("API/Agent Wallet"));
    assert!(message.contains("save credentials again"));
}
