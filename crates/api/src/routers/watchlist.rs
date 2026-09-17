use crate::middleware::audit;
use crate::services::watchlist_alerts;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get};
use axum::{Json, Router};
use common::AppError;
use realtime::alerts::WATCHLIST_ALERTS_FEATURE;
use serde_json::json;
use shared_types::{WatchlistEnvelope, WatchlistItem};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/watchlist", get(list).post(create))
        .route("/api/watchlist/:id", delete(remove))
}

async fn list(State(state): State<AppState>) -> Json<WatchlistEnvelope> {
    Json(watchlist_alerts::watchlist_envelope(
        &state,
        state.watchlist().read().await.clone(),
    ))
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(item): Json<WatchlistItem>,
) -> Result<Json<WatchlistEnvelope>, AppError> {
    item.validate().map_err(AppError::BadRequest)?;
    let actor = audit::extract_actor(&headers);
    let response = watchlist_alerts::create_watchlist_item(&state, item, &actor).await?;
    if response.changed {
        if let Some(item) = response.item.as_ref() {
            audit_watchlist_create(&headers, item);
        }
        publish_watchlist_changed(&state, &response.envelope)?;
    }
    Ok(Json(response.envelope))
}

async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<WatchlistEnvelope>, AppError> {
    let response = watchlist_alerts::remove_watchlist(&state, id).await?;
    for rule_id in &response.removed_alert_rule_ids {
        state.alert_cooldowns().remove(rule_id);
    }
    if response.changed {
        audit_watchlist_remove(&headers, id, response.removed_alert_rule_ids.len());
        publish_watchlist_changed(&state, &response.envelope)?;
        if let Some(alerts) = response.alert_rules_envelope.as_ref() {
            publish_alert_rules_changed(&state, alerts)?;
        }
    }
    Ok(Json(response.envelope))
}

fn audit_watchlist_create(headers: &HeaderMap, item: &WatchlistItem) {
    let resource = format!("watchlist:{}", item.id);
    audit::record_http_event(
        headers,
        "watchlist.create",
        resource.as_str(),
        "success",
        json!({
            "id": item.id,
            "symbol": item.symbol,
            "enabled": item.enabled,
            "venueLong": item.venue_long,
            "venueShort": item.venue_short,
            "featureGate": WATCHLIST_ALERTS_FEATURE,
            "authPolicy": "global_bearer",
        }),
    );
}

fn audit_watchlist_remove(headers: &HeaderMap, id: i64, removed_alert_rule_count: usize) {
    let resource = format!("watchlist:{id}");
    audit::record_http_event(
        headers,
        "watchlist.delete",
        resource.as_str(),
        "success",
        json!({
            "id": id,
            "removedAlertRuleCount": removed_alert_rule_count,
        }),
    );
}

fn publish_watchlist_changed(
    state: &AppState,
    envelope: &WatchlistEnvelope,
) -> Result<(), AppError> {
    let event = shared_types::WatchlistStreamEvent::WatchlistChanged {
        envelope: envelope.clone(),
        timestamp_ms: common::time::now_ms(),
    };
    state.ws_hub().publish(
        realtime::channels::WATCHLIST,
        realtime::WsMessage::json(&event)?,
    );
    Ok(())
}

fn publish_alert_rules_changed(
    state: &AppState,
    envelope: &shared_types::AlertRulesEnvelope,
) -> Result<(), AppError> {
    let event = shared_types::AlertStreamEvent::AlertRulesChanged {
        envelope: Box::new(envelope.clone()),
        timestamp_ms: common::time::now_ms(),
    };
    state.ws_hub().publish(
        realtime::channels::ALERTS,
        realtime::WsMessage::json(&event)?,
    );
    Ok(())
}

#[cfg(test)]
#[path = "watchlist/tests.rs"]
mod tests;
