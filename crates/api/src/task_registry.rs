//! 后台任务存活 + 进度登记表。
//!
//! 每个后台任务在 spawn 时 `register`（带预期迭代间隔），守护者在任务结束
//! （正常返回或 panic）时 `mark_exited`；任务体每轮迭代调用 `record_tick`
//! （存活/进度）或 `record_success`/`record_failure`（带成功语义的任务）。
//! `system_health::snapshot` 读取 `unhealthy_tasks` 把死亡 / 卡住 / 连续失败的
//! 任务暴露成 `RuntimeProblem`，使 `/health` 在后台任务异常时变 degraded。

use dashmap::DashMap;
use std::sync::Arc;

/// 连续失败多少次后判定任务为 Failing。
const FAILING_THRESHOLD: u32 = 3;
/// staleness 阈值 = max(interval * 倍数, 下限)，避免快任务抖动误报。
const SILENCE_MULTIPLIER: i64 = 5;
const SILENCE_FLOOR_MS: i64 = 60_000;
/// 单轮慢迭代阈值 = interval * 1.5，用于暴露任务仍运行但单次循环过慢。
const SLOW_MULTIPLIER_NUM: i64 = 3;
const SLOW_MULTIPLIER_DEN: i64 = 2;
const SLOW_FLOOR_MS: i64 = 1;

/// 单个后台任务的存活 + 进度记录。
#[derive(Clone, Debug)]
struct TaskEntry {
    enabled: bool,
    started_at_ms: i64,
    running: bool,
    last_tick_ms: i64,
    last_success_ms: Option<i64>,
    consecutive_failures: u32,
    last_error: Option<String>,
    last_exit_reason: Option<String>,
    last_exit_at_ms: Option<i64>,
    exit_count: u32,
    restart_count: u32,
    max_silence_ms: i64,
    slow_threshold_ms: i64,
    last_duration_ms: Option<u64>,
    slow_tick_count: u64,
    retry_at_ms: Option<i64>,
}

/// 不健康任务的类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskIssueKind {
    /// 任务已退出（正常返回或 panic）。
    Dead,
    /// 仍在运行但长时间无进度（疑似卡住）。
    Stale,
    /// 连续失败达到阈值。
    Failing,
}

impl TaskIssueKind {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Dead => "TASK_DOWN",
            Self::Stale => "TASK_STALE",
            Self::Failing => "TASK_FAILING",
        }
    }
}

/// 一个不健康后台任务的快照，供健康面板转成 `RuntimeProblem`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskIssue {
    pub name: &'static str,
    pub kind: TaskIssueKind,
    pub detail: String,
    pub since_ms: Option<i64>,
}

/// 单个后台任务的完整运行态快照。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskSnapshot {
    pub name: &'static str,
    pub enabled: bool,
    pub running: bool,
    pub started_at_ms: i64,
    pub last_tick_ms: i64,
    pub last_success_ms: Option<i64>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub last_exit_reason: Option<String>,
    pub last_exit_at_ms: Option<i64>,
    pub exit_count: u32,
    pub restart_count: u32,
    pub max_silence_ms: i64,
    pub slow_threshold_ms: i64,
    pub last_duration_ms: Option<u64>,
    pub slow_tick_count: u64,
    pub lag_ms: i64,
    pub retry_after_ms: Option<u64>,
    pub issue: Option<TaskIssue>,
}

/// 并发安全的后台任务登记表（克隆为廉价 `Arc` 克隆）。
#[derive(Clone, Default)]
pub(crate) struct TaskRegistry {
    inner: Arc<DashMap<&'static str, TaskEntry>>,
}

impl TaskRegistry {
    /// 登记一个后台任务为运行中，保留已有历史计数。
    /// `interval_ms` 为预期迭代间隔，用于推导 staleness 阈值。
    pub(crate) fn register(&self, name: &'static str, interval_ms: i64) {
        self.register_state(name, interval_ms, true);
    }

    /// 显式登记配置未启用的任务，使运行态能区分 disabled 与未知。
    pub(crate) fn register_disabled(&self, name: &'static str, interval_ms: i64) {
        self.register_state(name, interval_ms, false);
    }

