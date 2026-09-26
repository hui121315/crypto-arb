use crate::state::load_state::LoadState;
use shared_types::{ApiProblem, VenueOperationHealthSnapshot};

pub(super) fn worker_problem(health: &LoadState<VenueOperationHealthSnapshot>) -> Option<ApiProblem> {
    let LoadState::Ready(snapshot) = health else {
        return Some(health.problem().cloned().unwrap_or_else(|| {
            ApiProblem::new("AUTOMATION_WORKER_UNKNOWN", "等待后台自动化任务健康确认")
        }));
    };
    // A readable configuration is not proof that the worker is running.
    let Some(worker) = snapshot.rows.iter().find(|row| {
        row.venue == "system" && row.operation == "background_task:automated-arbitrage"
            && row.source == "task_registry"
    }) else {
        return Some(ApiProblem::new("AUTOMATION_WORKER_MISSING", "后台未登记自动化任务，暂不能启动或恢复"));
    };
    if worker.is_currently_usable() && worker.observed_at_ms > 0 {
        return None;
    }
    let mut problem = worker.problem.clone().unwrap_or_else(|| {
        ApiProblem::new("AUTOMATION_WORKER_UNAVAILABLE", worker.message.clone())
            .with_source("task_registry")
    });
    problem.message = format!("自动化任务未就绪：{}", worker.message);
    Some(problem)
}
