#![allow(clippy::expect_used)]

use super::*;

#[test]
fn backpack_stock_credentials_require_a_complete_matching_pair_without_reading_secrets(){
    use base64::{Engine,engine::general_purpose::STANDARD};
    let public=STANDARD.encode(common::signing::ed25519_public_key(&[7;32]).unwrap());
    let fields=|secret:&str|vec![VenueCredentialValue{key:"api_key".into(),value:public.clone()},VenueCredentialValue{key:"secret_key".into(),value:secret.into()}];
    let spec=find_provider("backpack_stocks").unwrap();
    assert!(credential_updates(spec,fields(&STANDARD.encode([7;32]))).is_ok());
    assert!(credential_updates(spec,fields(&STANDARD.encode([8;32]))).is_err());
    assert!(credential_updates(spec,vec![VenueCredentialValue{key:"api_key".into(),value:public}]).is_err());
    let (_,clear)=clear_fields(spec,&[]).unwrap();assert_eq!(clear,vec!["api_key","secret_key"]);
}

#[test]
fn update_contract_maps_only_official_provider_fields() {
    let spec = find_provider("okx_dex_v6").expect("known OKX provider");
    let (updates, affected) = credential_updates(
        spec,
        vec![
            VenueCredentialValue {
                key: "api_key".to_owned(),
                value: " key ".to_owned(),
            },
            VenueCredentialValue {
                key: "secret_key".to_owned(),
                value: " secret ".to_owned(),
            },
            VenueCredentialValue {
                key: "passphrase".to_owned(),
                value: " phrase ".to_owned(),
            },
        ],
    )
    .expect("official OKX fields map");

    assert_eq!(updates[0], ("OKX_DEX_API_KEY".to_owned(), "key".to_owned()));
    assert_eq!(updates[1].0, "OKX_DEX_SECRET_KEY");
    assert_eq!(updates[2].0, "OKX_DEX_PASSPHRASE");
    assert_eq!(affected, ["api_key", "secret_key", "passphrase"]);
}

#[test]
fn update_contract_rejects_unknown_or_empty_fields() {
    let spec = find_provider("zeroex_swap_v2").expect("known 0x provider");
    assert!(matches!(
        credential_updates(
            spec,
            vec![VenueCredentialValue {
                key: "private_key".to_owned(),
                value: "never".to_owned(),
            }]
        ),
        Err(ProviderCredentialError::UnknownField { .. })
    ));
    assert!(matches!(
        credential_updates(
            spec,
            vec![VenueCredentialValue {
                key: "api_key".to_owned(),
                value: "  ".to_owned(),
            }]
        ),
        Err(ProviderCredentialError::NoFields)
    ));
}

#[test]
fn empty_clear_request_expands_to_every_provider_field() {
    let spec = find_provider("okx_dex_v6").expect("known OKX provider");
    let (env_keys, affected) = clear_fields(spec, &[]).expect("clear all fields");

    assert_eq!(env_keys.len(), 3);
    assert_eq!(affected, ["api_key", "secret_key", "passphrase"]);
}

#[test]
fn jupiter_keyed_route_requires_the_official_header_secret() {
    let spec = find_provider("jupiter_swap_v2_keyed").expect("known Jupiter provider");

    assert_eq!(spec.official_docs_url, JUPITER_DOCS);
    assert_eq!(spec.fields.len(), 1);
    assert!(spec.fields[0].required);
    assert!(spec.fields[0].secret);
    assert_eq!(spec.fields[0].env_key, "JUPITER_API_KEY");
}

#[test]
fn lifi_key_is_optional_and_maps_to_the_server_only_header_secret() {
    let spec = find_provider("lifi").expect("known LI.FI provider");

    assert_eq!(spec.official_docs_url, LIFI_DOCS);
    assert_eq!(spec.fields.len(), 1);
    assert!(!spec.fields[0].required);
    assert!(spec.fields[0].secret);
    assert_eq!(spec.fields[0].env_key, "LIFI_API_KEY");
    let (updates, _) = credential_updates(
        spec,
        vec![VenueCredentialValue {
            key: "api_key".to_owned(),
            value: "server-secret".to_owned(),
        }],
    )
    .expect("optional key can still be stored");
    assert_eq!(updates[0].0, "LIFI_API_KEY");
}
