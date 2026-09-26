use crate::state::AppState;
use shared_types::{review::settlements::*, stocks::*, *};

pub(crate) fn snapshot(state: &AppState, query: &SettlementReviewQuery) -> SettlementReviewSnapshot {
    let now = common::time::now_ms();
    let mut out = SettlementReviewSnapshot { observed_at_ms: now, ..Default::default() };
    let include = |source| query.source == SettlementSource::All || query.source == source;
    let id = query.record.as_deref();
    if include(SettlementSource::Onchain) {
        let mut runs = state.onchain_execution_runs().iter().filter(|r| id.is_none_or(|id| r.key() == id))
            .map(|r| r.value().clone()).collect::<Vec<_>>();
        runs.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then(a.run_id.cmp(&b.run_id)));
        out.rows.extend(runs.iter().take(101).map(onchain));
        out.problems.extend(state.onchain_execution_run_store().readiness().err().map(|p| format!("链上执行记录：{p}")));
    }
    if include(SettlementSource::CrossChain) {
        let runs = state.onchain_cross_chain_runs().review_runs(id, now);
        out.rows.extend(runs.rows.iter().map(cross_chain));
        out.problems.extend(runs.recovery_problem.map(|p| format!("跨链记录：{p}")));
    }
    if include(SettlementSource::Stocks) {
        let (plans, problem) = state.backpack_stocks().review_plans(id);
        out.rows.extend(plans.iter().map(|p| stock(p, now)));
        out.problems.extend(problem.map(|p| format!("Backpack 股票记录：{p}")));
    }
    if include(SettlementSource::StockPeer) {
        let (plans, problem) = state.backpack_stocks().review_peer_plans(id);
        out.rows.extend(plans.iter().map(|p| stock_peer(p, now)));
        out.problems.extend(problem.map(|p| format!("股票跨所记录：{p}")));
    }
    out.rows.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms)
        .then(a.source.slug().cmp(b.source.slug())).then(a.id.cmp(&b.id)));
    out.truncated = out.rows.len() > 100;
    out.rows.truncate(100);
    out
}

fn record(source: SettlementSource, id: &str, title: String, updated: i64, execution: &str) -> SettlementReviewRecord {
    SettlementReviewRecord { source, id: id.into(), title, updated_at_ms: updated,
        execution_state: execution.into(), accounting_state: "实际收支待核算".into(), attention: true,
        amounts: vec![], references: vec![], notes: vec![] }
}
fn amount(label: &str, asset: &str, value: Option<String>) -> SettlementReviewAmount {
    SettlementReviewAmount { label: label.into(), asset: asset.into(), amount: value }
}
fn reference(out: &mut SettlementReviewRecord, label: &str, id: Option<&str>) {
    if let Some(id) = id { out.references.push((label.into(), id.into())); }
}
fn accounting_label(status: OnchainExecutionAccountingStatus) -> &'static str {
    match status {
        OnchainExecutionAccountingStatus::PendingReceipts => "成交收支待核算",
        OnchainExecutionAccountingStatus::PendingValuation => "原币收支已核算 · 汇率待齐",
        OnchainExecutionAccountingStatus::Valued => "成交收支已核算 · 美元为折算值",
    }
}
fn stock_accounting_label(status: StockAccountingStatus) -> &'static str {
    match status {
        StockAccountingStatus::AwaitingReceipts => "双腿收支待核对",
        StockAccountingStatus::NeedsReview => "双腿存在待处置缺口",
        StockAccountingStatus::LegsReconciled => "两腿原币收支已核对",
    }
}

