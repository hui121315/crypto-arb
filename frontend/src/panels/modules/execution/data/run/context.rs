//! 执行 run 上下文：从选择态 + localStorage 还原 run/ticket/opportunity 过滤键。
//!
//! 从 `run.rs` 拆出，单一职责负责「当前查询应针对哪个 run/ticket/opportunity」的
//! 解析与 localStorage 恢复；seed/stream 应用逻辑见 `seed.rs`，订阅与兜底见 `run.rs`。

use crate::panels::modules::execution::selection::ExecutionSelection;
use crate::state::module_runtime::{clear_choice, store_choice, stored_choice};
use shared_types::{ActionEvidence, ExecutionRun, HedgeConfirmContext};

const RUN_CONTEXT_OPPORTUNITY_KEY: &str = "crossline.execution.runContext.opportunityId";
const RUN_CONTEXT_TICKET_KEY: &str = "crossline.execution.runContext.ticketId";
const RUN_CONTEXT_RUN_KEY: &str = "crossline.execution.runContext.runId";
const RUN_CONTEXT_IDEMPOTENCY_KEY: &str = "crossline.execution.runContext.idempotencyKey";

pub(in crate::panels::modules::execution::data) fn store_execution_run_context(
    run: &ExecutionRun,
    idempotency_key: &str,
) {
    store_choice(RUN_CONTEXT_OPPORTUNITY_KEY, &run.opportunity_id);
    store_choice(RUN_CONTEXT_TICKET_KEY, &run.ticket_id);
    store_choice(RUN_CONTEXT_RUN_KEY, &run.run_id);
    store_choice(RUN_CONTEXT_IDEMPOTENCY_KEY, idempotency_key);
}

pub(in crate::panels::modules::execution::data) fn store_confirm_request_context(
    context: &HedgeConfirmContext,
) {
    store_choice(RUN_CONTEXT_OPPORTUNITY_KEY, &context.opportunity_id);
    store_optional_choice(RUN_CONTEXT_TICKET_KEY, context.ticket_id.as_deref());
    clear_choice(RUN_CONTEXT_RUN_KEY);
    store_choice(RUN_CONTEXT_IDEMPOTENCY_KEY, &context.idempotency_key);
}

pub(in crate::panels::modules::execution::data) fn store_workspace_route_context(
    opportunity_id: Option<&str>,
    run_id: Option<&str>,
    ticket_id: Option<&str>,
) {
    store_optional_choice(RUN_CONTEXT_OPPORTUNITY_KEY, opportunity_id);
    store_optional_choice(RUN_CONTEXT_TICKET_KEY, ticket_id);
    store_optional_choice(RUN_CONTEXT_RUN_KEY, run_id);
    clear_choice(RUN_CONTEXT_IDEMPOTENCY_KEY);
}

pub(in crate::panels::modules::execution::data) fn restored_execution_run_evidence(
    run: &ExecutionRun,
) -> ActionEvidence {
    let idempotency_key = restored_execution_run_matches(run)
        .then(|| stored_context_token(RUN_CONTEXT_IDEMPOTENCY_KEY))
        .flatten();
    ActionEvidence::from_execution_run(run).with_idempotency_key(idempotency_key)
}

pub(in crate::panels::modules::execution::data) fn restored_execution_run_matches(
    run: &ExecutionRun,
) -> bool {
    let stored_run_id = stored_context_token(RUN_CONTEXT_RUN_KEY);
    let stored_ticket_id = stored_context_token(RUN_CONTEXT_TICKET_KEY);
    (stored_run_id.is_some() || stored_ticket_id.is_some())
        && optional_id_matches(&stored_run_id, &run.run_id)
        && optional_id_matches(&stored_ticket_id, &run.ticket_id)
        && optional_id_matches(
            &stored_context_token(RUN_CONTEXT_OPPORTUNITY_KEY),
            &run.opportunity_id,
        )
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::panels::modules::execution::data::run) struct ExecutionRunContext {
    pub(in crate::panels::modules::execution::data::run) run_id: Option<String>,
    pub(in crate::panels::modules::execution::data::run) ticket_id: Option<String>,
    pub(in crate::panels::modules::execution::data::run) opportunity_id: Option<String>,
    pub(in crate::panels::modules::execution::data::run) idempotency_key: Option<String>,
    pub(in crate::panels::modules::execution::data::run) restored_without_selection: bool,
}

