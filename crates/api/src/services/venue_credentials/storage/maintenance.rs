use std::path::Path;

use super::{
    clear_backend_error, cleared_secrets, dotenv, dotenv_write_lock, keychain,
    persist_updates_to_backend, record_backend_error, runtime_secrets, secret_for_current_backend,
    CredentialUpdateError, MigrationResult, SecretBackend,
};

pub(super) async fn clear_fields(fields: &[String]) -> Result<(), CredentialUpdateError> {
    clear_fields_with_backend(SecretBackend::current(), fields, None).await
}

pub(super) async fn clear_fields_with_backend(
    backend: SecretBackend,
    fields: &[String],
    dotenv_path: Option<&Path>,
) -> Result<(), CredentialUpdateError> {
    match backend {
        SecretBackend::EnvFileAtomic => {
            remove_dotenv_fields(fields, dotenv_path).await?;
            mask_cleared(fields);
        }
        SecretBackend::Keychain => {
            keychain::remove_fields(fields)?;
            mask_cleared(fields);
            if let Err(error) = remove_dotenv_fields(fields, dotenv_path).await {
                record_backend_error(backend, &error);
                return Err(error);
            }
        }
        SecretBackend::RuntimeOnly => mask_cleared(fields),
    }
    clear_backend_error(backend);
    Ok(())
}

async fn remove_dotenv_fields(
    fields: &[String],
    dotenv_path: Option<&Path>,
) -> Result<(), CredentialUpdateError> {
    let _guard = dotenv_write_lock().lock().await;
    match dotenv_path {
        Some(path) => dotenv::remove_fields_from_dotenv_path(path, fields),
        None => dotenv::remove_fields_from_dotenv(fields),
    }
}

fn mask_cleared(fields: &[String]) {
    for key in fields {
        runtime_secrets().remove(key);
        cleared_secrets().insert(key.clone(), ());
    }
}

pub(super) async fn migrate_fields(
    fields: &[String],
) -> Result<MigrationResult, CredentialUpdateError> {
    migrate_fields_to_backend(SecretBackend::current(), fields, None).await
}

pub(super) async fn migrate_fields_to_backend(
    backend: SecretBackend,
    fields: &[String],
    dotenv_path: Option<&Path>,
) -> Result<MigrationResult, CredentialUpdateError> {
    if backend == SecretBackend::RuntimeOnly {
        return Err(CredentialUpdateError::MigrationUnavailable);
    }
    let mut updates = Vec::new();
    let mut missing_keys = Vec::new();
    for key in fields {
        match secret_for_current_backend(backend, key)? {
            Some(value) => updates.push((key.clone(), value)),
            None => missing_keys.push(key.clone()),
        }
    }
    if updates.is_empty() {
        return Err(CredentialUpdateError::NoFields);
    }
    persist_updates_to_backend(backend, &updates).await?;
    let migrated_keys = updates
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if backend == SecretBackend::Keychain {
        if let Err(error) = remove_dotenv_fields(&migrated_keys, dotenv_path).await {
            record_backend_error(backend, &error);
            return Err(error);
        }
    }
    Ok(MigrationResult {
        migrated_keys,
        missing_keys,
    })
}
