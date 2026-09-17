use super::specs::{FieldSpec, VenueSpec};
use super::{
    find_spec, status_for_spec, storage, validation_evidence_store, CredentialUpdateError,
};
use shared_types::{
    VenueCredentialClearRequest, VenueCredentialMaintenanceOperation,
    VenueCredentialMaintenanceResponse, VenueCredentialMigrateRequest,
};

pub(crate) async fn clear(
    request: VenueCredentialClearRequest,
) -> Result<VenueCredentialMaintenanceResponse, CredentialUpdateError> {
    let spec = find_spec(&request.venue)?;
    let selected = selected_fields(spec, request.fields)?;
    if selected.is_empty() {
        return Err(CredentialUpdateError::NoFields);
    }
    let env_keys = selected
        .iter()
        .map(|field| field.env_key.to_owned())
        .collect::<Vec<_>>();
    let inherited_environment = selected
        .iter()
        .any(|field| storage::environment_fallback_present(field.env_key));
    storage::clear_fields(&env_keys).await?;
    validation_evidence_store().remove(spec.venue);
    crate::services::trading_credentials::refresh_credential_fingerprint(spec.venue);
    let mut message = storage::clear_message(selected.len(), spec.label);
    if inherited_environment {
        message.push_str("；环境变量来源已在当前进程屏蔽，重启后仍由进程环境决定");
    }
    Ok(maintenance_response(
        spec,
        VenueCredentialMaintenanceOperation::Clear,
        selected.iter().map(|field| field.key.to_owned()).collect(),
        status_for_spec(spec).missing_fields,
        message,
    ))
}

pub(crate) async fn migrate(
    request: VenueCredentialMigrateRequest,
) -> Result<VenueCredentialMaintenanceResponse, CredentialUpdateError> {
    let spec = find_spec(&request.venue)?;
    let env_keys = spec
        .fields
        .iter()
        .map(|field| field.env_key.to_owned())
        .collect::<Vec<_>>();
    let migration = storage::migrate_fields(&env_keys).await?;
    crate::services::trading_credentials::refresh_credential_fingerprint(spec.venue);
    let migrated = logical_field_keys(spec, &migration.migrated_keys);
    let missing = logical_required_field_keys(spec, &migration.missing_keys);
    Ok(maintenance_response(
        spec,
        VenueCredentialMaintenanceOperation::Migrate,
        migrated.clone(),
        missing,
        storage::migration_message(migrated.len(), spec.label),
    ))
}

fn selected_fields(
    spec: &VenueSpec,
    fields: Vec<String>,
) -> Result<Vec<&FieldSpec>, CredentialUpdateError> {
    fields
        .into_iter()
        .map(|key| {
            spec.fields
                .iter()
                .find(|field| field.key == key.trim())
                .ok_or_else(|| CredentialUpdateError::UnknownField {
                    venue: spec.venue.to_owned(),
                    field: key,
                })
        })
        .collect()
}

fn logical_field_keys(spec: &VenueSpec, env_keys: &[String]) -> Vec<String> {
    spec.fields
        .iter()
        .filter(|field| env_keys.iter().any(|key| key == field.env_key))
        .map(|field| field.key.to_owned())
        .collect()
}

fn logical_required_field_keys(spec: &VenueSpec, env_keys: &[String]) -> Vec<String> {
    spec.fields
        .iter()
        .filter(|field| field.required && env_keys.iter().any(|key| key == field.env_key))
        .map(|field| field.key.to_owned())
        .collect()
}

fn maintenance_response(
    spec: &VenueSpec,
    operation: VenueCredentialMaintenanceOperation,
    affected_fields: Vec<String>,
    missing_fields: Vec<String>,
    message: String,
) -> VenueCredentialMaintenanceResponse {
    VenueCredentialMaintenanceResponse {
        venue: spec.venue.to_owned(),
        label: spec.label.to_owned(),
        operation,
        affected_fields,
        missing_fields,
        message,
        secret_storage: storage::status(),
        action_run_id: None,
        request_id: None,
    }
}
