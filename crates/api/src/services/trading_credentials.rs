use crate::services::okx_credential_profile::{select_okx_profile, ENV_KEYS};
use crate::trading_service::{
    AdapterCredentials, HyperliquidAdapterCredentials, KrakenAdapterCredentials,
};
use dashmap::DashMap;
use std::sync::OnceLock;

const CREDENTIAL_FINGERPRINT_PREFIX: &str = "hmac-sha256:";
const CREDENTIAL_FINGERPRINT_HEX_LEN: usize = 24;

pub(crate) fn current_adapter_credentials() -> AdapterCredentials {
    AdapterCredentials {
        binance_live: pair("BINANCE_API_KEY", "BINANCE_API_SECRET"),
        bitget_live: triple("BITGET_API_KEY", "BITGET_API_SECRET", "BITGET_PASSPHRASE"),
        bybit_live: pair("BYBIT_API_KEY", "BYBIT_API_SECRET"),
        gate_live: pair("GATE_API_KEY", "GATE_API_SECRET"),
        gate_crossex_live: pair("GATE_CROSSEX_API_KEY", "GATE_CROSSEX_API_SECRET"),
        hyperliquid_live: hyperliquid_live_credentials(),
        kucoin_live: triple("KUCOIN_API_KEY", "KUCOIN_API_SECRET", "KUCOIN_PASSPHRASE"),
        kraken_live: kraken_live_credentials(),
        okx_live: okx_live_credentials(),
    }
}

fn kraken_live_credentials() -> Option<KrakenAdapterCredentials> {
    let credentials = KrakenAdapterCredentials {
        spot: pair("KRAKEN_SPOT_API_KEY", "KRAKEN_SPOT_API_SECRET"),
        futures: pair("KRAKEN_FUTURES_API_KEY", "KRAKEN_FUTURES_API_SECRET"),
    };
    credentials.is_configured().then_some(credentials)
}

pub(crate) fn credential_fingerprint(venue: &str) -> Option<String> {
    let venue_id = shared_types::venue_family_id(venue)?;
    if let Some(fingerprint) = credential_fingerprints().get(venue_id.as_str()) {
        return Some(fingerprint.value().clone());
    }
    let fingerprint = compute_credential_fingerprint(venue_id)?;
    credential_fingerprints().insert(venue_id.as_str().to_owned(), fingerprint.clone());
    Some(fingerprint)
}

pub(crate) fn refresh_credential_fingerprint(venue: &str) -> Option<String> {
    let venue_id = shared_types::venue_family_id(venue)?;
    let key = venue_id.as_str();
    let fingerprint = compute_credential_fingerprint(venue_id);
    match fingerprint.as_ref() {
        Some(fingerprint) => {
            credential_fingerprints().insert(key.to_owned(), fingerprint.clone());
        }
        None => {
            credential_fingerprints().remove(key);
        }
    }
    fingerprint
}

fn credential_fingerprints() -> &'static DashMap<String, String> {
    static FINGERPRINTS: OnceLock<DashMap<String, String>> = OnceLock::new();
    FINGERPRINTS.get_or_init(DashMap::new)
}

fn compute_credential_fingerprint(venue_id: shared_types::VenueId) -> Option<String> {
    match venue_id {
        shared_types::VenueId::Binance => pair("BINANCE_API_KEY", "BINANCE_API_SECRET")
            .as_ref()
            .map(|value| pair_fingerprint(venue_id.as_str(), value)),
        shared_types::VenueId::Bitget => {
            triple("BITGET_API_KEY", "BITGET_API_SECRET", "BITGET_PASSPHRASE")
                .as_ref()
                .map(|value| triple_fingerprint(venue_id.as_str(), value))
        }
        shared_types::VenueId::Bybit => pair("BYBIT_API_KEY", "BYBIT_API_SECRET")
            .as_ref()
            .map(|value| pair_fingerprint(venue_id.as_str(), value)),
        shared_types::VenueId::Gate => pair("GATE_API_KEY", "GATE_API_SECRET")
            .as_ref()
            .map(|value| pair_fingerprint(venue_id.as_str(), value)),
        shared_types::VenueId::GateCrossEx => {
            pair("GATE_CROSSEX_API_KEY", "GATE_CROSSEX_API_SECRET")
                .as_ref()
                .map(|value| pair_fingerprint(venue_id.as_str(), value))
        }
        shared_types::VenueId::Htx => None,
        shared_types::VenueId::Hyperliquid => {
            hyperliquid_live_credentials().as_ref().map(|value| {
                credential_fingerprint_from_parts(
                    venue_id.as_str(),
                    &value.account_address,
                    &value.private_key,
                    value.vault_address.as_deref(),
                )
            })
        }
        shared_types::VenueId::Kraken => kraken_credential_fingerprint(),
        shared_types::VenueId::Kucoin => {
            triple("KUCOIN_API_KEY", "KUCOIN_API_SECRET", "KUCOIN_PASSPHRASE")
                .as_ref()
                .map(|value| triple_fingerprint(venue_id.as_str(), value))
        }
        shared_types::VenueId::Okx => okx_live_credentials()
            .as_ref()
            .map(|value| triple_fingerprint(venue_id.as_str(), value)),
    }
}

