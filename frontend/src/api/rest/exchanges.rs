use super::*;
use shared_types::{
    VenueCredentialClearRequest, VenueCredentialMaintenanceResponse, VenueCredentialMigrateRequest,
    VenueCredentialUpdateRequest, VenueCredentialUpdateResponse, VenueCredentialValue,
};

impl ApiClient {
    pub async fn venue_credentials(
        &self,
    ) -> Result<shared_types::VenueCredentialsResponse, ApiError> {
        self.get_json("/api/exchanges/credentials").await
    }

    pub async fn save_venue_credentials(
        &self,
        venue: &str,
        fields: &[(String, String)],
    ) -> Result<VenueCredentialUpdateResponse, ApiError> {
        self.save_venue_credentials_request(venue, fields, None)
            .await
    }

    pub async fn save_venue_credentials_with_idempotency_key(
        &self,
        venue: &str,
        fields: &[(String, String)],
        idempotency_key: &str,
    ) -> Result<VenueCredentialUpdateResponse, ApiError> {
        self.save_venue_credentials_with_context(
            venue,
            fields,
            &MutationRequestContext::with_idempotency_key(idempotency_key),
        )
        .await
    }

    pub(crate) async fn save_venue_credentials_with_context(
        &self,
        venue: &str,
        fields: &[(String, String)],
        context: &MutationRequestContext,
    ) -> Result<VenueCredentialUpdateResponse, ApiError> {
        let fields = fields
            .iter()
            .map(|(key, value)| VenueCredentialValue {
                key: key.clone(),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        let request = VenueCredentialUpdateRequest {
            venue: venue.to_owned(),
            fields,
        };
        self.post_json_with_context("/api/exchanges/credentials", &request, context)
            .await
    }

    async fn save_venue_credentials_request(
        &self,
        venue: &str,
        fields: &[(String, String)],
        idempotency_key: Option<&str>,
    ) -> Result<VenueCredentialUpdateResponse, ApiError> {
        let fields = fields
            .iter()
            .map(|(key, value)| VenueCredentialValue {
                key: key.clone(),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        let request = VenueCredentialUpdateRequest {
            venue: venue.to_owned(),
            fields,
        };
        match idempotency_key {
            Some(key) => {
                self.post_json_with_idempotency_key("/api/exchanges/credentials", &request, key)
                    .await
            }
            None => self.post_json("/api/exchanges/credentials", &request).await,
        }
    }

    pub async fn clear_venue_credentials(
        &self,
        venue: &str,
        fields: &[String],
    ) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
        self.clear_venue_credentials_request(venue, fields, None)
            .await
    }

    pub async fn clear_venue_credentials_with_idempotency_key(
        &self,
        venue: &str,
        fields: &[String],
        idempotency_key: &str,
    ) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
        self.clear_venue_credentials_with_context(
            venue,
            fields,
            &MutationRequestContext::with_idempotency_key(idempotency_key),
        )
        .await
    }

    pub(crate) async fn clear_venue_credentials_with_context(
        &self,
        venue: &str,
        fields: &[String],
        context: &MutationRequestContext,
    ) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
        let request = VenueCredentialClearRequest {
            venue: venue.to_owned(),
            fields: fields.to_vec(),
        };
        self.post_json_with_context("/api/exchanges/credentials/clear", &request, context)
            .await
    }

    async fn clear_venue_credentials_request(
        &self,
        venue: &str,
        fields: &[String],
        idempotency_key: Option<&str>,
    ) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
        let request = VenueCredentialClearRequest {
            venue: venue.to_owned(),
            fields: fields.to_vec(),
        };
        match idempotency_key {
            Some(key) => {
                self.post_json_with_idempotency_key(
                    "/api/exchanges/credentials/clear",
                    &request,
                    key,
                )
                .await
            }
            None => {
                self.post_json("/api/exchanges/credentials/clear", &request)
                    .await
            }
        }
    }

    pub async fn migrate_venue_credentials(
        &self,
        venue: &str,
    ) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
        self.migrate_venue_credentials_request(venue, None).await
    }

    pub async fn migrate_venue_credentials_with_idempotency_key(
        &self,
        venue: &str,
        idempotency_key: &str,
    ) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
        self.migrate_venue_credentials_with_context(
            venue,
            &MutationRequestContext::with_idempotency_key(idempotency_key),
        )
        .await
    }

    pub(crate) async fn migrate_venue_credentials_with_context(
        &self,
        venue: &str,
        context: &MutationRequestContext,
    ) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
        let request = VenueCredentialMigrateRequest {
            venue: venue.to_owned(),
        };
        self.post_json_with_context("/api/exchanges/credentials/migrate", &request, context)
            .await
    }

    async fn migrate_venue_credentials_request(
        &self,
        venue: &str,
        idempotency_key: Option<&str>,
    ) -> Result<VenueCredentialMaintenanceResponse, ApiError> {
        let request = VenueCredentialMigrateRequest {
            venue: venue.to_owned(),
        };
        match idempotency_key {
            Some(key) => {
                self.post_json_with_idempotency_key(
                    "/api/exchanges/credentials/migrate",
                    &request,
                    key,
                )
                .await
            }
            None => {
                self.post_json("/api/exchanges/credentials/migrate", &request)
                    .await
            }
        }
    }
}