    fn register_state(&self, name: &'static str, interval_ms: i64, enabled: bool) {
        let now = common::time::now_ms();
        let max_silence_ms = (interval_ms.max(0) * SILENCE_MULTIPLIER).max(SILENCE_FLOOR_MS);
        let slow_threshold_ms = slow_threshold_ms(interval_ms);
        self.inner
            .entry(name)
            .and_modify(|entry| {
                entry.enabled = enabled;
                entry.running = enabled;
                entry.started_at_ms = now;
                entry.last_tick_ms = now;
                entry.consecutive_failures = 0;
                entry.last_error = None;
                entry.max_silence_ms = max_silence_ms;
                entry.slow_threshold_ms = slow_threshold_ms;
                entry.last_duration_ms = None;
                entry.retry_at_ms = None;
            })
            .or_insert(TaskEntry {
                enabled,
                started_at_ms: now,
                running: enabled,
                last_tick_ms: now,
                last_success_ms: None,
                consecutive_failures: 0,
                last_error: None,
                last_exit_reason: None,
                last_exit_at_ms: None,
                exit_count: 0,
                restart_count: 0,
                max_silence_ms,
                slow_threshold_ms,
                last_duration_ms: None,
                slow_tick_count: 0,
                retry_at_ms: None,
            });
    }

    /// 记录 supervisor 已在预算内重新启动任务。
    pub(crate) fn record_restart(&self, name: &'static str) {
        if let Some(mut entry) = self.inner.get_mut(name) {
            let now = common::time::now_ms();
            entry.running = true;
            entry.enabled = true;
            entry.started_at_ms = now;
            entry.last_tick_ms = now;
            entry.consecutive_failures = 0;
            entry.last_error = None;
            entry.last_duration_ms = None;
            entry.restart_count = entry.restart_count.saturating_add(1);
            entry.retry_at_ms = None;
        }
    }

    /// 记录一次迭代完成（存活/进度心跳），用于无成功语义的任务。
    pub(crate) fn record_tick(&self, name: &'static str) {
        if let Some(mut entry) = self.inner.get_mut(name) {
            entry.last_tick_ms = common::time::now_ms();
        }
    }

    /// 记录一次成功迭代：刷新 tick / success，清零连续失败。
    #[cfg(test)]
    pub(crate) fn record_success(&self, name: &'static str) {
        self.record_success_at(name, common::time::now_ms(), None);
    }

    fn record_success_at(&self, name: &'static str, now: i64, duration_ms: Option<u64>) {
        if let Some(mut entry) = self.inner.get_mut(name) {
            entry.last_tick_ms = now;
            entry.last_success_ms = Some(now);
            entry.consecutive_failures = 0;
            entry.last_error = None;
            record_duration(&mut entry, duration_ms);
        }
    }

    /// 记录一次失败迭代：刷新 tick，累加连续失败并记录错误。
    #[cfg(test)]
    pub(crate) fn record_failure(&self, name: &'static str, error: impl Into<String>) {
        self.record_failure_at(name, common::time::now_ms(), error, None);
    }

    fn record_failure_at(
        &self,
        name: &'static str,
        now: i64,
        error: impl Into<String>,
        duration_ms: Option<u64>,
    ) {
        if let Some(mut entry) = self.inner.get_mut(name) {
            entry.last_tick_ms = now;
            entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
            entry.last_error = Some(error.into());
            record_duration(&mut entry, duration_ms);
        }
    }

    /// `record_success` / `record_failure` 的便捷封装。
    #[cfg(test)]
    pub(crate) fn record_result(&self, name: &'static str, result: Result<(), String>) {
        match result {
            Ok(()) => self.record_success(name),
            Err(error) => self.record_failure(name, error),
        }
    }

    /// `record_result` 的计时版本，暴露单轮耗时与慢迭代计数。
    pub(crate) fn record_result_timed(
        &self,
        name: &'static str,
        started_at_ms: i64,
        result: Result<(), String>,
    ) {
        let now = common::time::now_ms();
        let duration_ms = now.saturating_sub(started_at_ms).max(0) as u64;
        match result {
            Ok(()) => self.record_success_at(name, now, Some(duration_ms)),
            Err(error) => self.record_failure_at(name, now, error, Some(duration_ms)),
        }
    }

