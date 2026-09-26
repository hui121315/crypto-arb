use std::path::Path;

use super::{
    apply_changes_to_backend, dotenv_write_lock, secret_for_current_backend,
    CredentialUpdateError, MigrationResult, SecretBackend,
};

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
    let _guard = dotenv_write_lock().lock().await;
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
    apply_changes_to_backend(backend, &updates, &[], dotenv_path)?;
    let migrated_keys = updates
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    Ok(MigrationResult {
        migrated_keys,
        missing_keys,
    })
}