pub(crate) fn onchain(run: &OnchainExecutionSubmitResponse) -> SettlementReviewRecord {
    let execution = match run.status {
        OnchainExecutionRunStatus::Executing => "执行中",
        OnchainExecutionRunStatus::Completed => "全部腿已完成",
        OnchainExecutionRunStatus::AwaitingChainFinality => "等待链上终态",
        OnchainExecutionRunStatus::FinalityUnresolved => "执行终态待核验",
        OnchainExecutionRunStatus::Compensated => "已回滚 · 仍需核算费用",
        OnchainExecutionRunStatus::Failed => "执行失败",
        OnchainExecutionRunStatus::Exposed => "存在未对冲暴露",
    };
    let title = run.legs.iter().map(|leg| format!("{} {}", leg.venue, leg.symbol.as_deref().unwrap_or("链上")))
        .collect::<Vec<_>>().join(" / ");
    let mut out = record(SettlementSource::Onchain, &run.run_id, title, run.updated_at_ms, execution);
    if let Some(a) = &run.accounting {
        out.accounting_state = accounting_label(a.status).into();
        out.attention = run.status != OnchainExecutionRunStatus::Completed || !run.quantity_reconciled
            || a.status != OnchainExecutionAccountingStatus::Valued || !a.problems.is_empty();
        out.amounts.extend(a.net_assets.iter().map(|v| amount("原币净变化", &v.asset, Some(v.amount_exact.clone()))));
        out.amounts.push(amount("成交净变动折算", "USD", a.usd_value.as_ref()
            .filter(|_| a.status == OnchainExecutionAccountingStatus::Valued).map(|v| v.net_usd_exact.clone())));
        out.notes.extend(a.problems.clone());
    }
    out.notes.push(format!("仅包含本次执行及已归集的 {} 笔补库、{} 笔授权费用；不等于全部已实现利润。", run.replenishment_costs.len(), run.approval_costs.len()));
    if !run.quantity_reconciled { out.notes.push("对冲数量仍待核齐".into()); }
    out.notes.extend(run.problem.clone());
    reference(&mut out, "构建编号", Some(&run.build_id));
    for leg in &run.legs {
        let label = format!("步骤 {} · {} · {:?}", leg.position, leg.venue, leg.status);
        reference(&mut out, &label, leg.order_id.as_deref());
        reference(&mut out, &label, leg.transaction_id.as_deref());
    }
    out
}

fn cross_chain(run: &OnchainCrossChainRun) -> SettlementReviewRecord {
    let execution = match run.status {
        OnchainCrossChainRunStatus::AuthorizedAwaitingSubmit => "已授权 · 未提交",
        OnchainCrossChainRunStatus::AuthorizationExpired => "授权已过期",
        OnchainCrossChainRunStatus::Running => "执行中",
        OnchainCrossChainRunStatus::AwaitingSourceFinality => "等待源链确认",
        OnchainCrossChainRunStatus::AwaitingDestinationEvidence => "等待目标链到账",
        OnchainCrossChainRunStatus::Paused => "已暂停 · 原交易保留",
        OnchainCrossChainRunStatus::Compensating => "处置中",
        OnchainCrossChainRunStatus::Completed => "四步已完成",
        OnchainCrossChainRunStatus::Failed => "执行失败",
    };
    let mut out = record(SettlementSource::CrossChain, &run.run_id,
        format!("{} / {}", run.build.source_chain, run.build.peer_chain), run.updated_at_ms, execution);
    if let Some(a) = &run.accounting {
        out.accounting_state = accounting_label(a.status).into();
        out.attention = run.status != OnchainCrossChainRunStatus::Completed
            || a.status != OnchainExecutionAccountingStatus::Valued || !a.problems.is_empty();
        out.amounts.extend(a.net_assets.iter().map(|v| amount(&format!("{} · {} · {}", v.chain, v.wallet, v.asset.address),
            &v.asset.symbol, Some(v.amount_exact.clone()))));
        out.amounts.push(amount("成交净变动折算", "USD", a.usd_value.as_ref()
            .filter(|_| a.status == OnchainExecutionAccountingStatus::Valued).map(|v| v.net_usd_exact.clone())));
        out.notes.extend(a.problems.clone());
    }
    out.notes.extend(run.problem.clone());
    out.notes.push(run.next_action.clone());
    out.notes.push("源链确认不等于目标链到账；原币按链、钱包及合约分开，美元仅为已记录汇率的折算。".into());
    reference(&mut out, "构建编号", Some(&run.build.build_id));
    for leg in &run.legs {
        reference(&mut out, &format!("步骤 {} 源链", leg.position), leg.source_transaction_id.as_deref());
        reference(&mut out, &format!("步骤 {} 目标链", leg.position), leg.destination_transaction_id.as_deref());
    }
    out
}

