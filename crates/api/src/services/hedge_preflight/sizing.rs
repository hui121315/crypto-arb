use super::*;

/// 单腿下单规格/尺寸检查：`venue_covered` 标记该 venue 是否有 instrument feed；
/// `outcome` 为注册表 [`crate::services::instrument_registry::InstrumentRegistry::plan_leg_sizing`]
/// 的 fail-closed 结果。
pub(crate) struct InstrumentSizingCheck<'a> {
    pub(crate) plan: &'a OrderCompilePlan,
    pub(crate) venue_covered: bool,
    pub(crate) outcome: Result<ExecutionSizingPlan, SizingBlock>,
}

impl<'a> InstrumentSizingCheck<'a> {
    pub(crate) fn new(
        plan: &'a OrderCompilePlan,
        venue_covered: bool,
        outcome: Result<ExecutionSizingPlan, SizingBlock>,
    ) -> Self {
        Self {
            plan,
            venue_covered,
            outcome,
        }
    }
}

/// 实盘下单规格/尺寸 fail-closed 闸门。
///
/// 仅在 Live 模式生效。对**有 instrument feed 的 venue**（`venue_covered`），缺
/// 官方核验规格 / 不足步长 / 低于 `min_notional` 一律阻断；对尚未接线 instrument
/// 的 venue 不挂闸门（仍由下单能力/运行态证据闸门把关），避免误伤多 venue 流程。
pub(crate) fn instrument_sizing_guard(
    mode: ExecutionMode,
    checks: &[InstrumentSizingCheck<'_>],
) -> Option<ExecutionGuard> {
    if mode != ExecutionMode::Live || checks.is_empty() {
        return None;
    }
    if checks.iter().all(|check| !check.venue_covered) {
        return None;
    }
    let blockers = instrument_sizing_blockers(checks);
    let passed = blockers.is_empty();
    let detail = if passed {
        "通过".to_owned()
    } else {
        format!("下单尺寸阻断: {}", blockers.join("; "))
    };
    Some(ExecutionGuard {
        key: "instrument_sizing".to_owned(),
        label: "下单规格与尺寸".to_owned(),
        passed,
        detail: detail.clone(),
        preflight_outcome: Some(MarginPreflightOutcome {
            status: if passed {
                HedgePreflightStatus::Passed
            } else {
                HedgePreflightStatus::Blocked
            },
            checked_at_ms: common::time::now_ms(),
            scope: instrument_sizing_scope(checks),
            observed_venues: observed_instrument_sizing_venues(checks),
            balance_rows: Vec::new(),
            source: Some(
                "instrument_registry.exchange_info+execution_sizing.plan_leg_sizing".to_owned(),
            ),
            freshness_ms: None,
            retry_after_ms: None,
            request_id: None,
            problems: Vec::new(),
            field_quality: Vec::new(),
            row_health: Vec::new(),
            error: (!passed).then_some(detail),
        }),
    })
}

pub(super) fn instrument_sizing_blockers(checks: &[InstrumentSizingCheck<'_>]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| check.venue_covered)
        .filter_map(|check| {
            check.outcome.as_ref().err().map(|block| {
                format!(
                    "{} {} {}",
                    check.plan.exchange,
                    check.plan.symbol,
                    sizing_block_reason(block)
                )
            })
        })
        .fold(Vec::new(), push_unique)
}

pub(super) fn sizing_block_reason(block: &SizingBlock) -> &'static str {
    match block {
        SizingBlock::NonFiniteRequest => "目标名义/参考价非有限或非正",
        SizingBlock::SpecMissing => "缺官方核验下单规格（不可 fail-open）",
        SizingBlock::BelowOneStep => "对齐步长后不足一张",
        SizingBlock::BelowMinQty => "对齐后张数低于交易所 minSz/min_qty",
        SizingBlock::BelowMinNotional => "对齐后名义低于交易所 min_notional",
        SizingBlock::PairQuantityMismatch => "双腿官方数量步长无法对齐同一基础资产数量",
    }
}

pub(super) fn instrument_sizing_scope(checks: &[InstrumentSizingCheck<'_>]) -> HedgePreflightScope {
    HedgePreflightScope {
        venues: checks
            .iter()
            .filter(|check| check.venue_covered)
            .map(|check| normalized_venue_name(&check.plan.exchange))
            .filter(|venue| !venue.is_empty())
            .fold(Vec::new(), push_unique),
        symbols: checks
            .iter()
            .filter(|check| check.venue_covered)
            .map(|check| check.plan.symbol.clone())
            .fold(Vec::new(), push_unique),
        account_modes: Vec::new(),
        operations: vec![HedgePreflightOperation::OrderWrite],
    }
}

pub(super) fn observed_instrument_sizing_venues(
    checks: &[InstrumentSizingCheck<'_>],
) -> Vec<String> {
    checks
        .iter()
        .filter(|check| check.venue_covered && check.outcome.is_ok())
        .map(|check| normalized_venue_name(&check.plan.exchange))
        .filter(|venue| !venue.is_empty())
        .fold(Vec::new(), push_unique)
}
