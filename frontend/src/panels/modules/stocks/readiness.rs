use shared_types::stocks::{comparison::positive, *};

pub(super) fn quote_matches_draft(s: &StockMarketSnapshot, budget: &str, keyed: bool) -> bool {
    s.comparison.as_ref().is_some_and(|c| {
        s.security.as_ref().is_some_and(|v| v.asset == c.asset)
            && c.keyed == keyed
            && positive(budget.trim()).is_some_and(|n| Some(n) == positive(c.budget_usdc.trim()))
    })
}

pub(super) fn quote_draft_problem(s: &StockMarketSnapshot, budget: &str, keyed: bool) -> Option<&'static str> {
    if s.comparison.is_none() {
        Some("尚无链上报价，请先更新询价")
    } else if !quote_matches_draft(s, budget, keyed) {
        Some("金额、股票或 Jupiter 接入已变化，请先更新询价")
    } else {
        None
    }
}

pub(super) fn quote_summary(s: &StockMarketSnapshot, budget: &str, keyed: bool, now: i64) -> &'static str {
    let Some(c) = s.comparison.as_ref() else { return "等待询价"; };
    if !quote_matches_draft(s, budget, keyed) { return "参数已更改"; }
    if identity::backpack_token_identity(s).is_err() { return "合约映射待核实"; }
    if now < c.mint.checked_at_ms || now - c.mint.checked_at_ms > 60_000
        || c.mint.next_change_at_ms.is_some_and(|at| now >= at) {
        return "股数资料已过期";
    }
    match (comparison::quote_current(&c.buy, now), c.sell.as_ref().is_some_and(|q| comparison::quote_current(q, now))) {
        (true, true) => "双向报价 · 仅观察",
        (true, false) => "仅链买报价",
        (false, true) => "仅链卖报价",
        (false, false) => "报价已过期",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_plan_readiness_follows_draft_and_saved_state() {
        let mut s: StockMarketSnapshot = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../shared-types/fixtures/stocks_plan_build.json"
        )))
        .unwrap();
        let mut plan = s.plans.remove(0);
        let now = s.observed_at_ms;
        let wallet = plan.request.wallet_address.clone();
        for amount in ["10", "10.00", " 10.0 "] {
            assert!(preflight_current(&s, &wallet, amount, false, now));
            assert_eq!(
                build_block_reason(&s, &wallet, amount, false, StockChainDirection::Buy, now),
                None
            );
        }
        for amount in ["11", "0", "-1", "", "not-an-amount"] {
            assert!(!preflight_current(&s, &wallet, amount, false, now));
            assert!(
                build_block_reason(&s, &wallet, amount, false, StockChainDirection::Buy, now)
                    .unwrap()
                    .contains("更新询价")
            );
        }
        assert!(!preflight_current(&s, &wallet, "10", true, now));
        assert!(!preflight_current(&s, "another-wallet", "10", false, now));
        assert!(
            build_block_reason(&s, "", "10", false, StockChainDirection::Buy, now)
                .unwrap()
                .contains("钱包")
        );
        assert!(!preflight_current(&s, &wallet, "10", false, now + 10_000));
        // Building refreshes stale market evidence itself; it is not execution permission.
        assert_eq!(
            build_block_reason(
                &s,
                &wallet,
                "10",
                false,
                StockChainDirection::Buy,
                now + 10_000
            ),
            None
        );
        s.plan_problem = Some("journal damaged".into());
        assert!(
            build_block_reason(&s, &wallet, "10", false, StockChainDirection::Buy, now)
                .unwrap()
                .contains("资金记录")
        );
        s.plan_problem = None;
        s.plans.push(plan.clone());
        assert!(
            build_block_reason(&s, &wallet, "10", false, StockChainDirection::Buy, now)
                .unwrap()
                .contains("占用资金")
        );
        assert_eq!(
            module_status(&s, &wallet, "10", false, now).0,
            "已预留 · 未下单"
        );
        plan.phase = StockPlanPhase::SubmissionUnknown;
        s.plans[0] = plan.clone();
        assert_eq!(module_status(&s, &wallet, "10", false, now).0, "提交待核对");
        plan.phase = StockPlanPhase::Cancelled;
        s.plans[0] = plan;
        assert_eq!(
            build_block_reason(&s, &wallet, "10", false, StockChainDirection::Buy, now),
            None
        );
        assert_eq!(
            module_status(&s, &wallet, "10", false, now).0,
            "已完成交易检查 · 未预留"
        );
        s.comparison.as_mut().unwrap().sell = None;
        assert!(
            build_block_reason(&s, &wallet, "10", false, StockChainDirection::Sell, now)
                .unwrap()
                .contains("尚无链上报价")
        );
    }
}

pub(super) fn preflight_current(
    s: &StockMarketSnapshot,
    wallet: &str,
    budget: &str,
    keyed: bool,
    now: i64,
) -> bool {
    quote_matches_draft(s, budget, keyed)
        && s.preflight.as_ref().is_some_and(|p| {
            p.current(s, now) && p.wallet_address.as_deref().unwrap_or("") == wallet.trim()
        })
}

