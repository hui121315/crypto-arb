use crate::services::venue_credentials::{self, CredentialUpdateError};
use shared_types::{WebhookConfig, WebhookConfigPatch, WebhookEventKind, WebhookProvider};
use webhook::{WebhookDispatcher, WebhookError};

const ENABLED_KEY: &str = "APP_WEBHOOK__ENABLED";
const PROVIDER_KEY: &str = "APP_WEBHOOK__PROVIDER";
const URL_KEY: &str = "APP_WEBHOOK__URL";
const SECRET_KEY: &str = "APP_WEBHOOK__SECRET";
const EVENT_KINDS_KEY: &str = "APP_WEBHOOK__EVENT_KINDS";
const TIMEOUT_MS_KEY: &str = "APP_WEBHOOK__TIMEOUT_MS";
const MAX_ATTEMPTS_KEY: &str = "APP_WEBHOOK__MAX_ATTEMPTS";
const BASE_BACKOFF_MS_KEY: &str = "APP_WEBHOOK__BASE_BACKOFF_MS";
const QUEUE_CAPACITY_KEY: &str = "APP_WEBHOOK__QUEUE_CAPACITY";

pub(super) fn restore(dispatcher: &WebhookDispatcher) -> Result<(), WebhookError> {
    let mut values = std::collections::HashMap::new();
    for key in [ENABLED_KEY, PROVIDER_KEY, URL_KEY, SECRET_KEY, EVENT_KINDS_KEY,
        TIMEOUT_MS_KEY, MAX_ATTEMPTS_KEY, BASE_BACKOFF_MS_KEY, QUEUE_CAPACITY_KEY] {
        if let Some(value) = venue_credentials::checked_secret(key)
            .map_err(|_| WebhookError("Webhook 凭证存储读取失败，请检查存储后重启".into()))? {
            values.insert(key, value);
        }
    }
    restore_with(dispatcher, |key| values.get(key).cloned())
}

fn restore_with(
    dispatcher: &WebhookDispatcher,
    read: impl Fn(&str) -> Option<String>,
) -> Result<(), WebhookError> {
    let Some((mut patch, enabled)) = stored_patch(read)? else {
        return Ok(());
    };
    // Restore the target and limits first. An invalid enabled configuration remains
    // visibly configured but disabled instead of discarding every persisted field.
    patch.enabled = Some(false);
    dispatcher.update_config(patch)?;
    if enabled {
        dispatcher.update_config(WebhookConfigPatch {
            enabled: Some(true),
            ..WebhookConfigPatch::default()
        })?;
    }
    Ok(())
}

pub(super) async fn persist(
    patch: &WebhookConfigPatch,
    effective: &WebhookConfig,
) -> Result<(), CredentialUpdateError> {
    #[cfg(test)]
    if std::env::var_os("CROSSLINE_SETTINGS_BROWSER_DIR").is_none() {
        return Ok(());
    }
    let mut updates = public_updates(effective);
    let mut clears = Vec::new();
    if let Some(url) = patch.url.as_deref() {
        if url.trim().is_empty() {
            clears.push(URL_KEY.to_owned());
        } else {
            updates.push((URL_KEY.to_owned(), url.trim().to_owned()));
        }
    }
    let replacement_secret = patch
        .secret
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    if let Some(secret) = replacement_secret {
        updates.push((SECRET_KEY.to_owned(), secret.to_owned()));
    } else if patch.clear_secret.unwrap_or(false) {
        clears.push(SECRET_KEY.to_owned());
    }
    venue_credentials::persist_secret_changes(&updates, &clears).await
}

fn public_updates(config: &WebhookConfig) -> Vec<(String, String)> {
    vec![
        (ENABLED_KEY.to_owned(), config.enabled.to_string()),
        (
            PROVIDER_KEY.to_owned(),
            provider_key(config.provider).to_owned(),
        ),
        (
            EVENT_KINDS_KEY.to_owned(),
            event_kinds_value(&config.event_kinds),
        ),
        (TIMEOUT_MS_KEY.to_owned(), config.timeout_ms.to_string()),
        (MAX_ATTEMPTS_KEY.to_owned(), config.max_attempts.to_string()),
        (
            BASE_BACKOFF_MS_KEY.to_owned(),
            config.base_backoff_ms.to_string(),
        ),
        (
            QUEUE_CAPACITY_KEY.to_owned(),
            config.queue_capacity.to_string(),
        ),
    ]
}

fn stored_patch(read: impl Fn(&str) -> Option<String>) -> Result<Option<(WebhookConfigPatch, bool)>, WebhookError> {
    let enabled = read_parsed(&read, ENABLED_KEY, parse_bool)?;
    let provider = read_parsed(&read, PROVIDER_KEY, parse_provider)?;
    let url = read(URL_KEY).filter(|value| !value.trim().is_empty());
    let secret = read(SECRET_KEY).filter(|value| !value.trim().is_empty());
    let event_kinds = read_parsed(&read, EVENT_KINDS_KEY, parse_event_kinds)?;
    let timeout_ms = read_parsed(&read, TIMEOUT_MS_KEY, |value| value.trim().parse().ok())?;
    let max_attempts = read_parsed(&read, MAX_ATTEMPTS_KEY, |value| value.trim().parse().ok())?;
    let base_backoff_ms = read_parsed(&read, BASE_BACKOFF_MS_KEY, |value| value.trim().parse().ok())?;
    let queue_capacity = read_parsed(&read, QUEUE_CAPACITY_KEY, |value| value.trim().parse().ok())?;
    let configured = enabled.is_some()
        || provider.is_some()
        || url.is_some()
        || secret.is_some()
        || event_kinds.is_some()
        || timeout_ms.is_some()
        || max_attempts.is_some()
        || base_backoff_ms.is_some()
        || queue_capacity.is_some();
    Ok(configured.then(|| {
        (
            WebhookConfigPatch {
                enabled,
                provider,
                url,
                secret,
                event_kinds,
                timeout_ms,
                max_attempts,
                base_backoff_ms,
                queue_capacity,
                ..WebhookConfigPatch::default()
            },
            enabled.unwrap_or(false),
        )
    }))
}

