use super::*;

#[cfg(not(test))]
pub(super) struct FieldValues<'a> {
    pub(super) spec: &'a VenueSpec,
    pub(super) fields: &'a [VenueCredentialValue],
}

#[cfg(not(test))]
impl FieldValues<'_> {
    pub(super) fn get(&self, key: &str) -> Result<String, CredentialUpdateError> {
        self.optional(key)
            .ok_or_else(|| CredentialUpdateError::MissingField {
                venue: self.spec.venue.to_owned(),
                field: key.to_owned(),
            })
    }

    pub(super) fn optional(&self, key: &str) -> Option<String> {
        self.input_value(key).or_else(|| self.saved_value(key))
    }

    fn input_value(&self, key: &str) -> Option<String> {
        self.fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.value.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn saved_value(&self, key: &str) -> Option<String> {
        let Ok(env_key) = env_key_for(self.spec, key) else {
            return None;
        };
        secret(env_key)
    }

    pub(super) fn hyperliquid_account_address(&self) -> Result<String, CredentialUpdateError> {
        self.input_value("account_address")
            .or_else(|| secret("HYPERLIQUID_ACCOUNT_ADDRESS"))
            .or_else(|| self.input_value("user_address"))
            .or_else(|| secret("HYPERLIQUID_USER_ADDRESS"))
            .ok_or_else(|| CredentialUpdateError::MissingField {
                venue: self.spec.venue.to_owned(),
                field: "account_address".to_owned(),
            })
    }
}
