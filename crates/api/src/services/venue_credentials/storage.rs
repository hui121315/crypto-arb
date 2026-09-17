use dashmap::DashMap;
use shared_types::VenueCredentialFieldSource;
use std::sync::OnceLock;
use tokio::sync::Mutex;

use super::{dotenv, keychain, CredentialUpdateError};
use health::{clear_backend_error, record_backend_error};

pub(super) use health::{clear_message, migration_message, save_message, status};

const SECRET_BACKEND_ENV: &str = "APP_CREDENTIALS__SECRET_BACKEND";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SecretBackend {
    EnvFileAtomic,
    Keychain,
    RuntimeOnly,
}

impl SecretBackend {
    fn current() -> Self {
        match std::env::var(SECRET_BACKEND_ENV) {
            Ok(value) => Self::from_env_value(&value),
            Err(_) => Self::EnvFileAtomic,
        }
    }

    fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "env" | "env_file" | "env-file" | "dotenv" => Self::EnvFileAtomic,
            "keychain" | "macos_keychain" | "macos-keychain" => Self::Keychain,
            "runtime" | "runtime_only" | "runtime-only" => Self::RuntimeOnly,
            _ => Self::EnvFileAtomic,
        }
    }
}

#[derive(Clone)]
struct RuntimeSecret {
    value: String,
    source: VenueCredentialFieldSource,
}

#[derive(Debug, Default)]
pub(super) struct MigrationResult {
    pub(super) migrated_keys: Vec<String>,
    pub(super) missing_keys: Vec<String>,
}

static RUNTIME_SECRETS: OnceLock<DashMap<String, RuntimeSecret>> = OnceLock::new();
static CLEARED_SECRETS: OnceLock<DashMap<String, ()>> = OnceLock::new();
static DOTENV_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn runtime_secrets() -> &'static DashMap<String, RuntimeSecret> {
    RUNTIME_SECRETS.get_or_init(DashMap::new)
}

fn cleared_secrets() -> &'static DashMap<String, ()> {
    CLEARED_SECRETS.get_or_init(DashMap::new)
}

fn dotenv_write_lock() -> &'static Mutex<()> {
    DOTENV_WRITE_LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) fn secret(env_key: &str) -> Option<String> {
    let backend = SecretBackend::current();
    match secret_for_current_backend(backend, env_key) {
        Ok(value) => value,
        Err(error) => {
            record_backend_error(backend, &error);
            None
        }
    }
}

fn secret_for_current_backend(
    backend: SecretBackend,
    env_key: &str,
) -> Result<Option<String>, CredentialUpdateError> {
    if cleared_secrets().contains_key(env_key) {
        return Ok(None);
    }
    if let Some(value) = runtime_secrets().get(env_key) {
        return Ok(Some(value.value().value.clone()));
    }
    let value = backend_secret(backend, env_key)?;
    if value.is_some() {
        clear_backend_error(backend);
    }
    Ok(value)
}

pub(super) fn source(env_key: &str) -> VenueCredentialFieldSource {
    let backend = SecretBackend::current();
    match source_for_current_backend(backend, env_key) {
        Ok(source) => source,
        Err(error) => {
            record_backend_error(backend, &error);
            VenueCredentialFieldSource::Missing
        }
    }
}

fn source_for_current_backend(
    backend: SecretBackend,
    env_key: &str,
) -> Result<VenueCredentialFieldSource, CredentialUpdateError> {
    if cleared_secrets().contains_key(env_key) {
        return Ok(VenueCredentialFieldSource::Missing);
    }
    if let Some(value) = runtime_secrets().get(env_key) {
        return Ok(value.value().source);
    }
    backend_source(backend, env_key)
}

fn backend_secret(
    backend: SecretBackend,
    env_key: &str,
) -> Result<Option<String>, CredentialUpdateError> {
    match backend {
        SecretBackend::EnvFileAtomic => Ok(env_secret(env_key)),
        SecretBackend::Keychain => match keychain::secret(env_key)? {
            Some(value) => Ok(Some(value)),
            None => Ok(env_secret(env_key)),
        },
        SecretBackend::RuntimeOnly => Ok(None),
    }
}

