#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OkxCredentialProfile {
    pub(crate) api_key: String,
    pub(crate) api_secret: String,
    pub(crate) passphrase: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OkxCredentialProfileKeys {
    pub(crate) live: OkxProfileKeys,
    pub(crate) normal: OkxProfileKeys,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OkxProfileKeys {
    pub(crate) api_key: &'static str,
    pub(crate) api_secret: &'static str,
    pub(crate) passphrase: &'static str,
}

pub(crate) const ENV_KEYS: OkxCredentialProfileKeys = OkxCredentialProfileKeys {
    live: OkxProfileKeys {
        api_key: "OKX_LIVE_API_KEY",
        api_secret: "OKX_LIVE_API_SECRET",
        passphrase: "OKX_LIVE_PASSPHRASE",
    },
    normal: OkxProfileKeys {
        api_key: "OKX_API_KEY",
        api_secret: "OKX_API_SECRET",
        passphrase: "OKX_PASSPHRASE",
    },
};

pub(crate) const FIELD_KEYS: OkxCredentialProfileKeys = OkxCredentialProfileKeys {
    live: OkxProfileKeys {
        api_key: "live_key",
        api_secret: "live_secret",
        passphrase: "live_passphrase",
    },
    normal: OkxProfileKeys {
        api_key: "api_key",
        api_secret: "api_secret",
        passphrase: "passphrase",
    },
};

pub(crate) fn select_okx_profile(
    lookup: impl Fn(&str) -> Option<String>,
    keys: OkxCredentialProfileKeys,
) -> Option<OkxCredentialProfile> {
    complete_profile(&lookup, keys.live).or_else(|| complete_profile(&lookup, keys.normal))
}

fn complete_profile(
    lookup: &impl Fn(&str) -> Option<String>,
    keys: OkxProfileKeys,
) -> Option<OkxCredentialProfile> {
    Some(OkxCredentialProfile {
        api_key: lookup(keys.api_key)?,
        api_secret: lookup(keys.api_secret)?,
        passphrase: lookup(keys.passphrase)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let profile = select_okx_profile(
            lookup_from(&[
                ("OKX_API_KEY", "normal-key"),
                ("OKX_API_SECRET", "normal-secret"),
                ("OKX_PASSPHRASE", "normal-pass"),
                ("OKX_LIVE_API_KEY", "live-key"),
                ("OKX_LIVE_API_SECRET", "live-secret"),
                ("OKX_LIVE_PASSPHRASE", "live-pass"),
            ]),
            ENV_KEYS,
        );

        assert_eq!(
            profile,
            Some(OkxCredentialProfile {
                api_key: "live-key".to_owned(),
                api_secret: "live-secret".to_owned(),
                passphrase: "live-pass".to_owned(),
            })
        );
    }

    #[test]
    fn okx_profile_never_mixes_partial_live_with_normal() {
        let profile = select_okx_profile(
            lookup_from(&[
                ("OKX_API_KEY", "normal-key"),
                ("OKX_API_SECRET", "normal-secret"),
                ("OKX_PASSPHRASE", "normal-pass"),
                ("OKX_LIVE_API_KEY", "live-key"),
                ("OKX_LIVE_API_SECRET", "live-secret"),
            ]),
            ENV_KEYS,
        );

        assert_eq!(
            profile,
            Some(OkxCredentialProfile {
                api_key: "normal-key".to_owned(),
                api_secret: "normal-secret".to_owned(),
                passphrase: "normal-pass".to_owned(),
            })
        );
    }

    #[test]
    fn okx_profile_uses_normal_when_no_live_fields() {
        let profile = select_okx_profile(
            lookup_from(&[
                ("OKX_API_KEY", "normal-key"),
                ("OKX_API_SECRET", "normal-secret"),
                ("OKX_PASSPHRASE", "normal-pass"),
            ]),
            ENV_KEYS,
        );

        assert_eq!(
            profile,
            Some(OkxCredentialProfile {
                api_key: "normal-key".to_owned(),
                api_secret: "normal-secret".to_owned(),
                passphrase: "normal-pass".to_owned(),
            })
        );
    }

    #[test]
    fn okx_profile_none_when_normal_incomplete_and_no_live() {
        let profile = select_okx_profile(
            lookup_from(&[
                ("OKX_API_KEY", "normal-key"),
                ("OKX_API_SECRET", "normal-secret"),
            ]),
            ENV_KEYS,
        );

        assert_eq!(profile, None);
    }

    #[test]
    fn okx_profile_supports_save_form_field_keys() {
        let profile = select_okx_profile(
            lookup_from(&[
                ("api_key", "normal-key"),
                ("api_secret", "normal-secret"),
                ("passphrase", "normal-pass"),
                ("live_key", "live-key"),
                ("live_secret", "live-secret"),
                ("live_passphrase", "live-pass"),
            ]),
            FIELD_KEYS,
        );

        assert_eq!(
            profile,
            Some(OkxCredentialProfile {
                api_key: "live-key".to_owned(),
                api_secret: "live-secret".to_owned(),
                passphrase: "live-pass".to_owned(),
            })
        );
    }
}
