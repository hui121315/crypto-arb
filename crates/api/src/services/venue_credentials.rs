mod change_detection;
mod dotenv;
mod keychain;
mod maintenance;
mod specs;
mod storage;
mod validation;

use dashmap::DashMap;
use shared_types::{
    EnvTemplateLine, EnvTemplateResponse, SecretStorageStatus, VenueCredentialField,
    VenueCredentialFieldSource, VenueCredentialStatus, VenueCredentialUpdateRequest,
    VenueCredentialUpdateResponse, VenueCredentialValidationEvidence, VenueCredentialValue,
    VenueCredentialsResponse, VenueId,
};
use std::sync::OnceLock;
use thiserror::Error;

use specs::{FieldSpec, VenueSpec, SPECS};

pub(crate) use change_detection::update_changes_saved_values;
pub(crate) use maintenance::{clear, migrate};

pub(crate) fn status() -> VenueCredentialsResponse {
    VenueCredentialsResponse {
        venues: SPECS.iter().map(status_for_spec).collect(),
        secret_storage: storage::status(),
    }
}

pub(crate) fn env_template() -> EnvTemplateResponse {
    let lines = SPECS
        .iter()
        .flat_map(|spec| {
            spec.fields
                .iter()
                .map(move |field| template_line(spec, field))
        })
        .collect::<Vec<_>>();
    EnvTemplateResponse {
        text: env_template_text(&lines),
        lines,
    }
}

pub(crate) async fn update(
    request: VenueCredentialUpdateRequest,
) -> Result<VenueCredentialUpdateResponse, CredentialUpdateError> {
    let spec = find_spec(&request.venue)?;
    let validation_evidence = validation::validate(spec, &request.fields).await?;
    let saved = save_fields(spec, request.fields).await?;
    remember_validation_evidence(
        spec,
        validation_evidence.clone(),
        crate::services::trading_credentials::refresh_credential_fingerprint(spec.venue),
    );
    let configured_count = configured_count(spec);
    Ok(VenueCredentialUpdateResponse {
        venue: spec.venue.to_owned(),
        label: spec.label.to_owned(),
        configured_count,
        field_count: spec.fields.len(),
        message: storage::save_message(saved, spec.label),
        secret_storage: storage::status(),
        validation_evidence: Some(validation_evidence),
        action_run_id: None,
        request_id: None,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct VenueCredentialValidationSnapshot {
    pub(crate) venue: String,
    pub(crate) evidence: VenueCredentialValidationEvidence,
    pub(crate) credential_fingerprint: Option<String>,
}

#[derive(Debug, Clone)]
struct StoredValidationEvidence {
    evidence: VenueCredentialValidationEvidence,
    credential_fingerprint: Option<String>,
}

pub(crate) fn validation_evidence_snapshot() -> Vec<VenueCredentialValidationSnapshot> {
    let mut rows = validation_evidence_store()
        .iter()
        .map(|entry| VenueCredentialValidationSnapshot {
            venue: entry.key().clone(),
            evidence: entry.value().evidence.clone(),
            credential_fingerprint: entry.value().credential_fingerprint.clone(),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.venue.cmp(&right.venue));
    rows
}

pub(crate) fn secret(env_key: &str) -> Option<String> {
    storage::secret(env_key)
}

pub(crate) fn secret_source(env_key: &str) -> VenueCredentialFieldSource {
    storage::source(env_key)
}

pub(crate) fn secret_storage_status() -> SecretStorageStatus {
    storage::status()
}

pub(crate) async fn persist_secrets(
    updates: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    storage::persist_updates(updates).await
}

pub(crate) async fn clear_secrets(fields: &[String]) -> Result<(), CredentialUpdateError> {
    storage::clear_fields(fields).await
}

fn status_for_spec(spec: &VenueSpec) -> VenueCredentialStatus {
    let fields = spec.fields.iter().map(field_status).collect::<Vec<_>>();
    VenueCredentialStatus {
        venue: spec.venue.to_owned(),
        label: spec.label.to_owned(),
        missing_fields: fields
            .iter()
            .filter(|field| field.required && !field.configured)
            .map(|field| field.key.clone())
            .collect(),
        fields,
        public_market: true,
        private_read: spec.private_read,
        testnet_write: spec.testnet_write,
        live_write: spec.live_write,
        note: spec.note.to_owned(),
        validation_evidence: validation_evidence_store()
            .get(spec.venue)
            .map(|entry| entry.value().evidence.clone()),
    }
}

fn field_status(spec: &FieldSpec) -> VenueCredentialField {
    let source = storage::source(spec.env_key);
    VenueCredentialField {
        key: spec.key.to_owned(),
        label: spec.label.to_owned(),
        env_key: spec.env_key.to_owned(),
        configured: source != VenueCredentialFieldSource::Missing,
        secret: spec.secret,
        required: spec.required,
        source,
    }
}

fn template_line(spec: &VenueSpec, field: &FieldSpec) -> EnvTemplateLine {
    EnvTemplateLine {
        venue: spec.venue.to_owned(),
        field_label: format!("{} {}", spec.label, field.label),
        key: field.env_key.to_owned(),
        configured: env_present(field.env_key),
    }
}

fn env_template_text(lines: &[EnvTemplateLine]) -> String {
    let mut text = String::new();
    let mut current_venue = "";
    for line in lines {
        if line.venue != current_venue {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push('#');
            text.push(' ');
            text.push_str(&line.venue);
            text.push('\n');
            current_venue = &line.venue;
        }
        text.push_str(&line.key);
        text.push('=');
        text.push('\n');
    }
    text
}

fn env_present(key: &str) -> bool {
    secret(key).is_some()
}

fn configured_count(spec: &VenueSpec) -> usize {
    spec.fields
        .iter()
        .filter(|field| storage::source(field.env_key) != VenueCredentialFieldSource::Missing)
        .count()
}

fn find_spec(venue: &str) -> Result<&'static VenueSpec, CredentialUpdateError> {
    let Some(id) = VenueId::from_exchange_name(venue) else {
        return Err(CredentialUpdateError::UnknownVenue(venue.to_owned()));
    };
    SPECS
        .iter()
        .find(|spec| spec.id == id)
        .ok_or_else(|| CredentialUpdateError::UnknownVenue(venue.to_owned()))
}

async fn save_fields(
    spec: &VenueSpec,
    fields: Vec<VenueCredentialValue>,
) -> Result<usize, CredentialUpdateError> {
    let updates = credential_updates(spec, fields)?;
    if updates.is_empty() {
        return Err(CredentialUpdateError::NoFields);
    }
    storage::persist_updates(&updates).await?;
    Ok(updates.len())
}

fn credential_updates(
    spec: &VenueSpec,
    fields: Vec<VenueCredentialValue>,
) -> Result<Vec<(String, String)>, CredentialUpdateError> {
    let mut updates = Vec::with_capacity(fields.len());
    for field in fields {
        let value = field.value.trim();
        if value.is_empty() {
            continue;
        }
        let env_key = env_key_for(spec, &field.key)?;
        updates.push((env_key.to_owned(), value.to_owned()));
    }
    Ok(updates)
}

fn env_key_for<'a>(spec: &'a VenueSpec, key: &str) -> Result<&'a str, CredentialUpdateError> {
    spec.fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| field.env_key)
        .ok_or_else(|| CredentialUpdateError::UnknownField {
            venue: spec.venue.to_owned(),
            field: key.to_owned(),
        })
}

