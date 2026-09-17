use crate::error::{ExchangeError, ExchangeResult};
use sha2::{Digest, Sha256};
use shared_types::{venue_family, ClientOrderIdDerivation, ClientOrderIdPolicy, VenueId};

const POLICY_VERSION: &str = "client-order-id-policy-v1";

const BINANCE_DOCS: &[&str] = &[
    "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/New-Order",
    "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/Query-Order",
    "https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/Cancel-Order",
];
const OKX_DOCS: &[&str] = &["https://www.okx.com/docs-v5/en/"];
const BYBIT_DOCS: &[&str] = &[
    "https://bybit-exchange.github.io/docs/v5/order/create-order",
    "https://bybit-exchange.github.io/docs/v5/order/cancel-order",
    "https://bybit-exchange.github.io/docs/v5/order/open-order",
];
const BITGET_DOCS: &[&str] = &[
    "https://www.bitget.com/api-doc/uta/trade/Place-Order",
    "https://www.bitget.com/api-doc/uta/trade/Get-Order-Details",
    "https://www.bitget.com/api-doc/uta/trade/Cancel-Order",
];
const GATE_DOCS: &[&str] = &["https://www.gate.com/docs/developers/apiv4/en/"];
const GATE_CROSSEX_DOCS: &[&str] = &[
    "https://www.gate.com/docs/developers/crossex/ws/en/",
    "https://www.gate.com/docs/developers/crossex/en/",
];
const KUCOIN_DOCS: &[&str] = &[
    "https://www.kucoin.com/docs-new/rest/futures-trading/orders/add-order",
    "https://www.kucoin.com/docs-new/rest/futures-trading/orders/get-order-by-clientoid",
    "https://www.kucoin.com/docs-new/rest/futures-trading/orders/cancel-order-by-clientoid",
];
const HYPERLIQUID_DOCS: &[&str] = &[
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint",
    "https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint",
];
const KRAKEN_DOCS: &[&str] = &[
    "https://docs.kraken.com/exchange/api-reference/spot-websocket-v2/add_order",
    "https://docs.kraken.com/api/blog/cl-ord-id/",
    "https://docs.kraken.com/exchange/api-reference/futures-rest-api",
];

const BYBIT_ID_MAX_LEN: usize = 36;
const BYBIT_HASH_BYTES: usize = 12;
const GATE_TEXT_PREFIX: &str = "t-";
const GATE_TEXT_BODY_MAX_BYTES: usize = 28;
const GATE_HASH_BYTES: usize = 6;

pub fn client_order_id_policy(venue: &str, public_client_order_id: &str) -> ClientOrderIdPolicy {
    let family = venue_policy_family(venue);
    match family.as_str() {
        "binance" => binance_policy(venue, &family, public_client_order_id),
        "okx" => okx_policy(venue, &family, public_client_order_id),
        "bybit" => bybit_policy(venue, &family, public_client_order_id),
        "bitget" => bitget_policy(venue, &family, public_client_order_id),
        "gate" => gate_policy(venue, &family, public_client_order_id),
        "gate_crossex" => gate_crossex_policy(venue, &family, public_client_order_id),
        "kraken" => kraken_policy(venue, &family, public_client_order_id),
        "kucoin" => kucoin_policy(venue, &family, public_client_order_id),
        "hyperliquid" => hyperliquid_policy(venue, &family, public_client_order_id),
        _ => unsupported_policy(venue, &family, public_client_order_id),
    }
}

pub fn required_venue_client_order_id(
    venue: &str,
    public_client_order_id: &str,
) -> ExchangeResult<String> {
    let policy = client_order_id_policy(venue, public_client_order_id);
    if let Some(blocker) = policy.blockers.first() {
        return Err(policy_validation_error(&policy, blocker));
    }
    policy
        .venue_client_order_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            policy_validation_error(
                &policy,
                "client order id policy did not produce a venue client order id",
            )
        })
}

pub fn validate_client_order_id_policy(
    venue: &str,
    public_client_order_id: &str,
) -> ExchangeResult<()> {
    required_venue_client_order_id(venue, public_client_order_id).map(|_| ())
}

fn policy_validation_error(policy: &ClientOrderIdPolicy, reason: &str) -> ExchangeError {
    ExchangeError::Api {
        exchange: policy_error_exchange(policy),
        code: "validation".into(),
        message: format!(
            "{} {} client id policy rejected {}: {reason}",
            policy.venue_family, policy.venue_field, policy.public_client_order_id
        ),
    }
}

