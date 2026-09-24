use super::{execution_status_needs_poll, settlement_needs_poll};
use crate::api::rest::{ApiClient, ApiError};
use crate::state::polling::{now_ms, use_conditional_polling_result};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{OnchainExecutionRunsResponse, OnchainExecutionSubmitResponse};
use std::time::Duration;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::onchain) struct ExecutionHistory {
    pub state: ExecutionRecords,
    pub refresh: Callback<()>,
}

#[derive(Clone, Copy)]
pub(in crate::panels::modules::onchain) struct ExecutionRecords {
    pub loaded: RwSignal<bool>,
    pub reading: RwSignal<bool>,
    pub rows: RwSignal<Vec<OnchainExecutionSubmitResponse>>,
    pub selected: RwSignal<Option<Result<OnchainExecutionSubmitResponse, String>>>,
    pub problem: RwSignal<Option<String>>,
    pub pending_build: RwSignal<Option<String>>,
    pub submitting: RwSignal<bool>,
    revision: RwSignal<u64>,
    retry_at: RwSignal<u64>,
    refresh_due: RwSignal<bool>,
    storage_key: StoredValue<String>,
}

impl ExecutionRecords {
    fn new(base: &str) -> Self {
        let key = format!("onchain-execution-pending:{base}");
        Self {
            loaded: RwSignal::new(false),
            reading: RwSignal::new(false),
            rows: RwSignal::new(Vec::new()),
            selected: RwSignal::new(None),
            problem: RwSignal::new(Some("正在读取执行记录；尚未开放提交".into())),
            pending_build: RwSignal::new(read_pending(&key)),
            submitting: RwSignal::new(false),
            revision: RwSignal::new(0),
            retry_at: RwSignal::new(0),
            refresh_due: RwSignal::new(true),
            storage_key: StoredValue::new(key),
        }
    }

    pub(in crate::panels::modules::onchain) fn build_used(self, id: &str) -> bool {
        self.pending_build
            .with(|pending| pending.as_deref() == Some(id))
            || self
                .rows
                .with(|rows| rows.iter().any(|run| run.build_id == id))
    }

    pub(in crate::panels::modules::onchain) fn select(self, id: &str) {
        if let Some(run) = self
            .rows
            .with_untracked(|rows| rows.iter().find(|run| run.run_id == id).cloned())
        {
            self.selected.set(Some(Ok(run)));
        }
    }

    pub(super) fn begin_submission(self, build_id: &str) -> bool {
        if self.submitting.try_get_untracked() != Some(false)
            || !self.loaded.get_untracked()
            || self.problem.get_untracked().is_some()
            || self.pending_build.get_untracked().is_some()
            || self
                .rows
                .with_untracked(|rows| rows.iter().any(|run| run.build_id == build_id))
        {
            return false;
        }
        // Persist only the build identifier, before sending; reload must not unlock an unknown write.
        if !write_pending(&self.storage_key.get_value(), Some(build_id)) {
            self.problem.set(Some(
                "无法保存待核验的构建编号，未发送执行请求；请检查浏览器存储".into(),
            ));
            return false;
        }
        self.revision.update(|value| *value = value.wrapping_add(1));
        self.pending_build.set(Some(build_id.into()));
        self.submitting.set(true);
        true
    }

    pub(super) fn finish_submission(
        self,
        build_id: &str,
        result: Result<OnchainExecutionSubmitResponse, ApiError>,
    ) {
        if self.submitting.try_get_untracked().is_none() {
            return;
        }
        match result {
            Ok(run) if run.build_id == build_id => {
                self.merge_rows(vec![run.clone()]);
                self.selected.set(Some(Ok(run)));
                self.clear_pending();
            }
            Ok(_) => self
                .problem
                .set(Some("回执构建编号不匹配，正在核对原执行记录".into())),
            Err(error) => {
                // Only explicit pre-write rejections prove that this request did not submit funds.
                if matches!(
                    error.problem.code.as_str(),
                    "ONCHAIN_BUILD_NOT_FOUND"
                        | "ONCHAIN_BUILD_EXPIRED"
                        | "ONCHAIN_EXECUTION_CONFIG_CHANGED"
                        | "ONCHAIN_SUBMISSION_NOT_READY"
                ) {
                    self.clear_pending();
                } else {
                    self.problem
                        .set(Some("执行反馈未确认；只查询原构建记录，不重复提交".into()));
                }
                self.selected.set(Some(Err(error.to_string())));
            }
        }
        self.submitting.set(false);
        self.refresh_due.set(true);
        self.retry_at.set(0);
    }

    fn clear_pending(self) {
        self.pending_build.set(None);
        write_pending(&self.storage_key.get_value(), None);
    }

