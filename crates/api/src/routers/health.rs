//! `GET /health`：最小进程存活探针。
//!
//! 进程能响应即 HTTP 200（liveness）；`status`/`degraded` 字段反映后台任务是否
//! 出现死亡/卡住/连续失败，供监控与 CI smoke 判断 runtime 是否降级。
//!
//! `GET /health/ready`：readiness 探针。后台任务全部健康返回 200，存在死亡/卡住/
//! 连续失败任务时返回 503，供 K8s readiness probe / 负载均衡摘除降级实例；`/health`
//! 始终 200（liveness），二者职责分离。

use crate::services::venue_operation_health;
use crate::state::AppState;
use crate::task_registry::TaskRegistry;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use shared_types::{
    TaskHealthIssue, TaskHealthSummary, VenueOperationHealth, VenueOperationHealthSnapshot,
    VenueOperationStatus,
};

#[derive(Serialize)]
struct HealthResponse {
    /// 进程恒可响应；`ok` 表示后台任务全部健康，`degraded` 表示存在死亡/卡住/
    /// 连续失败的后台任务（详见 `tasks.unhealthy`）。
    status: &'static str,
    version: &'static str,
    timestamp_ms: i64,
    /// 是否存在不健康后台任务。监控 / CI smoke 可据此发现 runtime 降级。
    degraded: bool,
    /// 后台任务健康摘要。
    tasks: TaskHealthSummary,
}

/// readiness 探针响应：仅承载后台任务健康摘要，不含 metrics/ws 明细，
/// 便于 K8s readiness probe / 负载均衡快速判定是否摘除降级实例。
#[derive(Serialize)]
struct ReadinessResponse {
    /// `ok`（HTTP 200）表示后台任务和 readiness 关键运行态全部健康；
    /// `degraded` 可能仍返回 200（Warn）或 503（Blocked）。
    status: &'static str,
    timestamp_ms: i64,
    degraded: bool,
    tasks: TaskHealthSummary,
    operations: ReadinessOperationSummary,
}