fn policy_error_exchange(policy: &ClientOrderIdPolicy) -> String {
    if policy.venue_family.trim().is_empty() {
        policy.venue.clone()
    } else {
        policy.venue_family.clone()
    }
}

#[derive(Clone, Copy)]
struct PolicySpec {
    venue_field: &'static str,
    official_format: &'static str,
    max_length: Option<u16>,
    supports_query_by_client_id: bool,
    supports_cancel_by_client_id: bool,
    official_doc_urls: &'static [&'static str],
    constraints: &'static [&'static str],
}

struct PolicyOutcome {
    venue_client_order_id: Option<String>,
    derivation: ClientOrderIdDerivation,
    blockers: Vec<String>,
}

fn build_policy(
    venue: &str,
    family: &str,
    public_client_order_id: &str,
    spec: PolicySpec,
    outcome: PolicyOutcome,
) -> ClientOrderIdPolicy {
    ClientOrderIdPolicy {
        venue: venue.trim().to_owned(),
        venue_family: family.to_owned(),
        venue_field: spec.venue_field.to_owned(),
        public_client_order_id: public_client_order_id.to_owned(),
        venue_client_order_id: outcome.venue_client_order_id,
        derivation: outcome.derivation,
        policy_version: POLICY_VERSION.to_owned(),
        official_format: spec.official_format.to_owned(),
        max_length: spec.max_length,
        supports_query_by_client_id: spec.supports_query_by_client_id,
        supports_cancel_by_client_id: spec.supports_cancel_by_client_id,
        constraints: spec
            .constraints
            .iter()
            .map(|constraint| (*constraint).to_owned())
            .collect(),
        blockers: outcome.blockers,
        official_doc_urls: spec
            .official_doc_urls
            .iter()
            .map(|url| (*url).to_owned())
            .collect(),
    }
}

fn venue_policy_family(venue: &str) -> String {
    VenueId::from_exchange_name(venue).map_or_else(
        || venue_family(venue).trim().to_ascii_lowercase(),
        |venue_id| venue_id.as_str().to_owned(),
    )
}

fn binance_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let valid = !client_id.is_empty()
        && client_id.len() <= 36
        && client_id.bytes().all(is_binance_client_id_byte);
    let outcome = if valid {
        identity(client_id)
    } else {
        rejected("Binance newClientOrderId must match ^[\\.A-Z\\:/a-z0-9_-]{1,36}$")
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "newClientOrderId/origClientOrderId",
            official_format: "^[\\.A-Z\\:/a-z0-9_-]{1,36}$",
            max_length: Some(36),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            official_doc_urls: BINANCE_DOCS,
            constraints: &["unique among open orders"],
        },
        outcome,
    )
}

fn okx_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let valid = !client_id.is_empty()
        && client_id.len() <= 32
        && client_id.bytes().all(|byte| byte.is_ascii_alphanumeric());
    let outcome = if valid {
        identity(client_id)
    } else {
        rejected("OKX clOrdId must be 1..=32 ASCII alphanumeric characters")
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "clOrdId",
            official_format: "1..=32 ASCII alphanumeric characters",
            max_length: Some(32),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            official_doc_urls: OKX_DOCS,
            constraints: &["unique among current pending orders"],
        },
        outcome,
    )
}

fn bybit_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let raw = client_id.trim();
    let outcome = if raw.is_empty() {
        rejected("Bybit orderLinkId cannot be empty")
    } else if is_bybit_order_link_id(raw) {
        identity(raw)
    } else {
        derived(
            bybit_order_link_id(raw),
            ClientOrderIdDerivation::StableHash,
        )
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "orderLinkId",
            official_format: "1..=36 ASCII letters, numbers, dashes, and underscores",
            max_length: Some(36),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            official_doc_urls: BYBIT_DOCS,
            constraints: &[
                "cancel acknowledgement is asynchronous; finality requires order stream",
            ],
        },
        outcome,
    )
}

