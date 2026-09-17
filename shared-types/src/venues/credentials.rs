use super::*;

mod permissions;

pub use permissions::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VenueCredentialFieldSource {
    #[default]
    Missing,
    Environment,
    EnvFile,
    Keychain,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialField {
    pub key: String,
    pub label: String,
    pub env_key: String,
    pub configured: bool,
    pub secret: bool,
    #[serde(default = "credential_field_required_default")]
    pub required: bool,
    #[serde(default)]
    pub source: VenueCredentialFieldSource,
}

const fn credential_field_required_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialStatus {
    pub venue: String,
    pub label: String,
    pub fields: Vec<VenueCredentialField>,
    pub public_market: bool,
    pub private_read: bool,
    pub testnet_write: bool,
    pub live_write: bool,
    pub note: String,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_evidence: Option<VenueCredentialValidationEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueCredentialValidationStatus {
    ReadOnlyOk,
    LocalOnly,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueCredentialProbeStatus {
    Ok,
    /// 探针被交易所明确拒绝（认证/权限失败）。与 `Unknown`（瞬时/未证明、可重试）区分，
    /// 保证“被拒绝”不会在保存响应里伪装成中性的“未知”。
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialProbe {
    pub kind: String,
    pub status: VenueCredentialProbeStatus,
    pub scope: String,
    pub source: String,
    pub message: String,
    pub checked_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialValidationEvidence {
    pub status: VenueCredentialValidationStatus,
    pub checked_at_ms: i64,
    pub probes: Vec<VenueCredentialProbe>,
    #[serde(default)]
    pub permission_evidence: Vec<VenueCredentialPermissionEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretStorageMode {
    EnvFileAtomic,
    Keychain,
    RuntimeOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecretStorageHealth {
    Ready,
    Degraded,
    Unavailable,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretStorageStatus {
    pub mode: SecretStorageMode,
    #[serde(default)]
    pub health: SecretStorageHealth,
    pub persistent: bool,
    pub encrypted: bool,
    pub atomic_write: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub label: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl SecretStorageStatus {
    pub fn env_file_atomic(path: Option<String>) -> Self {
        Self {
            mode: SecretStorageMode::EnvFileAtomic,
            health: SecretStorageHealth::Degraded,
            persistent: true,
            encrypted: false,
            atomic_write: true,
            path,
            label: "本地 .env + 进程内缓存".to_owned(),
            message: "凭证会写入本地 .env 并同时刷新当前进程缓存。".to_owned(),
            warning: Some(
                "本地 .env 不是加密 secret store；长期实盘建议迁移到系统 Keychain 或专用密钥库。"
                    .to_owned(),
            ),
            last_error: None,
        }
    }

    pub fn keychain(service: impl Into<String>) -> Self {
        let service = service.into();
        Self {
            mode: SecretStorageMode::Keychain,
            health: SecretStorageHealth::Ready,
            persistent: true,
            encrypted: true,
            atomic_write: true,
            path: Some(format!("service:{service}")),
            label: "macOS Keychain".to_owned(),
            message: "系统 Keychain 已启用，凭证通过登录会话加密持久化".to_owned(),
            warning: None,
            last_error: None,
        }
    }

    pub fn keychain_unavailable(service: impl Into<String>) -> Self {
        let service = service.into();
        Self {
            mode: SecretStorageMode::Keychain,
            health: SecretStorageHealth::Unavailable,
            persistent: false,
            encrypted: false,
            atomic_write: false,
            path: Some(format!("service:{service}")),
            label: "macOS Keychain".to_owned(),
            message: "当前平台不支持 macOS Keychain，无法加密持久化凭证".to_owned(),
            warning: Some("切回 APP_CREDENTIALS__SECRET_BACKEND=env_file 或 runtime".to_owned()),
            last_error: Some("macOS Keychain backend is unavailable on this platform".to_owned()),
        }
    }

    pub fn runtime_only() -> Self {
        Self {
            mode: SecretStorageMode::RuntimeOnly,
            health: SecretStorageHealth::Degraded,
            persistent: false,
            encrypted: false,
            atomic_write: false,
            path: None,
            label: "仅进程内缓存".to_owned(),
            message: "凭证只保存在当前进程内，重启后需要重新配置。".to_owned(),
            warning: Some("当前没有持久化 secret backend。".to_owned()),
            last_error: None,
        }
    }

    pub fn with_backend_error(mut self, error: impl Into<String>) -> Self {
        let error = error.into();
        self.health = SecretStorageHealth::Unavailable;
        self.warning = Some(format!("Secret backend error: {error}"));
        self.last_error = Some(error);
        self
    }
}

impl Default for SecretStorageStatus {
    fn default() -> Self {
        Self::runtime_only()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialsResponse {
    pub venues: Vec<VenueCredentialStatus>,
    #[serde(default)]
    pub secret_storage: SecretStorageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialValue {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialUpdateRequest {
    pub venue: String,
    pub fields: Vec<VenueCredentialValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialUpdateResponse {
    pub venue: String,
    pub label: String,
    pub configured_count: usize,
    pub field_count: usize,
    pub message: String,
    #[serde(default)]
    pub secret_storage: SecretStorageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_evidence: Option<VenueCredentialValidationEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialClearRequest {
    pub venue: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialMigrateRequest {
    pub venue: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueCredentialMaintenanceOperation {
    Clear,
    Migrate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VenueCredentialMaintenanceResponse {
    pub venue: String,
    pub label: String,
    pub operation: VenueCredentialMaintenanceOperation,
    pub affected_fields: Vec<String>,
    pub missing_fields: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub secret_storage: SecretStorageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[cfg(test)]
#[path = "credentials/tests.rs"]
mod tests;
