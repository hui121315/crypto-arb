//! 顶部状态条聚合 DTO。

use serde::{Deserialize, Deserializer, Serialize};

pub type SystemHealthEnvelope = crate::ResourceEnvelope<SystemHealth>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemHealth {
    #[serde(default)]
    pub api_version: String,
    pub api: ApiHealthSlot,
    pub ws: WsHealthSlot,
    #[serde(deserialize_with = "deserialize_required_order_elapsed_ms")]
    pub order_elapsed_ms: Option<u32>,
    pub risk: RiskStatusSlot,
    pub net_delta_usd: f64,
    pub net_delta_pct_of_nav: f64,
    pub next_funding: Option<NextFundingSlot>,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default)]
    pub problems: Vec<RuntimeProblem>,
}

fn deserialize_required_order_elapsed_ms<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<u32>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeProblem {
    pub scope: String,
    pub operation: String,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub venue: Option<String>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<crate::ApiProblem>,
    pub observed_at_ms: i64,
}

impl RuntimeProblem {
    #[must_use]
    pub fn to_api_problem(&self) -> crate::ApiProblem {
        let mut problem = self
            .problem
            .clone()
            .unwrap_or_else(|| crate::ApiProblem::new(self.code.clone(), self.message.clone()));
        if problem.retry_after_ms.is_none() {
            problem.retry_after_ms = self.retry_after_ms;
        }
        if problem.source.is_none() {
            problem.source = Some(self.scope.clone());
        }
        let context = serde_json::json!({
            "operation": self.operation,
            "venue": self.venue,
            "observedAtMs": self.observed_at_ms,
        });
        match problem.details.as_mut() {
            Some(serde_json::Value::Object(details)) => {
                if let serde_json::Value::Object(context) = context {
                    for (key, value) in context {
                        details.entry(key).or_insert(value);
                    }
                }
            }
            Some(_) => {}
            None => problem.details = Some(context),
        }
        problem
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskHealthSummary {
    pub total: usize,
    pub healthy: usize,
    #[serde(default)]
    pub unhealthy: Vec<TaskHealthIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskHealthIssue {
    pub name: String,
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiHealthSlot {
    pub healthy: u32,
    pub total: u32,
    pub failed_venues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsHealthSlot {
    pub channels: u32,
    pub disconnected: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskStatusSlot {
    Ok,
    Warn,
    Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextFundingSlot {
    pub symbol: String,
    pub venue: String,
    pub minutes_to_settle: u32,
    pub estimated_outflow_usd: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_health_summary_serializes_shared_contract() {
        let summary = TaskHealthSummary {
            total: 2,
            healthy: 1,
            unhealthy: vec![TaskHealthIssue {
                name: "snapshot".to_owned(),
                code: "TASK_DOWN".to_owned(),
                detail: "panicked: boom".to_owned(),
            }],
        };

        let json = serde_json::to_value(&summary).expect("serialize task health");

        assert_eq!(json["total"], 2);
        assert_eq!(json["healthy"], 1);
        assert_eq!(json["unhealthy"][0]["name"], "snapshot");
        assert_eq!(json["unhealthy"][0]["code"], "TASK_DOWN");
        assert_eq!(json["unhealthy"][0]["detail"], "panicked: boom");
    }

    #[test]
    fn system_health_serializes_order_elapsed_contract() -> Result<(), serde_json::Error> {
        let health = system_health(Some(24));

        let json = serde_json::to_value(health)?;

        assert_eq!(json["apiVersion"], "1.2.3-test");
        assert_eq!(json["orderElapsedMs"], 24);
        assert!(json.get("avgOrderRttMs").is_none());
        Ok(())
    }

    #[test]
    fn system_health_serializes_missing_order_elapsed_sample_as_canonical_null(
    ) -> Result<(), serde_json::Error> {
        let health = system_health(None);

        let json = serde_json::to_value(health)?;

        assert!(json["orderElapsedMs"].is_null());
        assert!(json.get("avgOrderRttMs").is_none());
        Ok(())
    }

    #[test]
    fn system_health_rejects_legacy_avg_order_rtt_alias() {
        let payload = serde_json::json!({
            "api": { "healthy": 1, "total": 1, "failedVenues": [] },
            "ws": { "channels": 1, "disconnected": [] },
            "avgOrderRttMs": 42,
            "risk": "ok",
            "netDeltaUsd": 0.0,
            "netDeltaPctOfNav": 0.0,
            "nextFunding": null,
            "updatedAtMs": 1_000
        });

        let error =
            serde_json::from_value::<SystemHealth>(payload).expect_err("legacy alias is rejected");

        assert!(error.to_string().contains("orderElapsedMs"));
    }

    #[test]
    fn legacy_system_health_without_api_version_decodes_as_unverified(
    ) -> Result<(), serde_json::Error> {
        let payload = serde_json::json!({
            "api": { "healthy": 1, "total": 1, "failedVenues": [] },
            "ws": { "channels": 1, "disconnected": [] },
            "orderElapsedMs": null,
            "risk": "ok",
            "netDeltaUsd": 0.0,
            "netDeltaPctOfNav": 0.0,
            "nextFunding": null,
            "updatedAtMs": 1_000
        });

        let health = serde_json::from_value::<SystemHealth>(payload)?;

        assert!(health.api_version.is_empty());
        Ok(())
    }

    #[test]
    fn runtime_problem_preserves_typed_request_context() {
        let runtime = RuntimeProblem {
            scope: "market_data".to_owned(),
            operation: "funding_rates".to_owned(),
            code: "MARKET_DATA_RATE_LIMITED".to_owned(),
            message: "gate funding rate limited".to_owned(),
            venue: Some("gate".to_owned()),
            retry_after_ms: Some(2_000),
            problem: Some(
                crate::ApiProblem::new("UPSTREAM_RATE_LIMITED", "gate rate limited")
                    .with_status(429)
                    .with_request_id(Some("req-gate-funding".to_owned()))
                    .with_source("gate.GET /api/v4/futures/usdt/funding_rate"),
            ),
            observed_at_ms: 1_000,
        };

        let problem = runtime.to_api_problem();

        assert_eq!(problem.status, Some(429));
        assert_eq!(problem.request_id.as_deref(), Some("req-gate-funding"));
        assert_eq!(problem.retry_after_ms, Some(2_000));
        assert_eq!(
            problem
                .details
                .as_ref()
                .and_then(|details| details.get("venue"))
                .and_then(serde_json::Value::as_str),
            Some("gate")
        );
    }

    fn system_health(order_elapsed_ms: Option<u32>) -> SystemHealth {
        SystemHealth {
            api_version: "1.2.3-test".to_owned(),
            api: ApiHealthSlot {
                healthy: 1,
                total: 1,
                failed_venues: Vec::new(),
            },
            ws: WsHealthSlot {
                channels: 1,
                disconnected: Vec::new(),
            },
            order_elapsed_ms,
            risk: RiskStatusSlot::Ok,
            net_delta_usd: 0.0,
            net_delta_pct_of_nav: 0.0,
            next_funding: None,
            updated_at_ms: 1_000,
            degraded: false,
            problems: Vec::new(),
        }
    }
}