#[derive(Serialize)]
struct ReadinessOperationSummary {
    blocked: usize,
    warn: usize,
    attention: Vec<ReadinessOperation>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadinessOperation {
    venue: String,
    operation: String,
    status: VenueOperationStatus,
    source: String,
    message: String,
    problem_code: Option<String>,
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/health/ready", get(ready))
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(build_liveness(
        state.task_registry(),
        common::time::now_ms(),
    ))
}

/// readiness 探针：后台任务降级时返回 503，否则 200。与 `/health`（恒 200 liveness）
/// 分离职责，供编排器在 runtime 降级时摘除该实例。
async fn ready(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let operation_health = venue_operation_health::snapshot(&state);
    let (status, body) = build_readiness(
        state.task_registry(),
        &operation_health,
        common::time::now_ms(),
    );
    (status, Json(body))
}

/// 纯函数：由任务登记表和系统级运行态健康推导 readiness，便于单测。
fn build_readiness(
    registry: &TaskRegistry,
    operation_health: &VenueOperationHealthSnapshot,
    now_ms: i64,
) -> (StatusCode, ReadinessResponse) {
    let (degraded, tasks) = task_health(registry, now_ms);
    let operations = readiness_operations(operation_health);
    let blocked = degraded || operations.blocked > 0;
    let degraded = blocked || operations.warn > 0;
    let status = if blocked {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (
        status,
        ReadinessResponse {
            status: if degraded { "degraded" } else { "ok" },
            timestamp_ms: now_ms,
            degraded,
            tasks,
            operations,
        },
    )
}

fn build_liveness(registry: &TaskRegistry, now_ms: i64) -> HealthResponse {
    let (degraded, tasks) = task_health(registry, now_ms);
    HealthResponse {
        status: if degraded { "degraded" } else { "ok" },
        version: env!("CARGO_PKG_VERSION"),
        timestamp_ms: now_ms,
        degraded,
        tasks,
    }
}

fn readiness_operations(
    operation_health: &VenueOperationHealthSnapshot,
) -> ReadinessOperationSummary {
    let attention: Vec<ReadinessOperation> = operation_health
        .rows
        .iter()
        .filter(|row| is_readiness_operation(row))
        .filter(|row| {
            matches!(
                row.status,
                VenueOperationStatus::Blocked | VenueOperationStatus::Warn
            )
        })
        .map(readiness_operation)
        .collect();
    let blocked = attention
        .iter()
        .filter(|row| row.status == VenueOperationStatus::Blocked)
        .count();
    let warn = attention
        .iter()
        .filter(|row| row.status == VenueOperationStatus::Warn)
        .count();
    ReadinessOperationSummary {
        blocked,
        warn,
        attention,
    }
}

fn is_readiness_operation(row: &VenueOperationHealth) -> bool {
    row.venue == "system" && row.operation.starts_with("storage:")
}

fn readiness_operation(row: &VenueOperationHealth) -> ReadinessOperation {
    ReadinessOperation {
        venue: row.venue.clone(),
        operation: row.operation.clone(),
        status: row.status,
        source: row.source.clone(),
        message: row.message.clone(),
        problem_code: row.problem.as_ref().map(|problem| problem.code.clone()),
    }
}

/// 汇总后台任务健康：总数、健康数与不健康任务明细。
fn task_health(registry: &TaskRegistry, now_ms: i64) -> (bool, TaskHealthSummary) {
    let total = registry.task_count();
    let unhealthy: Vec<TaskHealthIssue> = registry
        .unhealthy_tasks(now_ms)
        .into_iter()
        .map(|issue| TaskHealthIssue {
            name: issue.name.to_owned(),
            code: issue.kind.code().to_owned(),
            detail: issue.detail,
        })
        .collect();
    (
        !unhealthy.is_empty(),
        TaskHealthSummary {
            healthy: total.saturating_sub(unhealthy.len()),
            total,
            unhealthy,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_ok_when_all_tasks_healthy() {
        let registry = crate::task_registry::TaskRegistry::default();
        registry.register("snapshot", 5_000);
        registry.register("funding", 5_000);

        let (degraded, tasks) = task_health(&registry, common::time::now_ms());

        assert!(!degraded);
        assert_eq!(tasks.total, 2);
        assert_eq!(tasks.healthy, 2);
        assert!(tasks.unhealthy.is_empty());
    }

    #[test]
    fn liveness_response_excludes_runtime_topology() -> Result<(), serde_json::Error> {
        let registry = crate::task_registry::TaskRegistry::default();
        registry.register("snapshot", 5_000);

        let response = build_liveness(&registry, 1_000);
        let json = serde_json::to_value(response)?;

        assert!(json.get("bind").is_none());
        assert!(json.get("metrics").is_none());
        assert!(json.get("wsChannels").is_none());
        assert_eq!(
            json.get("status").and_then(serde_json::Value::as_str),
            Some("ok")
        );
        Ok(())
    }

    #[test]
    fn health_degraded_when_task_down() {
        let registry = crate::task_registry::TaskRegistry::default();
        registry.register("snapshot", 5_000);
        registry.register("funding", 5_000);
        registry.mark_exited("snapshot", "panicked: boom");

        let (degraded, tasks) = task_health(&registry, common::time::now_ms());

        assert!(degraded);
        assert_eq!(tasks.total, 2);
        assert_eq!(tasks.healthy, 1);
        assert_eq!(tasks.unhealthy.len(), 1);
        assert_eq!(tasks.unhealthy[0].name, "snapshot");
        assert_eq!(tasks.unhealthy[0].code, "TASK_DOWN");
        assert!(tasks.unhealthy[0].detail.contains("boom"));
    }

    #[test]
    fn ready_returns_200_when_all_tasks_healthy() {
        let registry = crate::task_registry::TaskRegistry::default();
        registry.register("snapshot", 5_000);
        registry.register("funding", 5_000);

        let operation_health = empty_operation_health();
        let (status, body) = build_readiness(&registry, &operation_health, common::time::now_ms());

        assert_eq!(status, StatusCode::OK);
        assert!(!body.degraded);
        assert_eq!(body.status, "ok");
        assert_eq!(body.tasks.total, 2);
        assert!(body.tasks.unhealthy.is_empty());
        assert_eq!(body.operations.blocked, 0);
    }

    #[test]
    fn ready_returns_503_when_task_down() {
        let registry = crate::task_registry::TaskRegistry::default();
        registry.register("snapshot", 5_000);
        registry.register("funding", 5_000);
        registry.mark_exited("snapshot", "panicked: boom");
        let operation_health = empty_operation_health();

        let (status, body) = build_readiness(&registry, &operation_health, common::time::now_ms());

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.degraded);
        assert_eq!(body.status, "degraded");
        assert_eq!(body.tasks.unhealthy.len(), 1);
        assert_eq!(body.tasks.unhealthy[0].code, "TASK_DOWN");
    }

    #[test]
    fn ready_returns_503_when_system_storage_blocked() {
        let registry = crate::task_registry::TaskRegistry::default();
        registry.register("snapshot", 5_000);
        let operation_health =
            operation_health_with(system_storage_row(VenueOperationStatus::Blocked));

        let (status, body) = build_readiness(&registry, &operation_health, common::time::now_ms());

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.degraded);
        assert_eq!(body.operations.blocked, 1);
        assert_eq!(body.operations.attention[0].operation, "storage:history");
    }

    #[test]
    fn ready_keeps_200_for_system_storage_warn() {
        let registry = crate::task_registry::TaskRegistry::default();
        registry.register("snapshot", 5_000);
        let operation_health =
            operation_health_with(system_storage_row(VenueOperationStatus::Warn));

        let (status, body) = build_readiness(&registry, &operation_health, common::time::now_ms());

        assert_eq!(status, StatusCode::OK);
        assert!(body.degraded);
        assert_eq!(body.operations.warn, 1);
        assert_eq!(body.status, "degraded");
    }

    fn empty_operation_health() -> VenueOperationHealthSnapshot {
        VenueOperationHealthSnapshot::new(Vec::new(), common::time::now_ms())
    }

    fn operation_health_with(row: VenueOperationHealth) -> VenueOperationHealthSnapshot {
        VenueOperationHealthSnapshot::new(vec![row], common::time::now_ms())
    }

    fn system_storage_row(status: VenueOperationStatus) -> VenueOperationHealth {
        VenueOperationHealth {
            venue: "system".to_owned(),
            operation: "storage:history".to_owned(),
            status,
            source: "history_store".to_owned(),
            message: "history storage check".to_owned(),
            supported: Some(true),
            configured: Some(true),
            requested: None,
            rows: None,
            freshness_ms: Some(0),
            retry_after_ms: None,
            latency_ms: None,
            latency_p95_ms: None,
            error: None,
            evidence: None,
            problem: None,
            observed_at_ms: common::time::now_ms(),
        }
    }
}