    fn merge_rows(self, incoming: Vec<OnchainExecutionSubmitResponse>) {
        let before = self.rows.get_untracked();
        let mut rows = before.clone();
        for next in incoming {
            if let Some(row) = rows.iter_mut().find(|row| row.run_id == next.run_id) {
                if next.updated_at_ms >= row.updated_at_ms {
                    *row = next;
                }
            } else {
                rows.push(next);
            }
        }
        rows.sort_by(|a, b| {
            b.started_at_ms
                .cmp(&a.started_at_ms)
                .then_with(|| a.run_id.cmp(&b.run_id))
        });
        rows.truncate(100);
        if rows != before {
            self.rows.set(rows);
        }
    }

    fn begin_read(self) -> Option<u64> {
        if self.reading.try_get_untracked() != Some(false) || self.submitting.get_untracked() {
            return None;
        }
        self.reading.set(true);
        Some(self.revision.get_untracked())
    }

    fn finish_read(self, revision: u64, result: Result<OnchainExecutionRunsResponse, ApiError>) {
        if self.reading.try_get_untracked().is_none() {
            return;
        }
        self.reading.set(false);
        if self.revision.get_untracked() != revision {
            return;
        }
        self.refresh_due.set(false);
        match result {
            Ok(snapshot) => self.apply_snapshot(snapshot),
            Err(error) => self
                .problem
                .set(Some(format!("执行记录刷新失败，保留已有回执：{error}"))),
        }
        self.retry_at.set(
            now_ms()
                + if self.problem.get_untracked().is_some() {
                    5_000
                } else {
                    0
                },
        );
    }

    fn apply_snapshot(self, snapshot: OnchainExecutionRunsResponse) {
        self.loaded.set(true);
        self.merge_rows(snapshot.rows);
        let pending = self.pending_build.get_untracked();
        if let Some(build_id) = pending.as_deref() {
            if let Some(run) = self
                .rows
                .with_untracked(|rows| rows.iter().find(|run| run.build_id == build_id).cloned())
            {
                self.selected.set(Some(Ok(run)));
                self.clear_pending();
            }
        } else {
            match self.selected.get_untracked() {
                Some(Ok(current)) => {
                    if let Some(latest) = self.rows.with_untracked(|rows| {
                        rows.iter()
                            .find(|run| run.run_id == current.run_id)
                            .cloned()
                    }) {
                        if latest.updated_at_ms >= current.updated_at_ms && latest != current {
                            self.selected.set(Some(Ok(latest)));
                        }
                    }
                }
                None => {
                    let restored = self.rows.with_untracked(|rows| rows.iter().find(|run| matches!(run.status,
                        shared_types::OnchainExecutionRunStatus::Executing | shared_types::OnchainExecutionRunStatus::AwaitingChainFinality
                        | shared_types::OnchainExecutionRunStatus::Exposed | shared_types::OnchainExecutionRunStatus::FinalityUnresolved))
                        .or_else(|| rows.first()).cloned());
                    self.selected.set(restored.map(Ok));
                }
                Some(Err(_)) => {}
            }
        }
        self.problem.set(snapshot.recovery_problem.or_else(|| {
            self.pending_build
                .get_untracked()
                .map(|id| format!("尚未找到构建 {id} 的执行回执；保持待核验，请勿重复提交"))
        }));
    }

    fn needs_read(self) -> bool {
        self.reading.try_get_untracked() == Some(false)
            && !self.submitting.get_untracked()
            && now_ms() >= self.retry_at.get_untracked()
            && (self.refresh_due.get_untracked()
                || !self.loaded.get_untracked()
                || self.problem.get_untracked().is_some()
                || self.pending_build.get_untracked().is_some()
                || self.rows.with_untracked(|rows| {
                    rows.iter().any(|run| {
                        execution_status_needs_poll(run.status)
                            || settlement_needs_poll(run, now_ms() as i64)
                    })
                }))
    }
}

pub(super) fn use_execution_history(client: &ApiClient) -> ExecutionHistory {
    let state = ExecutionRecords::new(&client.base_url());
    let refresh = Callback::new({
        let client = client.clone();
        move |()| {
            let Some(revision) = state.begin_read() else {
                return;
            };
            let client = client.clone();
            spawn_local(async move {
                state.finish_read(revision, client.onchain_execution_runs(100).await);
            });
        }
    });
    let poll =
        use_conditional_polling_result(Duration::from_secs(2), move || state.needs_read(), {
            let client = client.clone();
            move || {
                let revision = state.begin_read();
                let client = client.clone();
                async move {
                    Ok::<_, ()>(match revision {
                        Some(revision) => {
                            Some((revision, client.onchain_execution_runs(100).await))
                        }
                        None => None,
                    })
                }
            }
        });
    Effect::new(move |_| {
        if let Some(Ok(Some((revision, result)))) =
            poll.get().and_then(|event| event.take().into_fetched())
        {
            state.finish_read(revision, result);
        }
    });
    ExecutionHistory { state, refresh }
}

fn read_pending(key: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_storage::Storage;
        gloo_storage::SessionStorage::get(key).ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        None
    }
}

