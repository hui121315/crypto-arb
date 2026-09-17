use shared_types::SecretStorageStatus;

use super::CredentialUpdateError;

pub(super) const SERVICE: &str = "com.crossline.crypto-arb.venue-credentials";

pub(super) fn persist_fields(updates: &[(String, String)]) -> Result<(), CredentialUpdateError> {
    platform::persist_fields(updates)
}

pub(super) fn secret(env_key: &str) -> Result<Option<String>, CredentialUpdateError> {
    platform::secret(env_key)
}

pub(super) fn remove_fields(fields: &[String]) -> Result<(), CredentialUpdateError> {
    platform::remove_fields(fields)
}

pub(super) fn storage_status() -> SecretStorageStatus {
    platform::storage_status()
}

#[cfg(all(target_os = "macos", not(test)))]
mod platform {
    use super::*;

    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25_300;

    pub(super) fn persist_fields(
        updates: &[(String, String)],
    ) -> Result<(), CredentialUpdateError> {
        for (key, value) in updates {
            security_framework::passwords::set_generic_password(SERVICE, key, value.as_bytes())
                .map_err(|error| CredentialUpdateError::SecretBackend(error.to_string()))?;
        }
        Ok(())
    }

    pub(super) fn secret(env_key: &str) -> Result<Option<String>, CredentialUpdateError> {
        let bytes = match security_framework::passwords::get_generic_password(SERVICE, env_key) {
            Ok(bytes) => bytes,
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => return Ok(None),
            Err(error) => {
                return Err(CredentialUpdateError::SecretBackend(error.to_string()));
            }
        };
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| CredentialUpdateError::SecretBackend(error.to_string()))
    }

    pub(super) fn remove_fields(fields: &[String]) -> Result<(), CredentialUpdateError> {
        for key in fields {
            if secret(key)?.is_some() {
                security_framework::passwords::delete_generic_password(SERVICE, key)
                    .map_err(|error| CredentialUpdateError::SecretBackend(error.to_string()))?;
            }
        }
        Ok(())
    }

    pub(super) fn storage_status() -> SecretStorageStatus {
        SecretStorageStatus::keychain(SERVICE)
    }
}

#[cfg(all(not(target_os = "macos"), not(test)))]
mod platform {
    use super::*;

    pub(super) fn persist_fields(
        _updates: &[(String, String)],
    ) -> Result<(), CredentialUpdateError> {
        Err(CredentialUpdateError::SecretBackend(
            "macOS Keychain backend is unavailable on this platform".to_owned(),
        ))
    }

    pub(super) fn secret(_env_key: &str) -> Result<Option<String>, CredentialUpdateError> {
        Err(CredentialUpdateError::SecretBackend(
            "macOS Keychain backend is unavailable on this platform".to_owned(),
        ))
    }

    pub(super) fn remove_fields(_fields: &[String]) -> Result<(), CredentialUpdateError> {
        Err(CredentialUpdateError::SecretBackend(
            "macOS Keychain backend is unavailable on this platform".to_owned(),
        ))
    }

    pub(super) fn storage_status() -> SecretStorageStatus {
        SecretStorageStatus::keychain_unavailable(SERVICE)
    }
}

#[cfg(test)]
mod platform {
    use dashmap::DashMap;
    use std::sync::OnceLock;

    use super::*;

    static SECRETS: OnceLock<DashMap<String, String>> = OnceLock::new();

    fn secrets() -> &'static DashMap<String, String> {
        SECRETS.get_or_init(DashMap::new)
    }

    pub(super) fn persist_fields(
        updates: &[(String, String)],
    ) -> Result<(), CredentialUpdateError> {
        for (key, value) in updates {
            if key.is_empty() {
                return Err(CredentialUpdateError::SecretBackend(
                    "empty keychain account".to_owned(),
                ));
            }
            secrets().insert(key.clone(), value.clone());
        }
        Ok(())
    }

    pub(super) fn secret(env_key: &str) -> Result<Option<String>, CredentialUpdateError> {
        if env_key.is_empty() {
            return Err(CredentialUpdateError::SecretBackend(
                "empty keychain account".to_owned(),
            ));
        }
        Ok(secrets().get(env_key).map(|value| value.value().clone()))
    }

    pub(super) fn remove_fields(fields: &[String]) -> Result<(), CredentialUpdateError> {
        if fields.iter().any(|key| key.is_empty()) {
            return Err(CredentialUpdateError::SecretBackend(
                "empty keychain account".to_owned(),
            ));
        }
        for key in fields {
            secrets().remove(key);
        }
        Ok(())
    }

    pub(super) fn storage_status() -> SecretStorageStatus {
        SecretStorageStatus::keychain(SERVICE)
    }
}
