use super::ActionRunKind;
use crate::{ApiProblem, OrderRecord, VenueOrderIdentity};
use serde::{Deserialize, Serialize};

#[path = "evidence/conversions.rs"]
mod conversions;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionEvidenceSource {
    #[default]
    ClientRequest,
    ActionRun,
    CloseRun,
    ExecutionRun,
    OrderRecord,
    ApiProblem,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_kind: Option<ActionRunKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client_order_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exchange_order_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub venues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ActionEvidenceSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<i64>,
}

impl ActionEvidence {
    #[must_use]
    pub fn client_request(request_id: impl Into<String>, idempotency_key: Option<String>) -> Self {
        let mut evidence = Self::default().with_request_id(Some(request_id.into()));
        evidence.idempotency_key = clean(idempotency_key);
        evidence.push_source(ActionEvidenceSource::ClientRequest);
        evidence
    }

    #[must_use]
    pub fn with_action_kind(mut self, value: ActionRunKind) -> Self {
        self.action_kind = Some(value);
        self
    }

    #[must_use]
    pub fn from_problem(problem: &ApiProblem) -> Self {
        let mut evidence = Self::default().with_request_id(problem.request_id.clone());
        if evidence.request_id.is_some() {
            evidence.push_source(ActionEvidenceSource::ApiProblem);
        }
        evidence
    }

    #[must_use]
    pub fn with_request_id(mut self, value: Option<String>) -> Self {
        self.request_id = clean(value).or(self.request_id);
        self
    }

    #[must_use]
    pub fn with_action_run_id(mut self, value: Option<String>) -> Self {
        self.action_run_id = clean(value).or(self.action_run_id);
        self
    }

    #[must_use]
    pub fn with_idempotency_key(mut self, value: Option<String>) -> Self {
        self.idempotency_key = clean(value).or(self.idempotency_key);
        self
    }

    #[must_use]
    pub fn with_run_id(mut self, value: Option<String>) -> Self {
        self.run_id = clean(value).or(self.run_id);
        self
    }

    #[must_use]
    pub fn with_ticket_id(mut self, value: Option<String>) -> Self {
        self.ticket_id = clean(value).or(self.ticket_id);
        self
    }

    #[must_use]
    pub fn with_client_order_ids(mut self, values: impl IntoIterator<Item = String>) -> Self {
        extend_unique(&mut self.client_order_ids, values);
        self
    }

    #[must_use]
    pub fn with_venues(mut self, values: impl IntoIterator<Item = String>) -> Self {
        extend_clean(&mut self.venues, values);
        self
    }

    #[must_use]
    pub fn with_symbols(mut self, values: impl IntoIterator<Item = String>) -> Self {
        extend_clean(&mut self.symbols, values);
        self
    }

    #[must_use]
    pub fn merged(mut self, other: Self) -> Self {
        self.merge(other);
        self
    }

    pub fn merge(&mut self, other: Self) {
        if self.action_kind.is_none() {
            self.action_kind = other.action_kind;
        }
        if self.request_id.is_none() {
            self.request_id = other.request_id;
        }
        if self.action_run_id.is_none() {
            self.action_run_id = other.action_run_id;
        }
        if self.idempotency_key.is_none() {
            self.idempotency_key = other.idempotency_key;
        }
        if self.run_id.is_none() {
            self.run_id = other.run_id;
        }
        if self.ticket_id.is_none() {
            self.ticket_id = other.ticket_id;
        }
        extend_unique(&mut self.order_ids, other.order_ids);
        extend_unique(&mut self.client_order_ids, other.client_order_ids);
        extend_unique(&mut self.exchange_order_ids, other.exchange_order_ids);
        extend_unique(&mut self.venues, other.venues);
        extend_unique(&mut self.symbols, other.symbols);
        extend_unique(&mut self.sources, other.sources);
        self.observed_at_ms = self.observed_at_ms.max(other.observed_at_ms);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.action_kind.is_none()
            && self.request_id.is_none()
            && self.action_run_id.is_none()
            && self.idempotency_key.is_none()
            && self.run_id.is_none()
            && self.ticket_id.is_none()
            && self.order_ids.is_empty()
            && self.client_order_ids.is_empty()
            && self.exchange_order_ids.is_empty()
            && self.venues.is_empty()
            && self.symbols.is_empty()
    }

    #[must_use]
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(kind) = self.action_kind {
            parts.push(format!("action_kind {}", kind.as_str()));
        }
        push_summary(&mut parts, "request_id", self.request_id.as_deref());
        push_summary(&mut parts, "action_run_id", self.action_run_id.as_deref());
        push_summary(&mut parts, "idempotency", self.idempotency_key.as_deref());
        push_summary(&mut parts, "run_id", self.run_id.as_deref());
        push_summary(&mut parts, "ticket_id", self.ticket_id.as_deref());
        push_list_summary(&mut parts, "order_id", &self.order_ids);
        push_list_summary(&mut parts, "client_order_id", &self.client_order_ids);
        push_list_summary(&mut parts, "exchange_order_id", &self.exchange_order_ids);
        push_list_summary(&mut parts, "venue", &self.venues);
        push_list_summary(&mut parts, "symbol", &self.symbols);
        parts.join(" · ")
    }

    fn merge_order(&mut self, order: &OrderRecord) {
        push_unique(&mut self.venues, &order.intent.exchange);
        push_unique(&mut self.symbols, &order.intent.symbol);
        push_unique(&mut self.order_ids, &order.intent.id);
        push_unique(&mut self.client_order_ids, &order.intent.client_order_id);
        self.merge_identity(&order.identity_snapshot());
    }

    fn merge_identity(&mut self, identity: &VenueOrderIdentity) {
        push_unique(&mut self.order_ids, &identity.internal_order_id);
        push_unique(&mut self.client_order_ids, &identity.public_client_order_id);
        push_optional(
            &mut self.client_order_ids,
            identity.venue_client_order_id.as_deref(),
        );
        push_optional(
            &mut self.exchange_order_ids,
            identity.exchange_order_id.as_deref(),
        );
    }

    fn merge_client_order_policy(&mut self, policy: &crate::ClientOrderIdPolicy) {
        push_unique(&mut self.client_order_ids, &policy.public_client_order_id);
        push_optional(
            &mut self.client_order_ids,
            policy.venue_client_order_id.as_deref(),
        );
    }

    fn push_source(&mut self, source: ActionEvidenceSource) {
        if !self.sources.contains(&source) {
            self.sources.push(source);
        }
    }
}

fn clean(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}

fn push_optional(values: &mut Vec<String>, value: Option<&str>) {
    if let Some(value) = value {
        push_unique(values, value);
    }
}

fn extend_unique<T>(values: &mut Vec<T>, incoming: impl IntoIterator<Item = T>)
where
    T: PartialEq,
{
    for value in incoming {
        if !values.contains(&value) {
            values.push(value);
        }
    }
}

fn extend_clean(values: &mut Vec<String>, incoming: impl IntoIterator<Item = String>) {
    for value in incoming {
        push_unique(values, &value);
    }
}

fn push_summary(parts: &mut Vec<String>, key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        parts.push(format!("{key} {value}"));
    }
}

fn push_list_summary(parts: &mut Vec<String>, key: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let shown = values.iter().take(2).cloned().collect::<Vec<_>>().join(",");
    if values.len() > 2 {
        parts.push(format!("{key} {shown} (+{})", values.len() - 2));
    } else {
        parts.push(format!("{key} {shown}"));
    }
}

#[cfg(test)]
#[path = "evidence/tests.rs"]
mod tests;
