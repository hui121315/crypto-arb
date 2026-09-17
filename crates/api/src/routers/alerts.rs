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
use shared_types::{AlertRule, AlertRulesEnvelope};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/alerts/rules", get(list).post(create))
        .route("/api/alerts/rules/:id", delete(remove))
}

async fn list(State(state): State<AppState>) -> Json<AlertRulesEnvelope> {
    Json(watchlist_alerts::alert_rules_envelope(
        &state,
        state.alert_rules().read().await.clone(),
    ))
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(rule): Json<AlertRule>,
) -> Result<Json<AlertRulesEnvelope>, AppError> {
    rule.validate().map_err(AppError::BadRequest)?;
    let actor = audit::extract_actor(&headers);
    let response = watchlist_alerts::create_alert_rule(&state, rule, &actor).await?;
    if response.changed {
        if let Some(rule) = response.rule.as_ref() {
            audit_alert_rule_create(&headers, rule);
        }
        publish_alert_rules_changed(&state, &response.envelope)?;
    }
    Ok(Json(response.envelope))
}

async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<AlertRulesEnvelope>, AppError> {
    let response = watchlist_alerts::remove_alert_rule_by_id(&state, id).await?;
    if response.changed {
        state.alert_cooldowns().remove(&id);
        audit_alert_rule_remove(&headers, id);
        publish_alert_rules_changed(&state, &response.envelope)?;
    }
    Ok(Json(response.envelope))
}

fn audit_alert_rule_create(headers: &HeaderMap, rule: &AlertRule) {
    let resource = format!("alert-rule:{}", rule.id);
    audit::record_http_event(
        headers,
        "alert_rule.create",
        resource.as_str(),
        "success",
        json!({
            "id": rule.id,
            "watchlistId": rule.watchlist_id,
            "enabled": rule.enabled,
            "cooldownSecs": rule.cooldown_secs,
            "channel": rule.channel.delivery_kind(),
            "featureGate": WATCHLIST_ALERTS_FEATURE,
            "authPolicy": "global_bearer",
        }),
    );
}

fn audit_alert_rule_remove(headers: &HeaderMap, id: i64) {
    let resource = format!("alert-rule:{id}");
    audit::record_http_event(
        headers,
        "alert_rule.delete",
        resource.as_str(),
        "success",
        json!({ "id": id }),
    );
}

fn publish_alert_rules_changed(
    state: &AppState,
    envelope: &AlertRulesEnvelope,
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
#[path = "alerts/tests.rs"]
mod tests;
