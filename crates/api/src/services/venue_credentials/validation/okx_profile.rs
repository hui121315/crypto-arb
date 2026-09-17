use crate::services::okx_credential_profile::{
    select_okx_profile, OkxCredentialProfile, FIELD_KEYS,
};

use super::*;

#[cfg(not(test))]
pub(super) fn required_okx_profile(
    values: &FieldValues<'_>,
) -> Result<OkxCredentialProfile, CredentialUpdateError> {
    select_okx_profile(|key| values.optional(key), FIELD_KEYS)
        .ok_or_else(|| missing_okx_profile_field(values))
}

#[cfg(not(test))]
fn missing_okx_profile_field(values: &FieldValues<'_>) -> CredentialUpdateError {
    CredentialUpdateError::MissingField {
        venue: values.spec.venue.to_owned(),
        field: missing_okx_field(values).to_owned(),
    }
}

#[cfg(not(test))]
fn missing_okx_field(values: &FieldValues<'_>) -> &'static str {
    let live_fields = ["live_key", "live_secret", "live_passphrase"];
    if live_fields.iter().any(|key| values.optional(key).is_some()) {
        return first_missing(values, live_fields).unwrap_or("live_key");
    }
    first_missing(values, ["api_key", "api_secret", "passphrase"]).unwrap_or("api_key")
}

#[cfg(not(test))]
fn first_missing<const N: usize>(
    values: &FieldValues<'_>,
    fields: [&'static str; N],
) -> Option<&'static str> {
    fields
        .into_iter()
        .find(|field| values.optional(field).is_none())
}