fn bitget_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let trimmed = client_id.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 32
        && trimmed.bytes().all(is_bitget_client_oid_byte);
    let outcome = if valid {
        normalized(client_id, trimmed)
    } else {
        rejected("Bitget UTA clientOid must match ^[\\.A-Z\\:/a-z0-9_-]{1,32}$")
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "clientOid",
            official_format: "^[\\.A-Z\\:/a-z0-9_-]{1,32}$",
            max_length: Some(32),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            official_doc_urls: BITGET_DOCS,
            constraints: &["orderId takes priority when both orderId and clientOid are provided"],
        },
        outcome,
    )
}

fn gate_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let trimmed = client_id.trim();
    let outcome = if trimmed.is_empty() {
        rejected("Gate futures text cannot be derived from an empty public client id")
    } else {
        let text = gate_text(trimmed);
        let derivation = if text == trimmed {
            ClientOrderIdDerivation::Identity
        } else if trimmed
            .strip_prefix(GATE_TEXT_PREFIX)
            .is_some_and(is_gate_text_body)
        {
            ClientOrderIdDerivation::Normalized
        } else {
            ClientOrderIdDerivation::StableHash
        };
        derived(text, derivation)
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "text",
            official_format: "t- prefix plus <=28 byte body using ASCII letters, numbers, _, -, .",
            max_length: Some(30),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: false,
            official_doc_urls: GATE_DOCS,
            constraints: &[
                "custom text lookup is time-limited by Gate; exchange order id is preferred for late cancel",
            ],
        },
        outcome,
    )
}

fn gate_crossex_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let trimmed = client_id.trim();
    let outcome = if trimmed.is_empty() {
        rejected("Gate CrossEx text cannot be derived from an empty public client id")
    } else if trimmed.bytes().all(is_gate_crossex_text_byte) {
        identity(trimmed)
    } else {
        derived(
            hash_id("cx-", b"crossline:gate-crossex:text:v1:", trimmed, 12),
            ClientOrderIdDerivation::StableHash,
        )
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "text",
            official_format: "lowercase ASCII letters, numbers, dashes, and underscores",
            max_length: None,
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            official_doc_urls: GATE_CROSSEX_DOCS,
            constraints: &[
                "official docs expose a length error but do not publish the maximum length",
            ],
        },
        outcome,
    )
}

fn kraken_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let trimmed = client_id.trim();
    let outcome = if trimmed.is_empty() {
        rejected("Kraken cl_ord_id cannot be derived from an empty public client id")
    } else if is_kraken_client_id(trimmed) {
        identity(trimmed)
    } else {
        derived(
            hash_id("", b"crossline:kraken:cl_ord_id:v1:", trimmed, 16),
            ClientOrderIdDerivation::StableHash,
        )
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "cl_ord_id/cliOrdId",
            official_format:
                "UUID, 32 hexadecimal characters, or free-format ASCII up to 18 characters",
            max_length: Some(36),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            official_doc_urls: KRAKEN_DOCS,
            constraints: &["cl_ord_id must be unique among open orders"],
        },
        outcome,
    )
}

fn kucoin_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let trimmed = client_id.trim();
    let outcome = if trimmed.is_empty() {
        rejected("KuCoin clientOid must be non-empty")
    } else {
        normalized(client_id, trimmed)
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "clientOid",
            official_format:
                "non-empty clientOid; exact max length/charset not exposed in fetched official docs",
            max_length: None,
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            official_doc_urls: KUCOIN_DOCS,
            constraints: &[
                "fetched official docs expose by-clientOid endpoints but not exact charset",
            ],
        },
        outcome,
    )
}

fn hyperliquid_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    let trimmed = client_id.trim();
    let outcome = if trimmed.is_empty() {
        rejected("Hyperliquid cloid cannot be derived from an empty public client id")
    } else {
        let cloid = hyperliquid_cloid(trimmed);
        let derivation = if cloid == trimmed {
            ClientOrderIdDerivation::Identity
        } else if is_hyperliquid_cloid(trimmed) {
            ClientOrderIdDerivation::Normalized
        } else {
            ClientOrderIdDerivation::StableHash
        };
        derived(cloid, derivation)
    };
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "c/cloid",
            official_format: "0x + 32 hex chars (128-bit client order id)",
            max_length: Some(34),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            official_doc_urls: HYPERLIQUID_DOCS,
            constraints: &[
                "order action uses compact key c; info/cancel docs call the same value cloid",
            ],
        },
        outcome,
    )
}

