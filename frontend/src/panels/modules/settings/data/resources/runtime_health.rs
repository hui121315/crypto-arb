use super::{local_refresh_resource, SettingsResource};
use leptos::prelude::*;
use shared_types::{VenueOperationHealthSnapshot, VenueRuntimeHealthSnapshot};

pub(in crate::panels::modules::settings) fn use_venue_operation_health(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<VenueOperationHealthSnapshot> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.venue_operation_health().await }
    })
}

pub(in crate::panels::modules::settings) fn use_venue_runtime_health(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<VenueRuntimeHealthSnapshot> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.venue_runtime_health().await }
    })
}