fn remember_validation_evidence(
    spec: &VenueSpec,
    evidence: VenueCredentialValidationEvidence,
    credential_fingerprint: Option<String>,
) {
    validation_evidence_store().insert(
        spec.venue.to_owned(),
        StoredValidationEvidence {
            evidence,
            credential_fingerprint,
        },
    );
}

fn validation_evidence_store() -> &'static DashMap<String, StoredValidationEvidence> {
    static STORE: OnceLock<DashMap<String, StoredValidationEvidence>> = OnceLock::new();
    STORE.get_or_init(DashMap::new)
}

#[derive(Debug, Error)]
pub(crate) enum CredentialUpdateError {
    #[error("unknown venue: {0}")]
    UnknownVenue(String),
    #[error("unknown field {field} for venue {venue}")]
    UnknownField { venue: String, field: String },
    #[error("missing credential field {field} for venue {venue}")]
    #[cfg_attr(test, allow(dead_code))]
    MissingField { venue: String, field: String },
    #[error("credential validation failed: {0}")]
    #[cfg_attr(test, allow(dead_code))]
    Validation(String),
    #[error("credential validation timed out")]
    #[cfg_attr(test, allow(dead_code))]
    ValidationTimeout,
    #[error("credential rejected by venue: {0}")]
    #[cfg_attr(test, allow(dead_code))]
    PermissionDenied(String),
    #[error("at least one credential field is required")]
    NoFields,
    #[error("credential migration requires a persistent secret backend")]
    MigrationUnavailable,
    #[error("failed to persist .env: {0}")]
    #[cfg_attr(test, allow(dead_code))]
    Persist(std::io::Error),
    #[error("failed to persist credential secret backend: {0}")]
    #[cfg_attr(test, allow(dead_code))]
    SecretBackend(String),
}

#[cfg(test)]
mod tests;