fn write_pending(key: &str, pending: Option<&str>) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_storage::Storage;
        if let Some(id) = pending {
            gloo_storage::SessionStorage::set(key, id).is_ok()
        } else {
            gloo_storage::SessionStorage::delete(key);
            true
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (key, pending);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::execution_recovery_tests::completed_run;
    use super::*;

    fn snapshot(rows: Vec<OnchainExecutionSubmitResponse>) -> OnchainExecutionRunsResponse {
        OnchainExecutionRunsResponse {
            rows,
            recovery_problem: None,
            observed_at_ms: 100,
        }
    }

    #[test]
    fn timeout_recovers_only_the_original_build_and_empty_reads_do_not_unlock_it() {
        Owner::new().with(|| {
            let state = ExecutionRecords::new("fixture");
            assert!(!state.begin_submission("build"));
            state.apply_snapshot(snapshot(vec![]));
            assert!(state.begin_submission("build"));
            state.finish_submission("build", Err(ApiError::client("TIMEOUT", "reply lost")));
            let mut other = completed_run();
            other.build_id = "other-build".into();
            other.run_id = "other-run".into();
            state.apply_snapshot(snapshot(vec![other]));
            assert_eq!(
                state.pending_build.get_untracked().as_deref(),
                Some("build")
            );
            assert!(!state.begin_submission("new-build"));
            state.apply_snapshot(snapshot(vec![]));
            assert!(state.problem.get_untracked().is_some());
            state.apply_snapshot(snapshot(vec![completed_run()]));
            assert_eq!(state.selected.get_untracked(), Some(Ok(completed_run())));
            assert!(state.pending_build.get_untracked().is_none());
            assert!(!state.begin_submission("build"));
            assert!(state.begin_submission("new-build"));
        });
    }

    #[test]
    fn older_read_cannot_erase_a_new_submission_barrier() {
        Owner::new().with(|| {
            let state = ExecutionRecords::new("fixture");
            state.apply_snapshot(snapshot(vec![]));
            let revision = state.begin_read().unwrap();
            assert!(state.begin_read().is_none());
            assert!(state.begin_submission("build"));
            state.finish_submission("build", Err(ApiError::client("TIMEOUT", "reply lost")));
            state.finish_read(revision, Ok(snapshot(vec![completed_run()])));
            assert!(state.problem.get_untracked().is_some());
            assert!(state.pending_build.get_untracked().is_some());
            assert!(state.rows.get_untracked().is_empty());
        });
    }

    #[test]
    fn history_keeps_completed_receipts_and_selection_without_regression() {
        Owner::new().with(|| {
            let state = ExecutionRecords::new("fixture");
            let current = completed_run();
            state.apply_snapshot(snapshot(vec![current.clone()]));
            assert_eq!(state.selected.get_untracked(), Some(Ok(current.clone())));
            let mut older = current.clone();
            older.updated_at_ms = 10;
            older.status = shared_types::OnchainExecutionRunStatus::Executing;
            let mut other = current.clone();
            other.run_id = "other".into();
            other.build_id = "other-build".into();
            state.apply_snapshot(snapshot(vec![older, other.clone()]));
            assert_eq!(state.selected.get_untracked(), Some(Ok(current.clone())));
            state.select("other");
            state.apply_snapshot(snapshot(vec![current]));
            assert_eq!(state.selected.get_untracked(), Some(Ok(other)));
            assert_eq!(state.rows.get_untracked().len(), 2);
        });
    }

    #[test]
    fn known_rejection_is_distinct_from_already_submitted_or_transport_failure() {
        Owner::new().with(|| {
            for (code, unresolved) in [
                ("ONCHAIN_BUILD_EXPIRED", false),
                ("ONCHAIN_BUILD_ALREADY_SUBMITTED", true),
                ("NETWORK", true),
            ] {
                let state = ExecutionRecords::new("fixture");
                state.apply_snapshot(snapshot(vec![]));
                assert!(state.begin_submission("build"));
                state.finish_submission("build", Err(ApiError::client(code, "fixture")));
                state.apply_snapshot(snapshot(vec![]));
                assert_eq!(state.pending_build.get_untracked().is_some(), unresolved);
                assert_eq!(state.problem.get_untracked().is_some(), unresolved);
                assert!(matches!(state.selected.get_untracked(), Some(Err(_))));
            }
        });
    }

    #[test]
    fn ack_stays_executing_and_backend_journal_failure_is_preserved() {
        Owner::new().with(|| {
            let state = ExecutionRecords::new("fixture");
            state.apply_snapshot(snapshot(vec![]));
            assert!(state.begin_submission("build"));
            let mut ack = completed_run();
            ack.status = shared_types::OnchainExecutionRunStatus::Executing;
            state.finish_submission("build", Ok(ack.clone()));
            let mut read = snapshot(vec![]);
            read.recovery_problem = Some("journal unavailable".into());
            state.apply_snapshot(read);
            assert_eq!(state.selected.get_untracked(), Some(Ok(ack)));
            assert_eq!(
                state.problem.get_untracked().as_deref(),
                Some("journal unavailable")
            );
            state.apply_snapshot(snapshot(vec![completed_run()]));
            assert!(state.problem.get_untracked().is_none());
        });
    }
}