impl ExecutionRunContext {
    pub(in crate::panels::modules::execution::data::run) fn with_pending(
        selection: &ExecutionSelection,
        pending: Option<&HedgeConfirmContext>,
    ) -> Self {
        if let Some(context) = pending {
            return Self {
                run_id: Some(
                    context
                        .run_id
                        .clone()
                        .unwrap_or_else(|| format!("run-{}", context.idempotency_key)),
                ),
                ticket_id: context.ticket_id.clone(),
                opportunity_id: Some(context.opportunity_id.clone()),
                idempotency_key: Some(context.idempotency_key.clone()),
                restored_without_selection: selection.opportunity_id.is_empty(),
            };
        }
        Self::from_selection(selection)
    }

    pub(in crate::panels::modules::execution::data::run) fn from_selection(
        selection: &ExecutionSelection,
    ) -> Self {
        context_from_selection_and_stored(selection, stored_execution_run_context(selection))
    }

    pub(in crate::panels::modules::execution::data::run) fn has_filter(&self) -> bool {
        self.run_id.is_some() || self.ticket_id.is_some() || self.opportunity_id.is_some()
    }

    pub(in crate::panels::modules::execution::data::run) fn matches(
        &self,
        run: &ExecutionRun,
    ) -> bool {
        optional_id_matches(&self.run_id, &run.run_id)
            && optional_id_matches(&self.ticket_id, &run.ticket_id)
            && optional_id_matches(&self.opportunity_id, &run.opportunity_id)
    }

    #[cfg(test)]
    fn is_local_persisted_restore(&self) -> bool {
        self.restored_without_selection
            && self.idempotency_key.is_some()
            && (self.run_id.is_some() || self.ticket_id.is_some())
    }
}

fn context_from_selection_and_stored(
    selection: &ExecutionSelection,
    stored: Option<ExecutionRunContext>,
) -> ExecutionRunContext {
    let opportunity_id = clean_context_token(&selection.opportunity_id);
    if let Some(mut stored) =
        stored.filter(|stored| opportunity_id.is_none() || stored.opportunity_id == opportunity_id)
    {
        stored.restored_without_selection = opportunity_id.is_none();
        return stored;
    }
    ExecutionRunContext {
        opportunity_id,
        ..ExecutionRunContext::default()
    }
}

fn stored_execution_run_context(selection: &ExecutionSelection) -> Option<ExecutionRunContext> {
    let opportunity_id = clean_context_token(&selection.opportunity_id);
    let stored_opportunity = stored_context_token(RUN_CONTEXT_OPPORTUNITY_KEY);
    if opportunity_id.as_ref().is_some_and(|selected| {
        stored_opportunity
            .as_ref()
            .is_some_and(|stored| stored != selected)
    }) {
        return None;
    }
    let run_id = stored_context_token(RUN_CONTEXT_RUN_KEY);
    let ticket_id = stored_context_token(RUN_CONTEXT_TICKET_KEY);
    (stored_opportunity.is_some() || run_id.is_some() || ticket_id.is_some()).then(|| {
        ExecutionRunContext {
            opportunity_id: stored_opportunity,
            ticket_id,
            run_id,
            idempotency_key: stored_context_token(RUN_CONTEXT_IDEMPOTENCY_KEY),
            restored_without_selection: false,
        }
    })
}

fn stored_context_token(key: &str) -> Option<String> {
    stored_choice(key, clean_context_token)
}

