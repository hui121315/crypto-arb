use serde_json::json;

pub(super) fn trading_status_result_summary(
    result: &serde_json::Value,
) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "adapter": result.get("adapter"),
        "environment": result.get("environment"),
        "openOrderCount": result.get("openOrderCount"),
        "actionRunId": result.get("actionRunId"),
        "requestId": result.get("requestId"),
        "idempotencyKey": result.get("idempotencyKey"),
        "mutation": result.get("mutation"),
        "risk": {
            "liveTradingEnabled": result.pointer("/risk/liveTradingEnabled"),
            "killSwitchActive": result.pointer("/risk/killSwitchActive"),
            "maxOrderNotional": result.pointer("/risk/maxOrderNotional"),
            "maxOpenOrders": result.pointer("/risk/maxOpenOrders"),
            "allowedExchanges": result.pointer("/risk/allowedExchanges"),
            "allowedSymbols": result.pointer("/risk/allowedSymbols"),
        }
    }))
}

pub(super) fn automation_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "state": result.get("state"),
        "enabled": result.pointer("/config/enabled"),
        "paused": result.pointer("/config/paused"),
        "environment": result.pointer("/config/environment"),
        "liveUnlocked": result.get("liveUnlocked"),
        "updatedAtMs": result.get("updatedAtMs"),
    }))
}

pub(super) fn webhook_result_summary(result: &serde_json::Value) -> Option<serde_json::Value> {
    if !result.is_object() {
        return None;
    }
    Some(json!({
        "enabled": result.pointer("/config/enabled"),
        "targetConfigured": result.pointer("/config/targetConfigured"),
        "secretConfigured": result.pointer("/config/secretConfigured"),
        "eventKinds": result.pointer("/config/eventKinds"),
        "queueDepth": result.get("queueDepth"),
        "updatedAtMs": result.get("updatedAtMs"),
    }))
}
