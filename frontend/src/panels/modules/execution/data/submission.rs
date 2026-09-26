use super::connection::ExecutionConnection;
use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionRun, ExecutionRunState, HedgeConfirmContext};

/// A durable identity scoped to the original backend and login, never a retry instruction.
#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct SubmissionRecovery {
    pub pending: RwSignal<Option<HedgeConfirmContext>>,
    pub sending: RwSignal<bool>,
    pub storage_problem: RwSignal<Option<ApiProblem>>,
    pub(super) connection: ExecutionConnection,
    key: StoredValue<String>,
    legacy_key: StoredValue<String>,
    pub legacy: RwSignal<Option<HedgeConfirmContext>>,
}

impl SubmissionRecovery {
    pub(super) fn new(connection: ExecutionConnection) -> Self {
        let key = connection.key("crossline.execution.pendingConfirm.v2");
        let legacy_key = format!(
            "crossline.execution.pendingConfirm:{}",
            connection.client().base_url()
        );
        let restored = read_pending(&key);
        let legacy = read_pending(&legacy_key);
        Self {
            pending: RwSignal::new(restored.as_ref().ok().cloned().flatten()),
            sending: RwSignal::new(false),
            storage_problem: RwSignal::new(
                restored
                    .err()
                    .or_else(|| legacy.as_ref().err().cloned())
                    .map(storage_problem),
            ),
            connection,
            key: StoredValue::new(key),
            legacy_key: StoredValue::new(legacy_key),
            legacy: RwSignal::new(legacy.ok().flatten()),
        }
    }

    pub(in crate::panels::modules::execution) fn blocked(self) -> bool {
        !self.connection.available()
            || self.pending.get().is_some()
            || self.sending.get()
            || self.storage_problem.get().is_some()
            || self.legacy.get().is_some()
    }

    pub(super) fn matches_backend(self, base: &str) -> bool {
        self.connection.current() && self.connection.client().base_url() == base
    }

    pub(super) fn begin(self, context: &HedgeConfirmContext, base: &str) -> bool {
        if self.pending.try_get_untracked().is_none() {
            return false;
        }
        if !self.matches_backend(base) {
            self.storage_problem.set(Some(storage_problem(
                "API 地址已改变，请刷新页面后核对原请求".into(),
            )));
            return false;
        }
        if self.blocked() {
            return false;
        }
        if !valid_context(context) {
            self.storage_problem
                .set(Some(storage_problem("提交身份不完整，未发送请求".into())));
            return false;
        }
        if let Err(error) = write_pending(&self.key.get_value(), None, Some(context)) {
            self.storage_problem.set(Some(storage_problem(error)));
            return false;
        }
        self.pending.set(Some(context.clone()));
        self.sending.set(true);
        true
    }

    pub(super) fn resolve_run(self, run: &ExecutionRun) -> bool {
        let Some(Some(context)) = self.pending.try_get_untracked() else {
            return false;
        };
        if run.state == ExecutionRunState::Previewed || !request_matches_run(&context, run) {
            return false;
        }
        self.resolve(&context.idempotency_key)
    }

    pub(super) fn resolve(self, key: &str) -> bool {
        if !self.connection.current() {
            return false;
        }
        if !self
            .pending
            .try_get_untracked()
            .flatten()
            .is_some_and(|context| context.idempotency_key == key)
        {
            return false;
        }
        if let Err(error) = write_pending(
            &self.key.get_value(),
            self.pending.get_untracked().as_ref(),
            None,
        ) {
            self.storage_problem.set(Some(storage_problem(error)));
            return false;
        }
        self.pending.set(None);
        self.storage_problem.set(None);
        true
    }

    pub(in crate::panels::modules::execution) fn verify_legacy(self, refresh: RwSignal<u64>) {
        if !self.connection.current()
            || self.sending.get_untracked()
            || self.pending.get_untracked().is_some()
        {
            return;
        }
        let Some(context) = self.legacy.get_untracked() else {
            return;
        };
        self.sending.set(true);
        let client = self.connection.client();
        leptos::task::spawn_local(async move {
            let run_id = context
                .run_id
                .clone()
                .unwrap_or_else(|| format!("run-{}", context.idempotency_key));
            let result = client
                .execution_runs_for_context(
                    Some(&context.opportunity_id),
                    context.ticket_id.as_deref(),
                    Some(&run_id),
                )
                .await;
            if !self.connection.current() {
                return;
            }
            self.sending.set(false);
            let proven = result.as_ref().is_ok_and(|envelope| {
                envelope.status == shared_types::ListStatus::Fresh
                    && envelope.problems.is_empty()
                    && envelope.rows.iter().any(|run| {
                        request_matches_run(&context, run)
                            && run.state != ExecutionRunState::Previewed
                    })
            });
            if !proven {
                self.storage_problem.set(Some(storage_problem(
                    "当前登录未取得匹配的原运行记录；旧版记录已保留，未提交新订单。请确认原 API 地址和 Token。".into(),
                )));
                return;
            }
            // Bind only after an authenticated, exact backend match. Keep either record on failure.
            let saved = write_pending(&self.key.get_value(), None, Some(&context));
            if let Err(error) = saved {
                self.storage_problem.set(Some(storage_problem(error)));
                return;
            }
            self.pending.set(Some(context.clone()));
            if let Err(error) = write_pending(&self.legacy_key.get_value(), Some(&context), None) {
                self.storage_problem.set(Some(storage_problem(error)));
                return;
            }
            self.legacy.set(None);
            self.storage_problem.set(None);
            refresh.update(|value| *value = value.wrapping_add(1));
        });
    }
}