pub(in crate::panels::modules::execution::data) fn clear_execution_run_context() {
    clear_choice(RUN_CONTEXT_OPPORTUNITY_KEY);
    clear_choice(RUN_CONTEXT_TICKET_KEY);
    clear_choice(RUN_CONTEXT_RUN_KEY);
    clear_choice(RUN_CONTEXT_IDEMPOTENCY_KEY);
}

fn store_optional_choice(key: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        store_choice(key, value);
    } else {
        clear_choice(key);
    }
}

fn optional_id_matches(expected: &Option<String>, actual: &str) -> bool {
    match expected {
        Some(expected) => actual == expected,
        None => true,
    }
}

fn clean_context_token(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_keeps_all_stored_run_identity_constraints() {
        let selection = selection("opp-a");
        let context = context_from_selection_and_stored(
            &selection,
            Some(ExecutionRunContext {
                opportunity_id: Some("opp-a".into()),
                ticket_id: Some("ticket-a".into()),
                run_id: Some("run-a".into()),
                idempotency_key: Some("idem-a".into()),
                restored_without_selection: false,
            }),
        );

        assert_eq!(context.run_id.as_deref(), Some("run-a"));
        assert_eq!(context.ticket_id.as_deref(), Some("ticket-a"));
        assert_eq!(context.opportunity_id.as_deref(), Some("opp-a"));
        assert_eq!(context.idempotency_key.as_deref(), Some("idem-a"));
    }

    #[test]
    fn context_ignores_stored_run_from_other_opportunity() {
        let selection = selection("opp-b");
        let context = context_from_selection_and_stored(
            &selection,
            Some(ExecutionRunContext {
                opportunity_id: Some("opp-a".into()),
                ticket_id: Some("ticket-a".into()),
                run_id: Some("run-a".into()),
                idempotency_key: Some("idem-a".into()),
                restored_without_selection: false,
            }),
        );

        assert!(context.run_id.is_none());
        assert!(context.ticket_id.is_none());
        assert_eq!(context.opportunity_id.as_deref(), Some("opp-b"));
    }

    #[test]
    fn empty_selection_restores_durable_run_context() {
        let context = context_from_selection_and_stored(
            &ExecutionSelection::empty(),
            Some(ExecutionRunContext {
                opportunity_id: Some("opp-a".into()),
                ticket_id: Some("ticket-a".into()),
                run_id: Some("run-a".into()),
                idempotency_key: Some("idem-a".into()),
                restored_without_selection: false,
            }),
        );

        assert_eq!(context.run_id.as_deref(), Some("run-a"));
        assert_eq!(context.opportunity_id.as_deref(), Some("opp-a"));
        assert_eq!(context.idempotency_key.as_deref(), Some("idem-a"));
        assert!(context.is_local_persisted_restore());
    }

    #[test]
    fn empty_selection_marks_local_confirm_ticket_restore() {
        let context = context_from_selection_and_stored(
            &ExecutionSelection::empty(),
            Some(ExecutionRunContext {
                opportunity_id: Some("opp-a".into()),
                ticket_id: Some("ticket-a".into()),
                idempotency_key: Some("idem-a".into()),
                ..ExecutionRunContext::default()
            }),
        );

        assert!(context.is_local_persisted_restore());
    }

    #[test]
    fn route_context_keeps_explicit_run_without_inventing_ticket_or_idempotency() {
        let context = ExecutionRunContext {
            opportunity_id: Some("opp-a".into()),
            run_id: Some("run-a".into()),
            ticket_id: None,
            idempotency_key: None,
            restored_without_selection: false,
        };

        assert_eq!(context.run_id.as_deref(), Some("run-a"));
        assert_eq!(context.opportunity_id.as_deref(), Some("opp-a"));
        assert!(context.ticket_id.is_none());
        assert!(context.idempotency_key.is_none());
        assert!(!context.is_local_persisted_restore());
    }

    fn selection(opportunity_id: &str) -> ExecutionSelection {
        ExecutionSelection {
            opportunity_id: opportunity_id.to_owned(),
            ..ExecutionSelection::empty()
        }
    }
}