pub(crate) fn stock(plan: &StockExecutionPlan, now: i64) -> SettlementReviewRecord {
    let execution = match plan.phase_at(now) {
        StockPlanPhase::Reserved => "已预留 · 未下单", StockPlanPhase::Cancelled => "已取消预留",
        StockPlanPhase::Expired => "预留已到期", StockPlanPhase::SubmissionUnknown => "提交后待核对 · 保留占用",
        StockPlanPhase::Settled => "交易已收尾 · 预留已释放",
    };
    let mut out = record(SettlementSource::Stocks, &plan.plan_id,
        format!("{} · {}", plan.request.asset, plan.request.direction.label()), plan.updated_at_ms, execution);
    let a = plan.accounting();
    out.accounting_state = stock_accounting_label(a.status).into();
    out.attention = plan.phase != StockPlanPhase::Settled || a.status != StockAccountingStatus::LegsReconciled || !a.problems.is_empty();
    out.amounts = vec![amount("USDC 净变化", "USDC", a.net_usdc_change),
        amount("股票库存合计变化", "股", a.net_stock_shares), amount("钱包 SOL 净变化", "SOL", a.net_sol_change)];
    if a.conversion_fee_usdc.is_some() {
        out.amounts.push(amount("已归集换币手续费", "USDC", a.conversion_fee_usdc));
        out.amounts.push(amount("扣除所选换币费后差额", "USDC", a.after_conversion_costs_usdc));
    }
    out.notes = a.problems;
    out.notes.push("原币收支包含已记录补偿与补回；未归集费用、换币价差及后续库存再平衡不在该差额内。".into());
    reference(&mut out, "CEX 订单", plan.cex_order.as_ref().and_then(|o| o.order_id.as_deref()));
    reference(&mut out, "RFQ", plan.rfq_acceptance.as_ref().and_then(|r| r.rfq_id.as_deref()));
    reference(&mut out, "链上交易", plan.chain_submission.as_ref().and_then(|s| s.transaction_id.as_deref()));
    out
}

fn stock_peer(plan: &StockPeerPlan, now: i64) -> SettlementReviewRecord {
    let execution = match plan.phase {
        StockPeerPlanPhase::Reserved if now >= plan.terms.reserved_until_ms => "预留已到期",
        StockPeerPlanPhase::Reserved => "已预留 · 未下单", StockPeerPlanPhase::Cancelled => "已取消预留",
        StockPeerPlanPhase::SubmissionUnknown => "提交后待核对 · 保留占用", StockPeerPlanPhase::Settled => "交易已收尾 · 预留已释放",
    };
    let mut out = record(SettlementSource::StockPeer, &plan.plan_id,
        format!("{} · {}", plan.request.asset, plan.request.direction.label()), plan.updated_at_ms, execution);
    let a = plan.accounting();
    out.accounting_state = stock_accounting_label(a.status).into();
    out.attention = plan.phase != StockPeerPlanPhase::Settled || a.status != StockAccountingStatus::LegsReconciled || !a.problems.is_empty();
    out.amounts.extend(a.cash_totals.into_iter().map(|(asset, v)| amount("已知原币现金变化", &asset, Some(v))));
    out.amounts.extend([amount("CEX 股票变化", "股", a.cex_stock_shares),
        amount("链上股票变化", "股", a.chain_stock_shares), amount("钱包 SOL 净变化", "SOL", a.wallet_sol_change)]);
    out.notes = a.problems;
    out.notes.extend(a.remaining);
    out.notes.extend(plan.execution_problem.clone());
    out.notes.push("不同发行方股票不互相抵消；不同币种现金不直接相加，缺失回执不记零。".into());
    reference(&mut out, "CEX 订单", plan.cex_order.as_ref().and_then(|o| o.order_id.as_deref()));
    reference(&mut out, "链上交易", plan.chain_submission.as_ref().and_then(|s| s.transaction_id.as_deref()));
    out
}