    /// 标记任务已停止运行，记录原因并累加退出计数。
    pub(crate) fn mark_exited(&self, name: &'static str, reason: impl Into<String>) {
        if let Some(mut entry) = self.inner.get_mut(name) {
            entry.running = false;
            entry.last_exit_reason = Some(reason.into());
            entry.last_exit_at_ms = Some(common::time::now_ms());
            entry.exit_count = entry.exit_count.saturating_add(1);
            entry.retry_at_ms = None;
        }
    }

    /// 记录 supervisor 已安排的有界重试，供运行健康面直接暴露剩余等待。
    pub(crate) fn mark_retry_scheduled(
        &self,
        name: &'static str,
        reason: impl Into<String>,
        retry_at_ms: i64,
    ) {
        if let Some(mut entry) = self.inner.get_mut(name) {
            let now = common::time::now_ms();
            entry.running = false;
            entry.last_exit_reason = Some(reason.into());
            entry.last_exit_at_ms = Some(now);
            entry.exit_count = entry.exit_count.saturating_add(1);
            entry.retry_at_ms = Some(retry_at_ms.max(now));
        }
    }

    /// 已登记的后台任务总数（含已退出但保留记录的任务）。
    pub(crate) fn task_count(&self) -> usize {
        self.inner.len()
    }

    /// 返回所有已登记任务的完整快照，按名字排序。
    pub(crate) fn task_snapshots(&self, now_ms: i64) -> Vec<TaskSnapshot> {
        let mut snapshots = self
            .inner
            .iter()
            .map(|entry| task_snapshot(entry.key(), &entry, now_ms))
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|snapshot| snapshot.name);
        snapshots
    }

    /// 返回所有不健康（Dead / Stale / Failing）的任务，按名字排序。
    pub(crate) fn unhealthy_tasks(&self, now_ms: i64) -> Vec<TaskIssue> {
        let mut issues = self
            .inner
            .iter()
            .filter_map(|entry| task_issue(entry.key(), &entry, now_ms))
            .collect::<Vec<_>>();
        issues.sort_by_key(|issue| issue.name);
        issues
    }
}

fn task_snapshot(name: &'static str, entry: &TaskEntry, now_ms: i64) -> TaskSnapshot {
    TaskSnapshot {
        name,
        enabled: entry.enabled,
        running: entry.running,
        started_at_ms: entry.started_at_ms,
        last_tick_ms: entry.last_tick_ms,
        last_success_ms: entry.last_success_ms,
        consecutive_failures: entry.consecutive_failures,
        last_error: entry.last_error.clone(),
        last_exit_reason: entry.last_exit_reason.clone(),
        last_exit_at_ms: entry.last_exit_at_ms,
        exit_count: entry.exit_count,
        restart_count: entry.restart_count,
        max_silence_ms: entry.max_silence_ms,
        slow_threshold_ms: entry.slow_threshold_ms,
        last_duration_ms: entry.last_duration_ms,
        slow_tick_count: entry.slow_tick_count,
        lag_ms: now_ms.saturating_sub(entry.last_tick_ms).max(0),
        retry_after_ms: entry
            .retry_at_ms
            .map(|retry_at_ms| retry_at_ms.saturating_sub(now_ms).max(0) as u64),
        issue: task_issue(name, entry, now_ms),
    }
}

fn slow_threshold_ms(interval_ms: i64) -> i64 {
    let interval_ms = interval_ms.max(SLOW_FLOOR_MS);
    interval_ms
        .saturating_mul(SLOW_MULTIPLIER_NUM)
        .saturating_div(SLOW_MULTIPLIER_DEN)
        .max(SLOW_FLOOR_MS)
}

fn record_duration(entry: &mut TaskEntry, duration_ms: Option<u64>) {
    let Some(duration_ms) = duration_ms else {
        return;
    };
    entry.last_duration_ms = Some(duration_ms);
    if duration_ms > entry.slow_threshold_ms.max(0) as u64 {
        entry.slow_tick_count = entry.slow_tick_count.saturating_add(1);
    }
}