fn unsupported_policy(venue: &str, family: &str, client_id: &str) -> ClientOrderIdPolicy {
    build_policy(
        venue,
        family,
        client_id,
        PolicySpec {
            venue_field: "unknown",
            official_format: "unknown",
            max_length: None,
            supports_query_by_client_id: false,
            supports_cancel_by_client_id: false,
            official_doc_urls: &[],
            constraints: &[],
        },
        rejected("Unknown venue client order id policy; official docs are required before live order write"),
    )
}

fn identity(client_id: &str) -> PolicyOutcome {
    derived(client_id.to_owned(), ClientOrderIdDerivation::Identity)
}

fn normalized(raw: &str, normalized: &str) -> PolicyOutcome {
    let derivation = if raw == normalized {
        ClientOrderIdDerivation::Identity
    } else {
        ClientOrderIdDerivation::Normalized
    };
    derived(normalized.to_owned(), derivation)
}

fn derived(venue_client_order_id: String, derivation: ClientOrderIdDerivation) -> PolicyOutcome {
    PolicyOutcome {
        venue_client_order_id: Some(venue_client_order_id),
        derivation,
        blockers: Vec::new(),
    }
}

fn rejected(message: &str) -> PolicyOutcome {
    PolicyOutcome {
        venue_client_order_id: None,
        derivation: ClientOrderIdDerivation::Rejected,
        blockers: vec![message.to_owned()],
    }
}

fn is_binance_client_id_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'.' | b':' | b'/' | b'_' | b'-' | b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
    )
}

fn is_bitget_client_oid_byte(byte: u8) -> bool {
    is_binance_client_id_byte(byte)
}

fn is_bybit_order_link_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= BYBIT_ID_MAX_LEN
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn bybit_order_link_id(raw: &str) -> String {
    hash_id(
        "bb-",
        b"crossline:bybit:orderLinkId:v1:",
        raw,
        BYBIT_HASH_BYTES,
    )
}

fn gate_text(client_order_id: &str) -> String {
    format!("{GATE_TEXT_PREFIX}{}", gate_text_body(client_order_id))
}

fn gate_text_body(client_order_id: &str) -> String {
    let raw_body = client_order_id
        .trim()
        .strip_prefix(GATE_TEXT_PREFIX)
        .unwrap_or_else(|| client_order_id.trim());
    let sanitized = sanitize_gate_text_body(raw_body);
    if !sanitized.is_empty() && sanitized == raw_body && sanitized.len() <= GATE_TEXT_BODY_MAX_BYTES
    {
        sanitized
    } else {
        compact_gate_text_body(raw_body, &sanitized)
    }
}

fn is_gate_text_body(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= GATE_TEXT_BODY_MAX_BYTES
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn is_gate_crossex_text_byte(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_')
}

fn is_kraken_client_id(value: &str) -> bool {
    let is_free_text = value.len() <= 18 && value.is_ascii();
    let is_short_uuid = value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    let is_long_uuid = value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        });
    is_free_text || is_short_uuid || is_long_uuid
}