fn valid_context(context: &HedgeConfirmContext) -> bool {
    !context.opportunity_id.trim().is_empty()
        && !context.idempotency_key.trim().is_empty()
        && context
            .ticket_id
            .as_ref()
            .is_some_and(|id| !id.trim().is_empty())
}

pub(super) fn request_matches_run(context: &HedgeConfirmContext, run: &ExecutionRun) -> bool {
    !context.idempotency_key.is_empty()
        && context.opportunity_id == run.opportunity_id
        && context.ticket_id.as_deref() == Some(run.ticket_id.as_str())
        && context.run_id.as_deref().map_or_else(
            || run.run_id == format!("run-{}", context.idempotency_key),
            |id| id == run.run_id,
        )
}

fn storage_problem(message: String) -> ApiProblem {
    ApiProblem::new("EXECUTION_RECOVERY_STORAGE", message)
        .with_source("frontend.execution_recovery")
}

#[cfg(target_arch = "wasm32")]
fn read_pending(key: &str) -> Result<Option<HedgeConfirmContext>, String> {
    let storage = web_sys::window()
        .ok_or("浏览器存储不可用")?
        .local_storage()
        .map_err(|_| "浏览器存储不可用")?
        .ok_or("浏览器存储不可用")?;
    let raw = storage.get_item(key).map_err(|_| "无法读取原提交记录")?;
    raw.map(|raw| {
        let context = serde_json::from_str(&raw)
            .map_err(|_| "原提交记录无法解析，请先核对后台执行记录".to_string())?;
        if !valid_context(&context) {
            return Err("原提交身份不完整，请先核对后台执行记录".into());
        }
        Ok(context)
    })
    .transpose()
}

#[cfg(target_arch = "wasm32")]
fn write_pending(
    key: &str,
    expected: Option<&HedgeConfirmContext>,
    context: Option<&HedgeConfirmContext>,
) -> Result<(), String> {
    if read_pending(key)?.as_ref() != expected {
        return Err("原提交记录已被其他页面改变，请刷新核对；未覆盖记录".into());
    }
    let storage = web_sys::window()
        .ok_or("浏览器存储不可用")?
        .local_storage()
        .map_err(|_| "浏览器存储不可用")?
        .ok_or("浏览器存储不可用")?;
    if let Some(context) = context {
        let raw = serde_json::to_string(context).map_err(|_| "无法保存原提交记录")?;
        storage
            .set_item(key, &raw)
            .map_err(|_| "无法保存原提交记录，未发送新请求")?;
    } else {
        storage
            .remove_item(key)
            .map_err(|_| "无法更新已核对的原提交记录")?;
    }
    if read_pending(key)?.as_ref() != context {
        return Err("原提交记录未能可靠保存，未发送新请求".into());
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_pending(_: &str) -> Result<Option<HedgeConfirmContext>, String> {
    Ok(None)
}
#[cfg(not(target_arch = "wasm32"))]
fn write_pending(
    _: &str,
    _: Option<&HedgeConfirmContext>,
    _: Option<&HedgeConfirmContext>,
) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_write_survives_sending_flag_and_only_matching_key_can_clear_it() {
        Owner::new().with(|| {
            let connection = ExecutionConnection::from_signals(
                RwSignal::new("fixture".into()),
                RwSignal::new(String::new()),
            );
            let recovery = SubmissionRecovery::new(connection);
            let context = HedgeConfirmContext {
                opportunity_id: "opp".into(),
                idempotency_key: "key".into(),
                ticket_id: Some("ticket".into()),
                ..Default::default()
            };
            assert!(recovery.begin(&context, "fixture"));
            assert!(!recovery.begin(&context, "fixture"));
            recovery.sending.set(false);
            assert!(recovery.blocked());
            assert!(!recovery.resolve("other"));
            assert!(recovery.resolve("key"));
            assert!(!recovery.blocked());
        });
    }

    #[test]
    fn changing_backend_cannot_send_with_previous_recovery_namespace() {
        Owner::new().with(|| {
            let connection = ExecutionConnection::from_signals(
                RwSignal::new("fixture-a".into()),
                RwSignal::new(String::new()),
            );
            let recovery = SubmissionRecovery::new(connection);
            assert!(!recovery.begin(&HedgeConfirmContext::default(), "fixture-b"));
            assert!(recovery.storage_problem.get_untracked().is_some());
        });
    }
}
