use super::*;
use crate::api::rest::{with_mutation_timeout, ApiError, MutationRequestContext};
use crate::panels::shared::operation_journal::{lookup_operation, validate_setting_response, PendingOperation};
use shared_types::{ActionRunKind, ActionRunStatus};

pub(super) fn callbacks(
    data: PeerPlanData,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    budget: RwSignal<String>,
    keyed: RwSignal<bool>,
) -> (Callback<(String, StockChainDirection)>, Callback<()>) {
    let journal = data.journal;
    let build = Callback::new(move |(wallet, direction): (String, StockChainDirection)| {
        if data.pending.get_untracked() || journal.locked() { return; }
        if let Some(problem) = market.with_untracked(|m| m.value().and_then(|s|
            super::super::super::super::readiness::quote_draft_problem(s, &budget.get_untracked(), keyed.get_untracked()))) {
            data.problem.set(Some(problem.into())); return;
        }
        let context = MutationRequestContext::new_idempotent_attempt("settings-stock_peer_plan_build");
        let Some(request) = market.with_untracked(|m| {
            let s = m.value()?;
            let c = s.comparison.as_ref()?;
            Some(StockPeerPlanRequest {
                request_id: context.request_id().into(), asset: c.asset.clone(),
                selection: s.peer.as_ref()?.selection.clone(), direction,
                wallet_address: wallet.trim().into(),
                input_raw: direction.quote(c)?.input_raw.clone(), keyed: c.keyed,
            })
        }) else { return; };
        if let Err(problem) = request.validate() { data.problem.set(Some(problem)); return; }
        let Some(attempt) = journal.begin_with_context(
            ActionRunKind::StockPeerPlanBuild, request.request_id.clone(), context,
        ) else { return; };
        let epoch = journal.epoch.get_untracked();
        let client = journal.client();
        data.problem.set(None);
        data.pending.set(true);
        spawn_local(async move {
            let result = client.build_stock_peer_plan(&request, &attempt.context).await.and_then(|receipt| {
                validate_setting_response(&attempt, &receipt)?;
                if receipt.request != request {
                    return Err(ApiError::client("STOCK_PEER_PLAN_RECEIPT_MISMATCH", "双边构建处理结果参数不匹配，请核对原请求"));
                }
                Ok(receipt)
            });
            if !journal.current(epoch) { return; }
            match result {
                Ok(receipt) => read_current(data, market, &attempt, &receipt, epoch).await,
                Err(error) => {
                    data.problem.try_set(Some(error.problem.message.clone()));
                    journal.failed(&attempt, &error);
                }
            }
            if journal.current(epoch) {
                journal.busy.set(false);
                data.pending.try_set(false);
            }
        });
    });
    let recheck = Callback::new(move |()| {
        if journal.busy.get_untracked() { return; }
        let Some(attempt) = journal.pending.get_untracked() else { journal.restore(); return; };
        let epoch = journal.epoch.get_untracked();
        if !journal.current(epoch) { return; }
        let client = journal.client();
        journal.busy.set(true);
        journal.problem.set(None);
        data.problem.set(None);
        spawn_local(async move {
            let result = with_mutation_timeout("核对原双边构建", lookup_operation(&client, &attempt)).await;
            if !journal.current(epoch) { return; }
            match result {
                Ok(run) if run.status == ActionRunStatus::Succeeded => {
                    let receipt = run.result.filter(|_| run.problem.is_none())
                        .and_then(|v| serde_json::from_value::<StockPeerPlanBuildReceipt>(v).ok())
                        .filter(|r| r.valid_for(&attempt.target));
                    if let Some(receipt) = receipt {
                        let mut known = attempt.clone();
                        known.run_id = Some(run.id.clone());
                        journal.remember_run(&attempt, run.id);
                        read_current(data, market, &known, &receipt, epoch).await;
                    } else {
                        journal.problem.set(Some("原双边构建处理结果不完整，保留待核对状态".into()));
                    }
                }
                Ok(run) if run.status == ActionRunStatus::Accepted => {
                    journal.remember_run(&attempt, run.id);
                    journal.problem.set(Some("原双边构建仍在处理，请稍后核对；未重新提交".into()));
                }
                Ok(run) => {
                    if journal.resolve(&attempt) {
                        data.problem.try_set(Some(format!("原双边构建未成功：{}", run.problem.map_or(run.message, |p| p.message))));
                    }
                }
                Err(error) => journal.problem.set(Some(error.problem.message)),
            }
            if journal.current(epoch) { journal.busy.set(false); }
        });
    });
    (build, recheck)
}

async fn read_current(
    data: PeerPlanData,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    attempt: &PendingOperation,
    receipt: &StockPeerPlanBuildReceipt,
    epoch: u64,
) {
    let journal = data.journal;
    let client = journal.client();
    let result = with_mutation_timeout("读取双边计划当前状态", client.stock_peer_plans()).await;
    if !journal.current(epoch) { return; }
    let matches = |s: &StockMarketSnapshot| s.peer_plan_problem.is_none()
        && s.peer_plans.iter().any(|p| p.plan_id == receipt.plan_id && p.request == receipt.request);
    match result {
        Ok(snapshot) => {
            let same_stock = market.with_untracked(|m| m.value().is_none_or(|s|
                s.security.as_ref().map(|s| &s.asset) == snapshot.security.as_ref().map(|s| &s.asset)));
            if !same_stock || !matches(&snapshot) {
                journal.problem.set(Some("已取得构建处理结果，但当前计划或股票记录未核齐；保留原请求，请再次核对".into()));
                return;
            }
            apply_snapshot(market, snapshot);
            // A newer WS may already contain cancellation or settlement. Never restore receipt.phase.
            if market.with_untracked(|m| m.value().is_some_and(matches)) {
                data.selected_plan.try_set(Some(receipt.plan_id.clone()));
                journal.resolve(attempt);
            } else {
                journal.problem.set(Some("最新快照尚未核到原双边计划，保留待核对状态".into()));
            }
        }
        Err(error) => journal.problem.set(Some(format!("原构建处理结果已取得，读取当前计划失败：{}", error.problem.message))),
    }
}