fn kraken_credential_fingerprint() -> Option<String> {
    let spot = pair("KRAKEN_SPOT_API_KEY", "KRAKEN_SPOT_API_SECRET");
    let futures = pair("KRAKEN_FUTURES_API_KEY", "KRAKEN_FUTURES_API_SECRET");
    let (public_id, secret) = match (spot, futures) {
        (Some(spot), Some(futures)) => (
            format!("{}\0{}", spot.0, futures.0),
            format!("{}\0{}", spot.1, futures.1),
        ),
        (Some(spot), None) => spot,
        (None, Some(futures)) => futures,
        (None, None) => return None,
    };
    Some(credential_fingerprint_from_parts(
        shared_types::VenueId::Kraken.as_str(),
        &public_id,
        &secret,
        None,
    ))
}

fn pair_fingerprint(venue: &str, credentials: &(String, String)) -> String {
    credential_fingerprint_from_parts(venue, &credentials.0, &credentials.1, None)
}

fn triple_fingerprint(venue: &str, credentials: &(String, String, String)) -> String {
    credential_fingerprint_from_parts(venue, &credentials.0, &credentials.1, Some(&credentials.2))
}

fn credential_fingerprint_from_parts(
    venue: &str,
    public_id: &str,
    secret: &str,
    third_factor: Option<&str>,
) -> String {
    let message = format!(
        "crossline-account-binding-v1\0{venue}\0{public_id}\0{}",
        third_factor.unwrap_or_default()
    );
    let digest = common::signing::hmac_sha256_hex(secret.as_bytes(), message.as_bytes());
    let fingerprint = digest
        .get(..CREDENTIAL_FINGERPRINT_HEX_LEN)
        .unwrap_or(digest.as_str());
    format!("{CREDENTIAL_FINGERPRINT_PREFIX}{fingerprint}")
}

fn hyperliquid_live_credentials() -> Option<HyperliquidAdapterCredentials> {
    hyperliquid_profile(value)
}

