use super::*;

pub(crate) fn update_changes_saved_values(
    request: &VenueCredentialUpdateRequest,
) -> Result<bool, CredentialUpdateError> {
    let spec = find_spec(&request.venue)?;
    for field in &request.fields {
        let value = field.value.trim();
        if value.is_empty() {
            continue;
        }
        let env_key = env_key_for(spec, &field.key)?;
        if storage::secret(env_key).as_deref() != Some(value) {
            return Ok(true);
        }
    }
    Ok(false)
}