fn task_issue(name: &'static str, entry: &TaskEntry, now_ms: i64) -> Option<TaskIssue> {
    if !entry.enabled {
        return None;
    }
    if !entry.running {
        return Some(TaskIssue {
            name,
            kind: TaskIssueKind::Dead,
            detail: entry
                .last_exit_reason
                .clone()
                .unwrap_or_else(|| "exited".to_owned()),
            since_ms: entry.last_exit_at_ms,
        });
    }
    let silent_ms = now_ms - entry.last_tick_ms;
    if silent_ms > entry.max_silence_ms {
        return Some(TaskIssue {
            name,
            kind: TaskIssueKind::Stale,
            detail: format!(
                "no progress for {silent_ms}ms (limit {}ms)",
                entry.max_silence_ms
            ),
            since_ms: Some(entry.last_tick_ms),
        });
    }
    if entry.consecutive_failures >= FAILING_THRESHOLD {
        return Some(TaskIssue {
            name,
            kind: TaskIssueKind::Failing,
            detail: format!(
                "{} consecutive failures: {}",
                entry.consecutive_failures,
                entry.last_error.as_deref().unwrap_or("unknown error")
            ),
            since_ms: entry.last_success_ms,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL: i64 = 5_000;

    fn now() -> i64 {
        common::time::now_ms()
    }

    #[test]
    fn registered_task_is_healthy() {
        let registry = TaskRegistry::default();
        registry.register("snapshot", INTERVAL);

        assert!(registry.unhealthy_tasks(now()).is_empty());
    }

    #[test]
    fn exited_task_is_dead() {
        let registry = TaskRegistry::default();
        registry.register("snapshot", INTERVAL);
        registry.mark_exited("snapshot", "panicked: boom");

        let issues = registry.unhealthy_tasks(now());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, TaskIssueKind::Dead);
        assert_eq!(issues[0].kind.code(), "TASK_DOWN");
        assert!(issues[0].detail.contains("boom"));
        assert!(issues[0].since_ms.is_some());
    }

    #[test]
    fn stuck_task_is_stale() {
        let registry = TaskRegistry::default();
        registry.register("snapshot", INTERVAL);

        let issues = registry.unhealthy_tasks(now() + 10 * 60_000);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, TaskIssueKind::Stale);
        assert_eq!(issues[0].kind.code(), "TASK_STALE");
    }

    #[test]
    fn fresh_tick_clears_stale() {
        let registry = TaskRegistry::default();
        registry.register("snapshot", INTERVAL);
        registry.record_tick("snapshot");

        assert!(registry.unhealthy_tasks(now()).is_empty());
    }

    #[test]
    fn consecutive_failures_trip_failing() {
        let registry = TaskRegistry::default();
        registry.register("funding", INTERVAL);
        for _ in 0..FAILING_THRESHOLD {
            registry.record_failure("funding", "no rates");
        }

        let issues = registry.unhealthy_tasks(now());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, TaskIssueKind::Failing);
        assert_eq!(issues[0].kind.code(), "TASK_FAILING");
        assert!(issues[0].detail.contains("no rates"));
    }

    #[test]
    fn success_resets_failures() {
        let registry = TaskRegistry::default();
        registry.register("funding", INTERVAL);
        for _ in 0..FAILING_THRESHOLD {
            registry.record_failure("funding", "x");
        }
        registry.record_success("funding");

        assert!(registry.unhealthy_tasks(now()).is_empty());
    }

    #[test]
    fn record_result_maps_ok_and_err() {
        let registry = TaskRegistry::default();
        registry.register("portfolio", INTERVAL);
        for _ in 0..FAILING_THRESHOLD {
            registry.record_result("portfolio", Err("boom".to_owned()));
        }
        assert_eq!(
            registry.unhealthy_tasks(now())[0].kind,
            TaskIssueKind::Failing
        );

        registry.record_result("portfolio", Ok(()));
        assert!(registry.unhealthy_tasks(now()).is_empty());
    }

    #[test]
    fn timed_result_records_duration_and_slow_tick() {
        let registry = TaskRegistry::default();
        registry.register("portfolio", INTERVAL);

        registry.record_result_timed("portfolio", now() - 10_000, Ok(()));

        let snapshot = registry.task_snapshots(now()).remove(0);
        assert!(snapshot
            .last_duration_ms
            .is_some_and(|duration_ms| duration_ms >= 10_000));
        assert_eq!(snapshot.slow_threshold_ms, 7_500);
        assert_eq!(snapshot.slow_tick_count, 1);
        assert!(snapshot.issue.is_none());
    }

    #[test]
    fn restart_keeps_exit_count_increments_restart_count_and_clears_dead() {
        let registry = TaskRegistry::default();
        registry.register("snapshot", INTERVAL);
        registry.mark_exited("snapshot", "panicked");
        registry.record_restart("snapshot");

        assert!(registry.unhealthy_tasks(now()).is_empty());
        let snapshot = registry.task_snapshots(now()).remove(0);
        assert_eq!(snapshot.exit_count, 1);
        assert_eq!(snapshot.restart_count, 1);
    }

    #[test]
    fn task_count_includes_exited_tasks() {
        let registry = TaskRegistry::default();
        assert_eq!(registry.task_count(), 0);

        registry.register("snapshot", INTERVAL);
        registry.register("funding", INTERVAL);
        assert_eq!(registry.task_count(), 2);

        registry.mark_exited("snapshot", "stopped");
        assert_eq!(registry.task_count(), 2);
    }

    #[test]
    fn unknown_task_records_are_ignored() {
        let registry = TaskRegistry::default();
        registry.record_tick("ghost");
        registry.record_failure("ghost", "x");
        registry.mark_exited("ghost", "x");

        assert!(registry.unhealthy_tasks(now()).is_empty());
    }

    #[test]
    fn task_snapshots_include_healthy_and_unhealthy_rows() {
        let registry = TaskRegistry::default();
        registry.register("funding", INTERVAL);
        registry.register("snapshot", INTERVAL);
        registry.mark_exited("snapshot", "panicked");

        let snapshots = registry.task_snapshots(now());

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].name, "funding");
        assert!(snapshots[0].issue.is_none());
        assert_eq!(snapshots[1].name, "snapshot");
        assert!(snapshots[1].issue.is_some());
        assert_eq!(snapshots[1].exit_count, 1);
        assert_eq!(snapshots[1].restart_count, 0);
    }

    #[test]
    fn unhealthy_tasks_are_sorted_by_name() {
        let registry = TaskRegistry::default();
        for name in ["system_health", "funding", "portfolio"] {
            registry.register(name, INTERVAL);
            registry.mark_exited(name, "stopped");
        }

        let names = registry
            .unhealthy_tasks(now())
            .into_iter()
            .map(|issue| issue.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["funding", "portfolio", "system_health"]);
    }

    #[test]
    fn disabled_task_is_explicit_without_becoming_unhealthy() {
        let registry = TaskRegistry::default();
        registry.register_disabled("ledger_projection_jobs", INTERVAL);

        let snapshot = registry.task_snapshots(now()).remove(0);

        assert!(!snapshot.enabled);
        assert!(!snapshot.running);
        assert_eq!(snapshot.retry_after_ms, None);
        assert!(snapshot.issue.is_none());
        assert!(registry.unhealthy_tasks(now()).is_empty());
    }

    #[test]
    fn scheduled_restart_exposes_retry_after_and_lag() {
        let registry = TaskRegistry::default();
        registry.register("funding", INTERVAL);
        let observed_at_ms = now();
        registry.mark_retry_scheduled("funding", "transient", observed_at_ms + 5_000);

        let snapshot = registry.task_snapshots(observed_at_ms + 1_000).remove(0);

        assert!(snapshot.enabled);
        assert!(!snapshot.running);
        assert!(snapshot.lag_ms >= 1_000);
        assert_eq!(snapshot.retry_after_ms, Some(4_000));
        assert_eq!(
            snapshot.issue.map(|issue| issue.kind),
            Some(TaskIssueKind::Dead)
        );
    }
}