fn sanitize_gate_text_body(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn compact_gate_text_body(raw: &str, sanitized: &str) -> String {
    let suffix = gate_text_hash(raw);
    let head_limit = GATE_TEXT_BODY_MAX_BYTES - suffix.len() - 1;
    let head = sanitized
        .trim_matches('-')
        .chars()
        .take(head_limit)
        .collect::<String>();
    let head = if head.is_empty() { "id" } else { &head };
    format!("{head}-{suffix}")
}

fn gate_text_hash(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    hex::encode(&digest[..GATE_HASH_BYTES])
}

fn hyperliquid_cloid(client_order_id: &str) -> String {
    if is_hyperliquid_cloid(client_order_id) {
        return client_order_id.to_ascii_lowercase();
    }
    hash_id(
        "0x",
        b"crossline:hyperliquid:cloid:v1:",
        client_order_id,
        16,
    )
}

fn is_hyperliquid_cloid(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .is_some_and(|hex| hex.len() == 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn hash_id(prefix: &str, namespace: &[u8], raw: &str, bytes: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace);
    hasher.update(raw.as_bytes());
    let digest = hasher.finalize();
    format!("{prefix}{}", hex::encode(&digest[..bytes]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binance_policy_accepts_official_regex() {
        let policy = client_order_id_policy("binance", "ABC.def:/xyz_09-1");

        assert_eq!(policy.venue_field, "newClientOrderId/origClientOrderId");
        assert_eq!(policy.derivation, ClientOrderIdDerivation::Identity);
        assert_eq!(
            policy.venue_client_order_id.as_deref(),
            Some("ABC.def:/xyz_09-1")
        );
        assert!(policy.supports_query_by_client_id);
        assert!(policy.supports_cancel_by_client_id);
        assert!(policy.blockers.is_empty());
    }

    #[test]
    fn okx_policy_rejects_hyphenated_public_id() {
        let policy = client_order_id_policy("okx", "bad-id");

        assert_eq!(policy.derivation, ClientOrderIdDerivation::Rejected);
        assert!(policy.venue_client_order_id.is_none());
        assert!(policy.blockers[0].contains("clOrdId"));
    }

    #[test]
    fn bybit_policy_derives_unsupported_public_id() {
        let policy = client_order_id_policy("bybit", "client:with:colon");

        assert_eq!(policy.venue_field, "orderLinkId");
        assert_eq!(policy.derivation, ClientOrderIdDerivation::StableHash);
        assert!(policy
            .venue_client_order_id
            .as_deref()
            .is_some_and(|id| id.starts_with("bb-") && id.len() == 27));
        assert!(policy.blockers.is_empty());
    }

    #[test]
    fn bitget_policy_rejects_outside_official_regex() {
        let policy = client_order_id_policy("bitget", "client id too long and has spaces");

        assert_eq!(policy.derivation, ClientOrderIdDerivation::Rejected);
        assert!(policy.blockers[0].contains("clientOid"));
    }

    #[test]
    fn gate_policy_compacts_to_official_text_shape() {
        let policy = client_order_id_policy("gate", "cid/with spaces/中文");
        let text = policy.venue_client_order_id.unwrap_or_default();

        assert_eq!(policy.venue_field, "text");
        assert_eq!(policy.derivation, ClientOrderIdDerivation::StableHash);
        assert!(text.starts_with("t-"));
        assert!(text.len() <= 30);
        assert!(policy.blockers.is_empty());
    }

    #[test]
    fn kucoin_policy_keeps_trimmed_client_oid_without_unverified_charset() {
        let policy = client_order_id_policy("kucoin", " client/with space ");

        assert_eq!(policy.derivation, ClientOrderIdDerivation::Normalized);
        assert_eq!(
            policy.venue_client_order_id.as_deref(),
            Some("client/with space")
        );
        assert!(policy.official_format.contains("not exposed"));
    }

    #[test]
    fn hyperliquid_policy_derives_128_bit_cloid_for_builder_venue() {
        let policy = client_order_id_policy("hyperliquid:xyz", "client-order-1");
        let cloid = policy.venue_client_order_id.unwrap_or_default();

        assert_eq!(policy.venue_family, "hyperliquid");
        assert_eq!(policy.derivation, ClientOrderIdDerivation::StableHash);
        assert!(cloid.starts_with("0x"));
        assert_eq!(cloid.len(), 34);
    }

    #[test]
    fn hyperliquid_policy_normalizes_official_cloid_case() {
        let policy = client_order_id_policy("hyperliquid", "0xABCDEFabcdef12345678901234567890");

        assert_eq!(policy.derivation, ClientOrderIdDerivation::Normalized);
        assert_eq!(
            policy.venue_client_order_id.as_deref(),
            Some("0xabcdefabcdef12345678901234567890")
        );
    }

    #[test]
    fn kraken_policy_derives_short_uuid_for_long_public_id() {
        let policy = client_order_id_policy(
            "kraken",
            "crossline-public-order-id-that-is-longer-than-eighteen-characters",
        );
        let client_id = policy.venue_client_order_id.unwrap_or_default();

        assert_eq!(policy.venue_field, "cl_ord_id/cliOrdId");
        assert_eq!(policy.derivation, ClientOrderIdDerivation::StableHash);
        assert_eq!(client_id.len(), 32);
        assert!(client_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn gate_crossex_policy_normalizes_to_documented_charset() {
        let policy = client_order_id_policy("gate_crossex", "Crossline Order/中文");
        let text = policy.venue_client_order_id.unwrap_or_default();

        assert_eq!(policy.venue_field, "text");
        assert_eq!(policy.derivation, ClientOrderIdDerivation::StableHash);
        assert!(text.starts_with("cx-"));
        assert!(text.bytes().all(is_gate_crossex_text_byte));
    }
}
