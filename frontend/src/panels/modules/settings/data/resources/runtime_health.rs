use super::{local_refresh_resource, SettingsResource};
use crate::state::context::use_global;
use leptos::prelude::*;
use shared_types::{VenueOperationHealthSnapshot, VenueRuntimeHealthSnapshot};

pub(in crate::panels::modules::settings) fn use_venue_operation_health(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<VenueOperationHealthSnapshot> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.venue_operation_health().await }
    })
}

pub(in crate::panels::modules::settings) fn use_venue_runtime_health(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<VenueRuntimeHealthSnapshot> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.venue_runtime_health().await }
    })
}
