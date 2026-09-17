use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageBackendKind {
    #[default]
    Unknown,
    Disabled,
    Memory,
    Postgres,
    Sqlite,
    Jsonl,
}

impl StorageBackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Disabled => "disabled",
            Self::Memory => "memory",
            Self::Postgres => "postgres",
            Self::Sqlite => "sqlite",
            Self::Jsonl => "jsonl",
        }
    }
}

impl fmt::Display for StorageBackendKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageDegradedReason {
    Disabled,
    Ephemeral,
    Fallback,
    Unavailable,
    ReadFailed,
    WriteFailed,
    ReplayFailed,
    SchemaDrift,
    MigrationUnapplied,
    MigrationFailed,
    ExtensionDegraded,
    Stale,
    Backpressure,
    PayloadInvalid,
}

impl StorageDegradedReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Ephemeral => "ephemeral",
            Self::Fallback => "fallback",
            Self::Unavailable => "unavailable",
            Self::ReadFailed => "read_failed",
            Self::WriteFailed => "write_failed",
            Self::ReplayFailed => "replay_failed",
            Self::SchemaDrift => "schema_drift",
            Self::MigrationUnapplied => "migration_unapplied",
            Self::MigrationFailed => "migration_failed",
            Self::ExtensionDegraded => "extension_degraded",
            Self::Stale => "stale",
            Self::Backpressure => "backpressure",
            Self::PayloadInvalid => "payload_invalid",
        }
    }
}

impl fmt::Display for StorageDegradedReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageRuntimeContract {
    #[serde(default)]
    pub backend_kind: StorageBackendKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degraded_reasons: Vec<StorageDegradedReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_authority: Option<StorageMigrationAuthority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageMigrationAuthority {
    pub migration_id: String,
    pub schema_name: String,
    pub migration_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_checksum: Option<String>,
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at_ms: Option<i64>,
}

impl StorageMigrationAuthority {
    pub fn matches(&self, schema_version: u32, migration_checksum: &str) -> bool {
        self.applied
            && self.schema_version == Some(schema_version)
            && self.migration_checksum.as_deref() == Some(migration_checksum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_contract_uses_stable_wire_labels() {
        assert_eq!(
            serde_json::to_string(&StorageBackendKind::Postgres).expect("serialize backend"),
            r#""postgres""#
        );
        assert_eq!(
            serde_json::to_string(&StorageDegradedReason::SchemaDrift)
                .expect("serialize degraded reason"),
            r#""schema_drift""#
        );
    }

    #[test]
    fn migration_authority_requires_applied_version_and_checksum() {
        let mut authority = StorageMigrationAuthority {
            migration_id: "20260701_history".into(),
            schema_name: "realtime_history".into(),
            migration_path: "history.sql".into(),
            schema_version: Some(2),
            migration_checksum: Some("fnv1a64:1234".into()),
            applied: true,
            applied_at_ms: Some(100),
        };

        assert!(authority.matches(2, "fnv1a64:1234"));
        authority.applied = false;
        assert!(!authority.matches(2, "fnv1a64:1234"));
    }

    #[test]
    fn legacy_payload_can_default_the_runtime_contract() {
        let contract: StorageRuntimeContract =
            serde_json::from_str("{}").expect("deserialize legacy storage contract");

        assert_eq!(contract.backend_kind, StorageBackendKind::Unknown);
        assert!(contract.degraded_reasons.is_empty());
        assert!(contract.migration_authority.is_none());
    }
}
