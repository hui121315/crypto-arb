use super::ActionEvidence;
use crate::ApiProblem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ActionState {
    #[default]
    Idle,
    Pending {
        label: String,
        #[serde(default, skip_serializing_if = "ActionEvidence::is_empty")]
        evidence: ActionEvidence,
    },
    Accepted {
        label: String,
        #[serde(default, skip_serializing_if = "ActionEvidence::is_empty")]
        evidence: ActionEvidence,
    },
    Succeeded {
        label: String,
        #[serde(default, skip_serializing_if = "ActionEvidence::is_empty")]
        evidence: ActionEvidence,
    },
    Failed {
        label: String,
        problem: ApiProblem,
        #[serde(default, skip_serializing_if = "ActionEvidence::is_empty")]
        evidence: ActionEvidence,
    },
}

impl ActionState {
    pub fn pending(label: impl Into<String>) -> Self {
        Self::Pending {
            label: label.into(),
            evidence: ActionEvidence::default(),
        }
    }

    pub fn accepted(label: impl Into<String>) -> Self {
        Self::Accepted {
            label: label.into(),
            evidence: ActionEvidence::default(),
        }
    }

    pub fn succeeded(label: impl Into<String>) -> Self {
        Self::Succeeded {
            label: label.into(),
            evidence: ActionEvidence::default(),
        }
    }

    pub fn failed(label: impl Into<String>, problem: ApiProblem) -> Self {
        let evidence = ActionEvidence::from_problem(&problem);
        Self::Failed {
            label: label.into(),
            problem,
            evidence,
        }
    }

    #[must_use]
    pub fn with_evidence(mut self, evidence: ActionEvidence) -> Self {
        if let Some(current) = self.evidence_mut() {
            *current = evidence.merged(current.clone());
        }
        self
    }

    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. })
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Pending { label, .. }
            | Self::Accepted { label, .. }
            | Self::Succeeded { label, .. }
            | Self::Failed { label, .. } => Some(label),
            Self::Idle => None,
        }
    }

    #[must_use]
    pub const fn problem(&self) -> Option<&ApiProblem> {
        match self {
            Self::Failed { problem, .. } => Some(problem),
            Self::Idle | Self::Pending { .. } | Self::Accepted { .. } | Self::Succeeded { .. } => {
                None
            }
        }
    }

    #[must_use]
    pub const fn evidence(&self) -> Option<&ActionEvidence> {
        match self {
            Self::Idle => None,
            Self::Pending { evidence, .. }
            | Self::Accepted { evidence, .. }
            | Self::Succeeded { evidence, .. }
            | Self::Failed { evidence, .. } => Some(evidence),
        }
    }

    #[must_use]
    pub fn message(&self, default: &str) -> String {
        let label = match self {
            Self::Idle => default.to_owned(),
            Self::Pending { label, .. }
            | Self::Accepted { label, .. }
            | Self::Succeeded { label, .. } => label.clone(),
            Self::Failed { label, problem, .. } => problem_message(label, problem),
        };
        let evidence = self
            .evidence()
            .map(ActionEvidence::summary)
            .unwrap_or_default();
        if evidence.is_empty() {
            label
        } else {
            format!("{label} · {evidence}")
        }
    }

    fn evidence_mut(&mut self) -> Option<&mut ActionEvidence> {
        match self {
            Self::Idle => None,
            Self::Pending { evidence, .. }
            | Self::Accepted { evidence, .. }
            | Self::Succeeded { evidence, .. }
            | Self::Failed { evidence, .. } => Some(evidence),
        }
    }
}

fn problem_message(prefix: &str, problem: &ApiProblem) -> String {
    let mut parts = vec![format!("{prefix}：{}", problem.message)];
    if !problem.code.trim().is_empty() {
        parts.push(format!("code {}", problem.code));
    }
    if let Some(source) = problem.source.as_deref() {
        parts.push(format!("source {source}"));
    }
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem.request_id.as_deref() {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_state_blocks_duplicate_submit() {
        let state = ActionState::pending("保存中");

        assert!(state.is_pending());
        assert_eq!(state.label(), Some("保存中"));
        assert!(state.problem().is_none());
    }

    #[test]
    fn failed_state_keeps_typed_problem_context() {
        let problem = ApiProblem::new("RATE_LIMITED", "slow")
            .with_source("trade-rest")
            .with_status(429)
            .with_request_id(Some("req-1".into()))
            .with_retry_after_ms(Some(2_000));
        let state = ActionState::failed("提交失败", problem);
        let message = state.message("idle");

        assert!(message.contains("code RATE_LIMITED"));
        assert!(message.contains("source trade-rest"));
        assert!(message.contains("HTTP 429"));
        assert!(message.contains("request_id req-1"));
        assert!(message.contains("retry 2000ms"));
    }

    #[test]
    fn legacy_state_without_evidence_still_decodes() {
        let state: ActionState = serde_json::from_str(r#"{"state":"pending","label":"保存中"}"#)
            .expect("legacy action state");

        assert!(state.is_pending());
        assert!(state.evidence().is_some_and(ActionEvidence::is_empty));
    }

    #[test]
    fn message_renders_structured_pending_evidence() {
        let state = ActionState::pending("提交中").with_evidence(
            ActionEvidence::client_request("req-pending", Some("idem-pending".into()))
                .with_client_order_ids(["client-order-1".into()]),
        );
        let message = state.message("idle");

        assert!(message.contains("request_id req-pending"));
        assert!(message.contains("idempotency idem-pending"));
        assert!(message.contains("client_order_id client-order-1"));
    }
}