fn backend_source(
    backend: SecretBackend,
    env_key: &str,
) -> Result<VenueCredentialFieldSource, CredentialUpdateError> {
    match backend {
        SecretBackend::EnvFileAtomic => Ok(env_secret(env_key)
            .map(|_| VenueCredentialFieldSource::Environment)
            .unwrap_or_default()),
        SecretBackend::Keychain => Ok(if keychain::secret(env_key)?.is_some() {
            VenueCredentialFieldSource::Keychain
        } else if env_secret(env_key).is_some() {
            VenueCredentialFieldSource::Environment
        } else {
            VenueCredentialFieldSource::Missing
        }),
        SecretBackend::RuntimeOnly => Ok(VenueCredentialFieldSource::Missing),
    }
}

fn env_secret(env_key: &str) -> Option<String> {
    match std::env::var(env_key) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        Ok(_) | Err(_) => None,
    }
}

pub(super) fn environment_fallback_present(env_key: &str) -> bool {
    env_secret(env_key).is_some()
}

pub(super) async fn persist_updates(
    updates: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    let backend = SecretBackend::current();
    persist_updates_to_backend(backend, updates).await
}

async fn persist_updates_to_backend(
    backend: SecretBackend,
    updates: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    match backend {
        SecretBackend::EnvFileAtomic => {
            let _guard = dotenv_write_lock().lock().await;
            dotenv::persist_fields_to_dotenv(updates)?;
        }
        SecretBackend::Keychain => {
            keychain::persist_fields(updates)?;
        }
        SecretBackend::RuntimeOnly => {}
    }
    clear_backend_error(backend);
    remember_runtime(updates, source_for_backend(backend));
    Ok(())
}

pub(super) async fn clear_fields(fields: &[String]) -> Result<(), CredentialUpdateError> {
    maintenance::clear_fields(fields).await
}

#[cfg(test)]
async fn clear_fields_with_backend(
    backend: SecretBackend,
    fields: &[String],
    dotenv_path: Option<&std::path::Path>,
) -> Result<(), CredentialUpdateError> {
    maintenance::clear_fields_with_backend(backend, fields, dotenv_path).await
}

pub(super) async fn migrate_fields(
    fields: &[String],
) -> Result<MigrationResult, CredentialUpdateError> {
    maintenance::migrate_fields(fields).await
}

#[cfg(test)]
async fn migrate_fields_to_backend(
    backend: SecretBackend,
    fields: &[String],
    dotenv_path: Option<&std::path::Path>,
) -> Result<MigrationResult, CredentialUpdateError> {
    maintenance::migrate_fields_to_backend(backend, fields, dotenv_path).await
}

fn remember_runtime(updates: &[(String, String)], source: VenueCredentialFieldSource) {
    for (key, value) in updates {
        runtime_secrets().insert(
            key.clone(),
            RuntimeSecret {
                value: value.clone(),
                source,
            },
        );
        cleared_secrets().remove(key);
    }
}

fn source_for_backend(backend: SecretBackend) -> VenueCredentialFieldSource {
    match backend {
        SecretBackend::EnvFileAtomic => VenueCredentialFieldSource::EnvFile,
        SecretBackend::Keychain => VenueCredentialFieldSource::Keychain,
        SecretBackend::RuntimeOnly => VenueCredentialFieldSource::Runtime,
    }
}

#[cfg(test)]
pub(super) fn persist_updates_for_backend(
    backend: SecretBackend,
    updates: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    match backend {
        SecretBackend::EnvFileAtomic => dotenv::persist_fields_to_dotenv(updates)?,
        SecretBackend::Keychain => keychain::persist_fields(updates)?,
        SecretBackend::RuntimeOnly => {}
    }
    clear_backend_error(backend);
    remember_runtime(updates, source_for_backend(backend));
    Ok(())
}

#[cfg(test)]
pub(super) fn secret_for_backend(backend: SecretBackend, env_key: &str) -> Option<String> {
    if cleared_secrets().contains_key(env_key) {
        return None;
    }
    match secret_for_current_backend(backend, env_key) {
        Ok(value) => value,
        Err(error) => {
            record_backend_error(backend, &error);
            None
        }
    }
}

#[cfg(test)]
#[path = "storage/tests.rs"]
mod tests;

mod health;
mod maintenance;

#[cfg(test)]
use health::{apply_backend_warning, status_for_backend};