// This is permission to ask the backend to build, never permission to place orders.
pub(super) fn build_block_reason(
    s: &StockMarketSnapshot,
    wallet: &str,
    budget: &str,
    keyed: bool,
    direction: StockChainDirection,
    now: i64,
) -> Option<&'static str> {
    let wallet = wallet.trim();
    if wallet.is_empty() {
        return Some("先填写 Solana 钱包地址");
    }
    if s.plan_problem.is_some()
        || s.funding_problem.is_some()
        || s.stablecoin_problem.is_some()
        || s.exchange_conversion_problem.is_some()
    {
        return Some("资金记录存在问题，请先查看下方计划记录");
    }
    if s.plans.iter().any(|p| p.holds_funds(now)) {
        return Some("已有股票计划占用资金，请先核对或取消原计划");
    }
    if s.funding_plans
        .iter()
        .any(|p| p.phase_at(now).holds_funds())
    {
        return Some("补库计划仍占用资金，请先核对或取消补库");
    }
    if s.exchange_conversions.iter().any(|p| p.holds_funds(now)) {
        return Some("Backpack 账户兑换仍占用资金，请先处理原兑换");
    }
    if s.stablecoin_plans
        .iter()
        .any(|p| p.request.conversion.wallet_address == wallet && p.holds_funds(now))
    {
        return Some("这个钱包的稳定币兑换尚未结束，请先处理原兑换");
    }
    let Some(c) = s.comparison.as_ref() else {
        return Some("先更新链上询价");
    };
    if !quote_matches_draft(s, budget, keyed) {
        return Some("金额、股票或 Jupiter 接入已变化，请先更新询价");
    }
    if direction.quote(c).is_none() {
        return Some("该方向尚无链上报价，请检查金额与数量限制");
    }
    None
}

pub(super) fn module_status(
    s: &StockMarketSnapshot,
    wallet: &str,
    budget: &str,
    keyed: bool,
    now: i64,
) -> (&'static str, &'static str) {
    if let Some(status)=funds_status(s,now) { return status; }
    if preflight_current(s, wallet, budget, keyed, now) {
        (
            "已完成交易检查 · 未预留",
            "显示本次库存与成本预算；构建时仍会复核，差额不是已实现收益",
        )
    } else if s.preflight.is_some() {
        (
            "交易检查需更新",
            "下方交易检查仅供查看历史；当前参数、行情或时效已变化，构建时会重新核对",
        )
    } else {
        (
            "尚未检查交易",
            "可先只读检查库存与成本；构建计划后才会预留资金，不会自动下单",
        )
    }
}

pub(super) fn funds_status(s: &StockMarketSnapshot, now: i64) -> Option<(&'static str, &'static str)> {
    if s.plan_problem.is_some()
        || s.peer_plan_problem.is_some()
        || s.funding_problem.is_some()
        || s.stablecoin_problem.is_some()
        || s.exchange_conversion_problem.is_some()
    {
        return Some((
            "资金记录待核对",
            "本模块资金记录存在问题，请查看库存与成本、执行记录；不能按没有占用处理",
        ));
    }
    // Submitted plans take priority regardless of the journal's row order or selected stock.
    if s.plans.iter().any(|p| p.phase_at(now) == StockPlanPhase::SubmissionUnknown)
        || s.peer_plans.iter().any(|p| p.phase == StockPeerPlanPhase::SubmissionUnknown)
    {
        return Some((
            "提交待核对",
            "本模块有尚未结清的股票交易，资金占用保留；请查看执行记录中的原双腿处理结果，不重复提交",
        ));
    }
    if s.funding_plans.iter().any(|p| {
        let phase = p.phase_at(now);
        phase.holds_funds() && phase != StockFundingPlanPhase::Reserved
    }) || s.exchange_conversions.iter().any(|p| p.order.is_some() && p.holds_funds(now))
        || s.stablecoin_plans.iter().any(|p| {
            p.holds_funds(now) && p.phase_at(now) != StockStablecoinPlanPhase::Reserved
        })
    {
        return Some((
            "资金待核对",
            "本模块有补库或兑换尚未结清，资金占用保留；请查看库存与成本、执行记录",
        ));
    }
    if s.plans.iter().any(|p| p.holds_funds(now))
        || s.peer_plans.iter().any(|p| p.holds_funds(now))
    {
        return Some((
            "已预留 · 未下单",
            "本模块股票计划已预留资金，尚未提交交易；可到执行记录继续或取消预留",
        ));
    }
    if s.funding_plans.iter().any(|p| p.phase_at(now).holds_funds())
        || s.exchange_conversions.iter().any(|p| p.holds_funds(now))
        || s.stablecoin_plans.iter().any(|p| p.holds_funds(now))
    {
        return Some((
            "资金已预留",
            "本模块补库或兑换已预留资金；请查看库存与成本、执行记录，不代表股票订单已提交",
        ));
    }
    None
}
