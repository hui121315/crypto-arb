use serde::Serialize;
use shared_types::ActionRunKind;

use super::AuditCorrelation;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuditActorKind {
    BearerToken,
    System,
    Internal,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuditResourceKind {
    RiskConfiguration,
    TradingAdapter,
    KillSwitch,
    FeeSnapshot,
    Order,
    OrderSet,
    AutomationConfiguration,
    AutomationControl,
    AutomationLiveUnlock,
    HedgeTicket,
    WebhookConfiguration,
    MarketSubscriptionConfiguration,
    VenueCredentials,
    ProviderCredentials,
    CloseRun,
}

/// Stable, machine-queryable dimensions for an audit line.
///
/// Singular venue/symbol fields identify the primary subject. Multi-leg actions also retain the
/// complete ordered, deduplicated sets in `venues` and `symbols`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditEventContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<u16>,
    pub(crate) actor_kind: AuditActorKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) action_kind: Option<ActionRunKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) resource_kind: Option<AuditResourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ticket_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) client_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) venue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) problem_code: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) venues: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) symbols: Vec<String>,
}

impl AuditEventContext {
    pub(crate) fn for_actor(actor: &str) -> Self {
        Self {
            actor_kind: classify_actor(actor),
            ..Self::default()
        }
    }

    pub(crate) fn push_venue(&mut self, venue: &str) {
        push_subject(&mut self.venue, &mut self.venues, venue);
    }

    pub(crate) fn push_symbol(&mut self, symbol: &str) {
        push_subject(&mut self.symbol, &mut self.symbols, symbol);
    }
}

fn classify_actor(actor: &str) -> AuditActorKind {
    if actor.starts_with("api-token:") {
        AuditActorKind::BearerToken
    } else if actor == "system" || actor.starts_with("system:") {
        AuditActorKind::System
    } else if actor == "unknown" || actor.trim().is_empty() {
        AuditActorKind::Unknown
    } else {
        AuditActorKind::Internal
    }
}

fn push_subject(primary: &mut Option<String>, all: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() || all.iter().any(|existing| existing == value) {
        return;
    }
    if primary.is_none() {
        *primary = Some(value.to_owned());
    }
    all.push(value.to_owned());
}

#[derive(Debug, Serialize)]
pub(crate) struct AuditEvent<'a> {
    /// Event time (ISO 8601 UTC).
    pub ts: String,
    /// Verified caller identity or an explicit internal/system identity.
    pub actor: &'a str,
    /// Stable operation name such as `trading.order.submit`.
    pub action: &'a str,
    /// Human-readable primary resource identity retained for compatibility.
    pub resource: &'a str,
    /// Result: `accepted` / `success` / `denied` / `error`.
    pub outcome: &'a str,
    /// Machine-queryable route, actor, action and trading subject dimensions.
    #[serde(flatten)]
    pub context: AuditEventContext,
    /// Machine-queryable request/action/order/run correlation fields.
    #[serde(flatten)]
    pub correlation: AuditCorrelation,
    /// Free-form safe details. Secret plaintext is forbidden.
    pub detail: serde_json::Value,
}

impl<'a> AuditEvent<'a> {
    pub(crate) fn now(
        actor: &'a str,
        action: &'a str,
        resource: &'a str,
        outcome: &'a str,
        detail: serde_json::Value,
    ) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            actor,
            action,
            resource,
            outcome,
            context: AuditEventContext::for_actor(actor),
            correlation: AuditCorrelation::request(common::request_id::current()),
            detail,
        }
    }

    pub(crate) fn with_context(mut self, context: AuditEventContext) -> Self {
        self.context = context;
        self
    }

    pub(crate) fn with_correlation(mut self, mut correlation: AuditCorrelation) -> Self {
        if correlation.request_id.is_none() {
            correlation.request_id = self.correlation.request_id.take();
        }
        self.correlation = correlation;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_classifies_actor_and_deduplicates_multi_leg_subjects() {
        let mut context = AuditEventContext::for_actor("api-token:operator:0123456789abcdef");
        context.push_venue("okx");
        context.push_venue("okx");
        context.push_venue("bybit");
        context.push_symbol("BTC-USDT-SWAP");

        assert_eq!(context.actor_kind, AuditActorKind::BearerToken);
        assert_eq!(context.venue.as_deref(), Some("okx"));
        assert_eq!(context.venues, ["okx", "bybit"]);
        assert_eq!(context.symbol.as_deref(), Some("BTC-USDT-SWAP"));
    }
}
