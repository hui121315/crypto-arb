use shared_types::{
    OnchainCrossChainLegRunStatus as LegStatus, OnchainCrossChainRecheckRequest,
    OnchainCrossChainRun, OnchainCrossChainRunStatus as RunStatus, OnchainCrossChainSubmitRequest,
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(in crate::panels::modules::onchain) struct PendingSubmission {
    pub request: OnchainCrossChainSubmitRequest,
    pub previous_updated_at_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::panels::modules::onchain) struct CrossChainRecovery {
    pub plans: Vec<shared_types::OnchainCrossChainRecoveryPlan>,
    pub rows: Vec<OnchainCrossChainRun>,
    pub selected_id: Option<String>,
    pub loaded: bool,
    pub problem: Option<String>,
    pub read_problem: Option<String>,
    pub recovery_problem: Option<String>,
    pub pending_authorization: Option<String>,
    pub pending_submission: Option<PendingSubmission>,
}

impl CrossChainRecovery {
    pub(in crate::panels::modules::onchain) fn recovery_actions_ready(&self) -> bool {
        self.loaded && self.read_problem.is_none() && self.recovery_problem.is_none()
            && self.pending_authorization.is_none() && self.pending_submission.is_none()
    }

    pub(in crate::panels::modules::onchain) fn can_reserve_recovery(&self, plan_id: &str, now_ms: i64) -> bool {
        self.recovery_actions_ready() && self.plans.iter().any(|plan|
            plan.plan_id == plan_id
                && plan.status == shared_types::OnchainCrossChainRecoveryPlanStatus::AwaitingAuthorization
                && plan.preview.quote_ready && plan.preview.blockers.is_empty()
                && plan.preview.valid_until_ms.is_some_and(|deadline| now_ms < deadline)
                && self.selected().is_some_and(|run| run.run_id == plan.preview.source_run_id
                    && run.updated_at_ms == plan.preview.source_run_updated_at_ms))
    }

    pub(in crate::panels::modules::onchain) fn can_cancel_recovery(&self, plan_id: &str, now_ms: i64) -> bool {
        self.recovery_actions_ready() && self.plans.iter().any(|plan|
            plan.plan_id == plan_id
                && matches!(plan.status, shared_types::OnchainCrossChainRecoveryPlanStatus::AwaitingAuthorization
                    | shared_types::OnchainCrossChainRecoveryPlanStatus::Reserved)
                && plan.preview.valid_until_ms.is_some_and(|deadline| now_ms < deadline)
                && self.selected().is_some_and(|run| run.run_id == plan.preview.source_run_id))
    }

    pub(in crate::panels::modules::onchain) fn accept_recovery_plan(&mut self, plan: shared_types::OnchainCrossChainRecoveryPlan) {
        if let Some(old) = self.plans.iter_mut().find(|old| old.plan_id == plan.plan_id) {
            if plan.updated_at_ms >= old.updated_at_ms { *old = plan; }
        } else {
            self.plans.push(plan);
        }
    }

    pub(in crate::panels::modules::onchain) fn selected(&self) -> Option<&OnchainCrossChainRun> {
        self.rows
            .iter()
            .find(|row| Some(&row.run_id) == self.selected_id.as_ref())
    }
    pub(in crate::panels::modules::onchain) fn can_build(&self, now_ms: i64) -> bool {
        self.loaded
            && self.recovery_problem.is_none()
            && self.read_problem.is_none()
            && self.pending_authorization.is_none()
            && self.pending_submission.is_none()
            && !self.rows.iter().any(|run| blocks_new_build(run, now_ms))
    }
    pub(in crate::panels::modules::onchain) fn can_authorize(
        &self,
        key: &str,
        now_ms: i64,
    ) -> bool {
        self.loaded
            && self.recovery_problem.is_none()
            && self.read_problem.is_none()
            && self.pending_submission.is_none()
            && self
                .pending_authorization
                .as_deref()
                .is_none_or(|pending| pending == key)
            && !self.rows.iter().any(|run| blocks_new_build(run, now_ms))
    }
    pub(in crate::panels::modules::onchain) fn can_submit(
        &self,
        request: &OnchainCrossChainSubmitRequest,
        now_ms: i64,
    ) -> bool {
        self.loaded
            && self.recovery_problem.is_none()
            && self.read_problem.is_none()
            && self.pending_submission.is_none()
            && self.pending_authorization.is_none()
            && self.selected().is_some_and(|run| {
                run.run_id == request.run_id
                    && next_submit_position(run, now_ms) == Some(request.expected_position)
            })
    }
    pub(super) fn begin_submission(&mut self, request: OnchainCrossChainSubmitRequest) {
        if let Some(run) = self.selected() {
            self.pending_submission = Some(PendingSubmission { request, previous_updated_at_ms: run.updated_at_ms });
        }
    }

    fn reconcile_pending(&mut self, run: &OnchainCrossChainRun) {
        if self.rows.iter().find(|old| old.run_id == run.run_id).is_some_and(|old| run.updated_at_ms < old.updated_at_ms) {
            return;
        }
        let recovered_authorization = self.pending_authorization.as_deref() == Some(&run.idempotency_key);
        if recovered_authorization { self.pending_authorization = None; }
        let recovered_submission = self.pending_submission.as_ref().is_some_and(|pending|
            pending.request.run_id == run.run_id && run.updated_at_ms >= pending.previous_updated_at_ms
                && run.legs.iter().any(|leg| leg.position == pending.request.expected_position
                    && leg.status != LegStatus::RequoteRequired));
        if recovered_submission { self.pending_submission = None; }
        if recovered_authorization || recovered_submission { self.problem = None; }
    }

    pub(in crate::panels::modules::onchain) fn needs_poll(&self) -> bool {
        !self.loaded
            || self.pending_authorization.is_some()
            || self.pending_submission.is_some()
            || self.rows.iter().any(|run| {
                matches!(
                    run.status,
                    RunStatus::AwaitingSourceFinality | RunStatus::AwaitingDestinationEvidence
                ) || run.status == RunStatus::Running && run.active_position.is_some()
                    || run.accounting_refresh_due(crate::panels::modules::timestamp::now_ms())
            })
    }
    pub(in crate::panels::modules::onchain) fn can_recheck(
        &self,
        request: &OnchainCrossChainRecheckRequest,
    ) -> bool {
        self.loaded
            && self.recovery_problem.is_none()
            && self.read_problem.is_none()
            && self.pending_submission.is_none()
            && self.pending_authorization.is_none()
            && self.selected().is_some_and(|run| {
                run.run_id == request.run_id
                    && next_recheck_position(run) == Some(request.expected_position)
            })
    }
    pub(in crate::panels::modules::onchain) fn accept_run(
        &mut self,
        run: OnchainCrossChainRun,
        select: bool,
    ) {
        self.reconcile_pending(&run);
        if select {
            self.selected_id = Some(run.run_id.clone());
        }
        if let Some(old) = self.rows.iter_mut().find(|old| old.run_id == run.run_id) {
            if run.updated_at_ms >= old.updated_at_ms {
                *old = run;
            }
        } else {
            self.rows.push(run);
        }
        self.rows
            .sort_by_key(|run| std::cmp::Reverse(run.updated_at_ms));
    }
    pub(in crate::panels::modules::onchain) fn accept_snapshot(
        &mut self,
        rows: Vec<OnchainCrossChainRun>,
        now_ms: i64,
    ) {
        for row in rows {
            let recovered = self.pending_authorization.as_deref() == Some(&row.idempotency_key);
            self.accept_run(row, recovered);
        }
        self.loaded = true;
        self.read_problem = None;
        if self.selected().is_none() {
            self.selected_id = self
                .rows
                .iter()
                .find(|run| blocks_new_build(run, now_ms))
                .or_else(|| self.rows.first())
                .map(|run| run.run_id.clone());
        }
    }
}

pub(in crate::panels::modules::onchain) fn next_submit_position(
    run: &OnchainCrossChainRun,
    now_ms: i64,
) -> Option<u8> {
    if run.active_position.is_some() {
        return None;
    }
    match run.status {
        RunStatus::AuthorizedAwaitingSubmit if now_ms < run.authorization.valid_until_ms => (),
        RunStatus::Running => (),
        _ => return None,
    }
    run.legs
        .iter()
        .find(|leg| leg.status != LegStatus::Completed)
        .filter(|leg| leg.status == LegStatus::RequoteRequired)
        .map(|leg| leg.position)
}

fn blocks_new_build(run: &OnchainCrossChainRun, now_ms: i64) -> bool {
    match run.status {
        RunStatus::AuthorizationExpired | RunStatus::Completed => false,
        RunStatus::AuthorizedAwaitingSubmit => now_ms < run.authorization.valid_until_ms,
        // A failed or paused cycle may still hold funds on the destination chain.
        _ => true,
    }
}

pub(in crate::panels::modules::onchain) fn next_recheck_position(
    run: &OnchainCrossChainRun,
) -> Option<u8> {
    if run.status != RunStatus::Paused {
        return None;
    }
    let position = run.active_position?;
    run.legs
        .iter()
        .find(|leg| {
            leg.position == position
                && leg.status == LegStatus::Paused
                && leg
                    .source_transaction_id
                    .as_deref()
                    .is_some_and(|hash| !hash.trim().is_empty())
        })
        .map(|leg| leg.position)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
