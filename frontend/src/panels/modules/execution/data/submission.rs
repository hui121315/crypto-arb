use leptos::prelude::*;
use shared_types::{ApiProblem, ExecutionRun, ExecutionRunState, HedgeConfirmContext};

/// A durable, backend-scoped identity for a write whose response may be lost.
#[derive(Clone, Copy)]
pub(in crate::panels::modules::execution) struct SubmissionRecovery {
    pub pending: RwSignal<Option<HedgeConfirmContext>>,
    pub sending: RwSignal<bool>,
    pub storage_problem: RwSignal<Option<ApiProblem>>,
    base: StoredValue<String>,
    key: StoredValue<String>,
}

impl SubmissionRecovery {
    pub(super) fn new(base: String) -> Self {
        let key = format!("crossline.execution.pendingConfirm:{base}");
        let restored = read_pending(&key);
        Self {
            pending: RwSignal::new(restored.as_ref().ok().cloned().flatten()),
            sending: RwSignal::new(false),
            storage_problem: RwSignal::new(restored.err().map(storage_problem)),
            base: StoredValue::new(base),
            key: StoredValue::new(key),
        }
    }

    pub(in crate::panels::modules::execution) fn blocked(self) -> bool {
        self.pending.get().is_some() || self.sending.get() || self.storage_problem.get().is_some()
    }

    pub(super) fn matches_backend(self, base: &str) -> bool {
        self.base.try_get_value().is_some_and(|saved| saved == base)
    }

    pub(super) fn begin(self, context: &HedgeConfirmContext, base: &str) -> bool {
        if self.pending.try_get_untracked().is_none() || self.blocked() {
            return false;
        }
        if !self.matches_backend(base) {
            self.storage_problem.set(Some(storage_problem(
                "API 地址已改变，请刷新页面后核对原请求".into(),
            )));
            return false;
        }
        if let Err(error) = write_pending(&self.key.get_value(), Some(context)) {
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
        if !self
            .pending
            .try_get_untracked()
            .flatten()
            .is_some_and(|context| context.idempotency_key == key)
        {
            return false;
        }
        if let Err(error) = write_pending(&self.key.get_value(), None) {
            self.storage_problem.set(Some(storage_problem(error)));
            return false;
        }
        self.pending.set(None);
        self.storage_problem.set(None);
        true
    }
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
        serde_json::from_str(&raw).map_err(|_| "原提交记录无法解析，请先核对后台执行记录".into())
    })
    .transpose()
}

#[cfg(target_arch = "wasm32")]
fn write_pending(key: &str, context: Option<&HedgeConfirmContext>) -> Result<(), String> {
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
            .map_err(|_| "无法更新已核验的原提交记录")?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_pending(_: &str) -> Result<Option<HedgeConfirmContext>, String> {
    Ok(None)
}
#[cfg(not(target_arch = "wasm32"))]
fn write_pending(_: &str, _: Option<&HedgeConfirmContext>) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_write_survives_sending_flag_and_only_matching_key_can_clear_it() {
        Owner::new().with(|| {
            let recovery = SubmissionRecovery::new("fixture".into());
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
            let recovery = SubmissionRecovery::new("fixture-a".into());
            assert!(!recovery.begin(&HedgeConfirmContext::default(), "fixture-b"));
            assert!(recovery.storage_problem.get_untracked().is_some());
        });
    }
}
