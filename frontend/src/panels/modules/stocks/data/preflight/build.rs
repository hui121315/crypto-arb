use super::*;
use crate::api::rest::{ApiError, MutationRequestContext, with_mutation_timeout};
use crate::panels::shared::operation_journal::validate_setting_response;
use shared_types::{ActionRunKind, ActionRunStatus};

pub(super) struct BuildInput {
    pub journal: OperationJournal,
    pub market: RwSignal<LoadState<StockMarketSnapshot>>,
    pub notice: Notice,
    pub wallet: RwSignal<String>,
    pub budget: RwSignal<String>,
    pub keyed: RwSignal<bool>,
    pub costs: RwSignal<Vec<String>>,
    pub section: RwSignal<u8>,
    pub selection: RwSignal<u64>,
    pub pending: RwSignal<bool>,
    pub selecting: RwSignal<bool>,
    pub quoting: RwSignal<bool>,
    pub monitoring: RwSignal<bool>,
    pub selected_plan: RwSignal<Option<String>>,
}

pub(super) fn callbacks(
    input: BuildInput,
) -> (Callback<(String, StockChainDirection)>, Callback<()>) {
    let BuildInput {
        journal,
        market,
        notice,
        wallet,
        budget,
        keyed,
        costs,
        section,
        selection,
        pending,
        selecting,
        quoting,
        monitoring,
        selected_plan,
    } = input;
    let recheck = journal.recheck(Callback::new(move |run: shared_types::ActionRun| {
        if run.status == ActionRunStatus::Succeeded {
            notice.inform("原构建请求已核对；当前状态以执行记录为准，未重复预留");
        } else {
            notice.set(Some(format!(
                "原构建未成功：{}",
                run.problem.map_or(run.message, |p| p.message)
            )));
        }
        spawn_local(async move {
            read_current(journal, market).await;
        });
    }));
    let build = Callback::new(move |(asset, direction): (String, StockChainDirection)| {
        if pending.get_untracked()
            || journal.locked()
            || selecting.get_untracked()
            || quoting.get_untracked()
            || monitoring.get_untracked()
        {
            return;
        }
        let problem = market.with_untracked(|m| {
            m.value()
                .map(|s| {
                    super::super::super::readiness::build_block_reason(
                        s,
                        &wallet.get_untracked(),
                        &budget.get_untracked(),
                        keyed.get_untracked(),
                        direction,
                        super::super::super::super::timestamp::now_ms(),
                    )
                })
                .unwrap_or(Some("股票行情尚未就绪"))
        });
        if let Some(problem) = problem {
            notice.set(Some(problem.into()));
            return;
        }
        let Some(comparison) =
            market.with_untracked(|m| m.value().and_then(|s| s.comparison.clone()))
        else {
            return;
        };
        if comparison.asset != asset {
            return;
        }
        let Some(quote) = direction.quote(&comparison) else {
            return;
        };
        let context = MutationRequestContext::new_idempotent_attempt("settings-stock_plan_build");
        let request = StockPlanBuildRequest {
            request_id: context.request_id().into(),
            asset,
            direction,
            wallet_address: wallet.get_untracked().trim().into(),
            input_raw: quote.input_raw.clone(),
            keyed: comparison.keyed,
            conversion_cost_ids: costs.get_untracked(),
        };
        let Some(attempt) = journal.begin_with_context(
            ActionRunKind::StockPlanBuild,
            request.request_id.clone(),
            context,
        ) else {
            return;
        };
        let epoch = journal.epoch.get_untracked();
        let selected = selection.get_untracked();
        let client = journal.client();
        notice.set(None);
        spawn_local(async move {
            let result = client
                .build_stock_plan(&request, &attempt.context)
                .await
                .and_then(|receipt| {
                    validate_setting_response(&attempt, &receipt)?;
                    if receipt.request != request {
                        return Err(ApiError::client(
                            "STOCK_PLAN_RECEIPT_MISMATCH",
                            "构建处理结果参数不匹配，请核对原请求",
                        ));
                    }
                    Ok(receipt)
                });
            if !journal.current(epoch) {
                return;
            }
            match result {
                Ok(receipt) => {
                    journal.resolve(&attempt);
                    let current = read_current(journal, market).await;
                    if !journal.current(epoch) {
                        return;
                    }
                    if selection.try_get_untracked() == Some(selected) {
                        costs.try_set(vec![]);
                    }
                    let plan = current.as_ref().and_then(|s| {
                        s.plans.iter().find(|p| {
                            p.plan_id == receipt.plan_id
                                && p.request.build.as_ref() == Some(&request)
                        })
                    });
                    let message = plan
                        .map(|p| {
                            match p.phase_at(super::super::super::super::timestamp::now_ms()) {
                                StockPlanPhase::Reserved => "计划已保存并预留，尚未下单",
                                StockPlanPhase::Cancelled => "原计划已取消，没有重新预留",
                                StockPlanPhase::Expired => "原计划预留已到期，没有重新预留",
                                StockPlanPhase::SubmissionUnknown => {
                                    "原计划已提交，请继续核对原处理结果"
                                }
                                StockPlanPhase::Settled => "原计划已收尾，没有重新预留",
                            }
                        })
                        .unwrap_or("构建处理结果已保存，计划当前状态未读到；请刷新执行记录");
                    notice.inform(message);
                    if plan.is_some()
                        && selection.try_get_untracked() == Some(selected)
                        && section.try_get_untracked() == Some(1)
                        && current
                            .as_ref()
                            .and_then(|s| s.security.as_ref())
                            .is_some_and(|s| s.asset == request.asset)
                    {
                        selected_plan.try_set(Some(receipt.plan_id.clone()));
                        section.try_set(3);
                        #[cfg(target_arch = "wasm32")]
                        request_animation_frame(move || {
                            if let Some(el) =
                                web_sys::window().and_then(|w| w.document()).and_then(|d| {
                                    d.get_element_by_id(&format!("stock-plan-{}", receipt.plan_id))
                                })
                            {
                                el.scroll_into_view_with_bool(true);
                            }
                        });
                    }
                }
                Err(error) => {
                    notice.try_set(Some(error.problem.message.clone()));
                    journal.failed(&attempt, &error);
                    journal.busy.set(false);
                }
            }
        });
    });
    (build, recheck)
}

async fn read_current(
    journal: OperationJournal,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
) -> Option<StockMarketSnapshot> {
    let epoch = journal.epoch.get_untracked();
    journal.busy.set(true);
    let client = journal.client();
    let result =
        with_mutation_timeout("读取股票计划当前状态", client.stock_market_snapshot()).await;
    if !journal.current(epoch) {
        return None;
    }
    journal.busy.set(false);
    match result {
        Ok(snapshot) => {
            apply_snapshot(market, snapshot);
            market.try_with_untracked(|m| m.value().cloned()).flatten()
        }
        Err(error) => {
            market.try_update(|m| m.apply_result(Err(error.problem)));
            None
        }
    }
}