fn read_parsed<T>(read: &impl Fn(&str) -> Option<String>, key: &str,
    parse: impl Fn(&str) -> Option<T>) -> Result<Option<T>, WebhookError> {
    read(key).map(|value| parse(&value).ok_or_else(|| WebhookError(format!(
        "Webhook 配置项 {key} 无效，请修复配置后重启"
    )))).transpose()
}

const fn provider_key(provider: WebhookProvider) -> &'static str {
    match provider {
        WebhookProvider::Generic => "generic",
        WebhookProvider::Bark => "bark",
    }
}

fn parse_provider(value: &str) -> Option<WebhookProvider> {
    match value.trim().to_ascii_lowercase().as_str() {
        "generic" => Some(WebhookProvider::Generic),
        "bark" => Some(WebhookProvider::Bark),
        _ => None,
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn event_kinds_value(kinds: &[WebhookEventKind]) -> String {
    if kinds.is_empty() {
        return "none".to_owned();
    }
    kinds
        .iter()
        .map(|kind| event_kind_key(*kind))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_event_kinds(value: &str) -> Option<Vec<WebhookEventKind>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let kinds = value
        .split(',')
        .map(|kind| parse_event_kind(kind.trim()))
        .collect::<Option<Vec<_>>>()?;
    (!kinds.is_empty()).then_some(kinds)
}

const fn event_kind_key(kind: WebhookEventKind) -> &'static str {
    match kind {
        WebhookEventKind::Opportunity => "opportunity",
        WebhookEventKind::OpportunityMonitor => "opportunity_monitor",
        WebhookEventKind::AutomationDecision => "automation_decision",
        WebhookEventKind::ExecutionResult => "execution_result",
        WebhookEventKind::Compensation => "compensation",
        WebhookEventKind::RiskAlert => "risk_alert",
        WebhookEventKind::SystemDegradation => "system_degradation",
        WebhookEventKind::OnchainSpread => "onchain_spread",
        WebhookEventKind::StockSpread => "stock_spread",
        WebhookEventKind::Test => "test",
    }
}

fn parse_event_kind(value: &str) -> Option<WebhookEventKind> {
    match value {
        "opportunity" => Some(WebhookEventKind::Opportunity),
        "opportunity_monitor" => Some(WebhookEventKind::OpportunityMonitor),
        "automation_decision" => Some(WebhookEventKind::AutomationDecision),
        "execution_result" => Some(WebhookEventKind::ExecutionResult),
        "compensation" => Some(WebhookEventKind::Compensation),
        "risk_alert" => Some(WebhookEventKind::RiskAlert),
        "system_degradation" => Some(WebhookEventKind::SystemDegradation),
        "onchain_spread" => Some(WebhookEventKind::OnchainSpread),
        "stock_spread" => Some(WebhookEventKind::StockSpread),
        "test" => Some(WebhookEventKind::Test),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn stock_alert_subscription_round_trips_without_enabling_old_subscriptions() {
        assert_eq!(parse_event_kind(event_kind_key(WebhookEventKind::StockSpread)),Some(WebhookEventKind::StockSpread));
        assert!(!parse_event_kinds("onchain_spread").unwrap().contains(&WebhookEventKind::StockSpread));
    }

    #[tokio::test]
    async fn restored_bark_config_keeps_target_and_enabled_state() -> anyhow::Result<()> {
        let values = HashMap::from([
            (ENABLED_KEY, "true"),
            (PROVIDER_KEY, "bark"),
            (URL_KEY, "https://api.day.app/device-key/group"),
            (EVENT_KINDS_KEY, "automation_decision,execution_result"),
            (TIMEOUT_MS_KEY, "2500"),
            (MAX_ATTEMPTS_KEY, "4"),
            (BASE_BACKOFF_MS_KEY, "750"),
            (QUEUE_CAPACITY_KEY, "64"),
        ]);
        let dispatcher = WebhookDispatcher::default();

        restore_with(&dispatcher, |key| values.get(key).map(ToString::to_string))?;
        let status = dispatcher.status(1).await;

        assert!(status.config.enabled);
        assert!(status.config.url_configured);
        assert_eq!(status.config.provider, WebhookProvider::Bark);
        assert_eq!(status.config.timeout_ms, 2_500);
        assert_eq!(status.config.max_attempts, 4);
        assert_eq!(status.config.queue_capacity, 64);
        assert_eq!(
            status.config.event_kinds,
            [
                WebhookEventKind::AutomationDecision,
                WebhookEventKind::ExecutionResult
            ]
        );
        Ok(())
    }

    #[test]
    fn empty_event_scope_round_trips_explicitly() {
        assert_eq!(event_kinds_value(&[]), "none");
        assert_eq!(parse_event_kinds("none"), Some(Vec::new()));
    }

    #[test]
    fn opportunity_monitor_scope_round_trips_without_expanding_deterministic_scope() {
        let kinds = [
            WebhookEventKind::Opportunity,
            WebhookEventKind::OpportunityMonitor,
        ];

        assert_eq!(event_kinds_value(&kinds), "opportunity,opportunity_monitor");
        assert_eq!(
            parse_event_kinds("opportunity,opportunity_monitor"),
            Some(kinds.to_vec())
        );
        assert_eq!(
            parse_event_kinds("opportunity"),
            Some(vec![WebhookEventKind::Opportunity])
        );
    }
}
