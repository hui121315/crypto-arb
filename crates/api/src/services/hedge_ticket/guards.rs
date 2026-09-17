use super::*;

pub(super) struct GuardInputs<'a> {
    pub(super) opp: &'a ArbitrageOpportunityDto,
    pub(super) long: &'a HedgeLegQuote,
    pub(super) short: &'a HedgeLegQuote,
    pub(super) long_target_notional: f64,
    pub(super) short_target_notional: f64,
    pub(super) blockers: &'a [String],
    pub(super) fees_required: bool,
    pub(super) fee_snapshot_count: usize,
    pub(super) cost: Option<&'a ExecutionCostProfile>,
    pub(super) profit_proof: &'a arbitrage::profit_proof::StrategyProfitProof,
}

pub(super) fn guards(input: &GuardInputs<'_>) -> Vec<ExecutionGuard> {
    vec![
        guard(
            "execution_eligible",
            "机会可执行",
            input.opp.execution_eligible,
            "机会被策略层标记为不可执行",
        ),
        guard(
            "complete_cost",
            "完整成本证据",
            input.cost.is_some(),
            "缺少双腿 WS 深度或完整开平仓费用，无法核验单次净利",
        ),
        guard(
            "positive_edge",
            "当前策略收益为正",
            input.cost.is_some_and(|cost| cost.gross_edge_bps > 0.0),
            "当前策略收益非正，不能构建对冲",
        ),
        profit_lock_guard(input.profit_proof),
        depth_guard(
            "long_depth",
            "多腿深度",
            input.long,
            input.long_target_notional,
        ),
        depth_guard(
            "short_depth",
            "空腿深度",
            input.short,
            input.short_target_notional,
        ),
        guard(
            "fee_snapshot",
            "标准费率",
            !input.fees_required || input.fee_snapshot_count >= 2,
            "缺少标准费率快照，实盘下单前需重新验证费用",
        ),
        guard(
            "no_blockers",
            "无硬阻断",
            input.blockers.is_empty(),
            "票据存在硬阻断",
        ),
    ]
}

fn profit_lock_guard(proof: &arbitrage::profit_proof::StrategyProfitProof) -> ExecutionGuard {
    ExecutionGuard {
        key: "profit_lock".to_owned(),
        label: "策略收益证明".to_owned(),
        passed: proof.passed,
        detail: format!("class={}; {}", proof.class.label(), proof.detail),
        preflight_outcome: None,
    }
}

pub(super) fn depth_guard(
    key: &'static str,
    label: &'static str,
    leg: &HedgeLegQuote,
    target_notional: f64,
) -> ExecutionGuard {
    let Some(depth) = executable_depth(leg) else {
        let detail = leg.blockers.first().cloned().unwrap_or_else(|| {
            format!("{} {} 深度暂不可用，等待盘口刷新", leg.exchange, leg.symbol)
        });
        return guard(key, label, false, &detail);
    };
    guard(
        key,
        label,
        depth + f64::EPSILON >= target_notional,
        &format!(
            "{} {} {}0.05% 滑点带内可成交深度 ${:.0} 低于目标 ${:.0}",
            leg.exchange,
            leg.symbol,
            order_side_label(leg.side),
            depth,
            target_notional
        ),
    )
}

pub(super) fn guard(key: &str, label: &str, passed: bool, detail: &str) -> ExecutionGuard {
    ExecutionGuard {
        key: key.to_owned(),
        label: label.to_owned(),
        passed,
        detail: if passed { "通过" } else { detail }.to_owned(),
        preflight_outcome: None,
    }
}

pub(super) fn append_failed_guard_blockers(blockers: &mut Vec<String>, guards: &[ExecutionGuard]) {
    blockers.extend(
        guards
            .iter()
            .filter(|guard| !guard.passed && guard.key != "no_blockers")
            .map(|guard| guard.detail.clone()),
    );
}

pub(super) fn refresh_submit_market_guards(
    ticket: &mut HedgeTicket,
    profit_proof: &arbitrage::profit_proof::StrategyProfitProof,
) {
    let quantity = ticket.sizing.target_base_quantity;
    let long_target = quantity
        .and_then(|quantity| {
            ticket
                .long_leg
                .open_vwap_price
                .map(|price| quantity * price)
        })
        .unwrap_or(ticket.sizing.long_notional_cap_usd);
    let short_target = quantity
        .and_then(|quantity| {
            ticket
                .short_leg
                .open_vwap_price
                .map(|price| quantity * price)
        })
        .unwrap_or(ticket.sizing.short_notional_cap_usd);
    let market_guards = [
        guard(
            "complete_cost",
            "完整成本证据",
            ticket.cost.is_some(),
            "缺少双腿 WS 深度或完整开平仓费用，无法核验单次净利",
        ),
        guard(
            "positive_edge",
            "当前策略收益为正",
            ticket
                .cost
                .as_ref()
                .is_some_and(|cost| cost.gross_edge_bps > 0.0),
            "当前策略收益非正，不能构建对冲",
        ),
        profit_lock_guard(profit_proof),
        depth_guard("long_depth", "多腿深度", &ticket.long_leg, long_target),
        depth_guard("short_depth", "空腿深度", &ticket.short_leg, short_target),
    ];
    for guard in market_guards {
        replace_guard(&mut ticket.guards, guard);
    }
}

fn replace_guard(guards: &mut Vec<ExecutionGuard>, guard: ExecutionGuard) {
    if let Some(existing) = guards.iter_mut().find(|existing| existing.key == guard.key) {
        *existing = guard;
    } else {
        guards.push(guard);
    }
}
