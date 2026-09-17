use shared_types::stocks::{comparison::positive, *};

pub(super) fn quote_matches_draft(s: &StockMarketSnapshot, budget: &str, keyed: bool) -> bool {
    s.comparison.as_ref().is_some_and(|c| {
        s.security.as_ref().is_some_and(|v| v.asset == c.asset)
            && c.keyed == keyed
            && positive(budget.trim()).is_some_and(|n| Some(n) == positive(c.budget_usdc.trim()))
    })
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
            "已预检 · 未预留"
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
    if let Some(plan) = s.plans.iter().find(|p| p.holds_funds(now)) {
        return if plan.phase_at(now) == StockPlanPhase::SubmissionUnknown {
            (
                "提交待核对",
                "保留原计划与资金占用，请查看执行计划的双腿回执；不重复提交",
            )
        } else {
            (
                "已预留 · 未下单",
                "资金已预留，订单与链上交易均未提交",
            )
        };
    }
    if preflight_current(s, wallet, budget, keyed, now) {
        (
            "已预检 · 未预留",
            "显示本次库存与成本预算；构建时仍会复核，差额不是已实现收益",
        )
    } else if s.preflight.is_some() {
        (
            "预检需更新",
            "下方预检仅供查看历史；当前参数、行情或时效已变化，构建时会重新核对",
        )
    } else {
        (
            "尚未预检",
            "可先只读检查库存与成本；构建计划后才会预留资金，不会自动下单",
        )
    }
}
