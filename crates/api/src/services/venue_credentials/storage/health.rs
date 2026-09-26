use dashmap::DashMap;
use shared_types::SecretStorageStatus;
use std::sync::OnceLock;

use super::super::{keychain, CredentialUpdateError};
use super::SecretBackend;

static BACKEND_READ_ERRORS: OnceLock<DashMap<SecretBackend, String>> = OnceLock::new();

fn backend_read_errors() -> &'static DashMap<SecretBackend, String> {
    BACKEND_READ_ERRORS.get_or_init(DashMap::new)
}

pub(super) fn record_backend_error(backend: SecretBackend, error: &CredentialUpdateError) {
    backend_read_errors().insert(backend, error.to_string());
}

pub(super) fn clear_backend_error(backend: SecretBackend) {
    backend_read_errors().remove(&backend);
}

pub(super) fn apply_backend_warning(
    backend: SecretBackend,
    mut status: SecretStorageStatus,
) -> SecretStorageStatus {
    if let Some(error) = backend_read_errors().get(&backend) {
        status.message = "凭证存储读写异常；字段状态与上次操作结果分别显示，请修复存储后重试。".to_owned();
        status = status.with_backend_error(error.value().clone());
    }
    status
}

pub(in crate::services::venue_credentials) fn status() -> SecretStorageStatus {
    let backend = SecretBackend::current();
    apply_backend_warning(backend, status_for_backend(backend))
}

pub(super) fn status_for_backend(backend: SecretBackend) -> SecretStorageStatus {
    match backend {
        SecretBackend::EnvFileAtomic => SecretStorageStatus::env_file_atomic(dotenv_status_path()),
        SecretBackend::Keychain => keychain::storage_status(),
        SecretBackend::RuntimeOnly => SecretStorageStatus::runtime_only(),
    }
}

#[cfg(not(test))]
fn dotenv_status_path() -> Option<String> {
    match super::super::dotenv::env_file_path() {
        Ok(path) => Some(path.display().to_string()),
        Err(error) => {
            record_backend_error(SecretBackend::EnvFileAtomic, &error);
            None
        }
    }
}

#[cfg(test)]
fn dotenv_status_path() -> Option<String> {
    None
}

pub(in crate::services::venue_credentials) fn save_message(
    saved: usize,
    venue_label: &str,
) -> String {
    let storage = status();
    format!(
        "已保存 {saved} 个字段，{venue_label} adapter 已刷新；{}",
        storage.message
    )
}

pub(in crate::services::venue_credentials) fn clear_message(
    cleared: usize,
    venue_label: &str,
) -> String {
    format!(
        "已清空 {cleared} 个 {venue_label} 凭证字段，并使相关运行态证明失效；{}",
        status().message
    )
}

pub(in crate::services::venue_credentials) fn migration_message(
    migrated: usize,
    venue_label: &str,
) -> String {
    format!(
        "已迁移 {migrated} 个 {venue_label} 凭证字段到当前 secret backend，并刷新 adapter；{}",
        status().message
    )
}
