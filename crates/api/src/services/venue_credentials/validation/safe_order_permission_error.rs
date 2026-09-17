pub(super) fn api_error_looks_like_permission_denial(code: &str, message: &str) -> bool {
    permission_text(code) || permission_text(message)
}

pub(super) fn http_body_looks_like_permission_denial(body: &str) -> bool {
    permission_text(body)
}

fn permission_text(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    const DENIAL_PATTERNS: &[&str] = &[
        "permission",
        "api-key",
        "api key",
        "api_key",
        "invalid api",
        "invalid signature",
        "signature not valid",
        "not authorized",
        "not authorised",
        "unauthorized",
        "unauthorised",
        "forbidden",
        "access denied",
        "ip restricted",
        "-2015",
        "10005",
        "40037",
        "50113",
    ];

    DENIAL_PATTERNS
        .iter()
        .any(|pattern| value.contains(pattern))
}

pub(super) fn hyperliquid_signer_is_not_registered(error: &exchange::ExchangeError) -> bool {
    matches!(
        error,
        exchange::ExchangeError::Api {
            exchange,
            message,
            ..
        } if exchange.eq_ignore_ascii_case("hyperliquid")
            && message.contains("User or API Wallet")
            && message.contains("does not exist")
    )
}