fn hyperliquid_profile(
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<HyperliquidAdapterCredentials> {
    let account_address =
        lookup("HYPERLIQUID_ACCOUNT_ADDRESS").or_else(|| lookup("HYPERLIQUID_USER_ADDRESS"))?;
    Some(HyperliquidAdapterCredentials {
        account_address,
        private_key: lookup("HYPERLIQUID_PRIVATE_KEY")?,
        vault_address: lookup("HYPERLIQUID_VAULT_ADDRESS"),
    })
}

fn okx_live_credentials() -> Option<(String, String, String)> {
    okx_profile(value)
}

/// 选定 OKX 实盘采用的凭证档案。
///
/// 仅当 `OKX_LIVE_*` 三件套全部就绪时整体采用 live 档案；否则整体回退到普通
/// `OKX_*` 三件套。绝不跨档案逐字段混用（例如 live key 配普通 passphrase），
/// 以免设置页验证的是普通档案、实盘路由却用上未验证的混合档案。
fn okx_profile(lookup: impl Fn(&str) -> Option<String>) -> Option<(String, String, String)> {
    let profile = select_okx_profile(lookup, ENV_KEYS)?;
    Some((profile.api_key, profile.api_secret, profile.passphrase))
}

fn triple(key: &str, secret: &str, passphrase: &str) -> Option<(String, String, String)> {
    Some((value(key)?, value(secret)?, value(passphrase)?))
}

fn pair(key: &str, secret: &str) -> Option<(String, String)> {
    Some((value(key)?, value(secret)?))
}

fn value(key: &str) -> Option<String> {
    crate::services::venue_credentials::secret(key)
}

#[cfg(test)]
mod tests {
    use super::{credential_fingerprint_from_parts, hyperliquid_profile, okx_profile};
    use std::collections::HashMap;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn okx_profile_prefers_complete_live_triple() {
        let lookup = lookup_from(&[
            ("OKX_API_KEY", "normal-key"),
            ("OKX_API_SECRET", "normal-secret"),
            ("OKX_PASSPHRASE", "normal-pass"),
            ("OKX_LIVE_API_KEY", "live-key"),
            ("OKX_LIVE_API_SECRET", "live-secret"),
            ("OKX_LIVE_PASSPHRASE", "live-pass"),
        ]);
        assert_eq!(
            okx_profile(lookup),
            Some((
                "live-key".to_owned(),
                "live-secret".to_owned(),
                "live-pass".to_owned()
            ))
        );
    }

    #[test]
    fn okx_profile_never_mixes_partial_live_with_normal() {
        let lookup = lookup_from(&[
            ("OKX_API_KEY", "normal-key"),
            ("OKX_API_SECRET", "normal-secret"),
            ("OKX_PASSPHRASE", "normal-pass"),
            ("OKX_LIVE_API_KEY", "live-key"),
            ("OKX_LIVE_API_SECRET", "live-secret"),
        ]);
        assert_eq!(
            okx_profile(lookup),
            Some((
                "normal-key".to_owned(),
                "normal-secret".to_owned(),
                "normal-pass".to_owned()
            )),
            "partial live profile must fall back to the complete normal profile, not mix fields"
        );
    }

    #[test]
    fn okx_profile_uses_normal_when_no_live_fields() {
        let lookup = lookup_from(&[
            ("OKX_API_KEY", "normal-key"),
            ("OKX_API_SECRET", "normal-secret"),
            ("OKX_PASSPHRASE", "normal-pass"),
        ]);
        assert_eq!(
            okx_profile(lookup),
            Some((
                "normal-key".to_owned(),
                "normal-secret".to_owned(),
                "normal-pass".to_owned()
            ))
        );
    }

    #[test]
    fn okx_profile_none_when_normal_incomplete_and_no_live() {
        let lookup = lookup_from(&[
            ("OKX_API_KEY", "normal-key"),
            ("OKX_API_SECRET", "normal-secret"),
        ]);
        assert_eq!(okx_profile(lookup), None);
    }

    #[test]
    fn hyperliquid_profile_preserves_optional_vault_scope() {
        let lookup = lookup_from(&[
            ("HYPERLIQUID_ACCOUNT_ADDRESS", "0xmain"),
            ("HYPERLIQUID_PRIVATE_KEY", "agent-key"),
            ("HYPERLIQUID_VAULT_ADDRESS", "0xvault"),
        ]);

        let profile = hyperliquid_profile(lookup);

        assert!(profile.is_some_and(|profile| {
            profile.account_address == "0xmain"
                && profile.private_key == "agent-key"
                && profile.vault_address.as_deref() == Some("0xvault")
        }));
    }

    #[test]
    fn hyperliquid_profile_accepts_legacy_user_address_without_vault() {
        let lookup = lookup_from(&[
            ("HYPERLIQUID_USER_ADDRESS", "0xlegacy"),
            ("HYPERLIQUID_PRIVATE_KEY", "agent-key"),
        ]);

        let profile = hyperliquid_profile(lookup);

        assert!(profile.is_some_and(|profile| {
            profile.account_address == "0xlegacy" && profile.vault_address.is_none()
        }));
    }

    #[test]
    fn credential_fingerprint_is_stable_opaque_and_generation_sensitive() {
        let first = credential_fingerprint_from_parts(
            "okx",
            "public-key",
            "secret-value",
            Some("passphrase-value"),
        );
        let replay = credential_fingerprint_from_parts(
            "okx",
            "public-key",
            "secret-value",
            Some("passphrase-value"),
        );
        let rotated = credential_fingerprint_from_parts(
            "okx",
            "public-key",
            "rotated-secret",
            Some("passphrase-value"),
        );

        assert_eq!(first, replay);
        assert_ne!(first, rotated);
        assert!(first.starts_with("hmac-sha256:"));
        assert_eq!(first.len(), "hmac-sha256:".len() + 24);
        for secret in ["public-key", "secret-value", "passphrase-value"] {
            assert!(!first.contains(secret));
        }
    }
}
