//! Execution sizing plan DTOs (PR-BM).
//!
//! 把请求名义（capital × leverage → `target_notional_usd`）转换成交易所**合法**
//! 下单数量的单一事实源：给定 [`crate::instrument_registry::VenueInstrument`] 的
//! 注册表规格（`contract_size`/`qty_step`/`min_notional`）与参考价，算出对齐步长
//! 后的合约张数、真实数量、真实名义与 rounding delta，并在 `HedgeTicket` 阶段就
//! **fail-closed** 阻断（规格缺失、不足一步长、低于 `min_notional` 一律拒绝），
//! 绝不把非法 size 留到 adapter/交易所才发现。单位约定：`reference_price` 为
//! 报价资产/基础单位，`contract_size` 为每张合约的基础单位数（现货/线性默认 1），
//! `qty_step` 为合约张数步长，`min_notional` 为最小名义；`contract_size = 1` 时
//! 退化为基础数量步长。

use crate::instrument_registry::VenueInstrument;
use serde::{Deserialize, Serialize};
use std::fmt;

/// sizing 请求被 fail-closed 阻断的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizingBlock {
    /// 请求侧 `target_notional_usd` 或 `reference_price` 非有限或非正。
    NonFiniteRequest,
    /// 注册表条目不可据此构建对冲（来源非官方/非 Trading/必需规格缺失）。
    SpecMissing,
    /// 对齐到步长后数量落到一个 `qty_step` 以下（不足一张/一步）。
    BelowOneStep,
    /// 对齐后合约张数低于交易所最小下单张数 `min_qty`（如 OKX `minSz`）。
    BelowMinQty,
    /// 对齐后真实名义低于 `min_notional`。
    BelowMinNotional,
    /// 双腿按各自官方数量步长对齐后无法得到相同的基础资产数量。
    PairQuantityMismatch,
}

impl SizingBlock {
    /// 稳定的机器可读阻断码，供 preflight blocker 文案与前端分类复用。
    pub fn code(self) -> &'static str {
        match self {
            Self::NonFiniteRequest => "SIZING_NON_FINITE_REQUEST",
            Self::SpecMissing => "SIZING_SPEC_MISSING",
            Self::BelowOneStep => "SIZING_BELOW_ONE_STEP",
            Self::BelowMinQty => "SIZING_BELOW_MIN_QTY",
            Self::BelowMinNotional => "SIZING_BELOW_MIN_NOTIONAL",
            Self::PairQuantityMismatch => "SIZING_PAIR_QUANTITY_MISMATCH",
        }
    }
}

/// 从注册表规格归一化出的下单规格输入（各字段须有限且为正）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizingSpec {
    pub reference_price: f64,
    pub price_tick: f64,
    pub contract_size: f64,
    pub qty_step: f64,
    /// 最小下单张数下限（合约张数）；`0.0` 表示该交易所不提供此下限。
    pub min_qty: f64,
    /// 最小名义下限；`0.0` 表示该交易所不提供此下限。
    pub min_notional: f64,
}

impl SizingSpec {
    /// 从注册表条目 + 参考价归一化出 sizing 规格。
    ///
    /// 仅当 `instrument` [`VenueInstrument::is_hedge_constructible`] 为真（已保证
    /// `qty_step`/`min_notional` 给出且有限为正）且 `reference_price` 有限为正时
    /// 返回 `Some`；否则返回 `None`（交由调用方记 [`SizingBlock::SpecMissing`] /
    /// [`SizingBlock::NonFiniteRequest`]）。`contract_size` 缺失退化为 1。
    pub fn from_instrument(instrument: &VenueInstrument, reference_price: f64) -> Option<Self> {
        if !instrument.is_hedge_constructible() {
            return None;
        }
        if !is_finite_positive(reference_price) {
            return None;
        }
        let qty_step = instrument.qty_step?;
        let price_tick = instrument.price_tick?;
        let contract_size = instrument.contract_size.unwrap_or(1.0);
        let min_notional = instrument.min_notional.unwrap_or(0.0);
        let min_qty = instrument.min_qty.unwrap_or(0.0);
        if !(is_finite_positive(qty_step) && is_finite_positive(contract_size)) {
            return None;
        }
        // 最小下单下限：`min_notional`/`min_qty` 至少其一为正（fail-closed）。
        if !(is_finite_positive(min_notional) || is_finite_positive(min_qty)) {
            return None;
        }
        Some(Self {
            reference_price,
            price_tick,
            contract_size,
            qty_step,
            min_qty,
            min_notional,
        })
    }
}

/// 归一化后的执行 sizing 计划——前端可据此展示真实可下单数量与调整差额。
///
/// `rounded_contracts` 向下取整（绝不上修以免超额），`rounded_base_qty =
/// rounded_contracts * contract_size`，`actual_notional_usd = rounded_base_qty *
/// reference_price`，`rounding_delta_usd = target - actual`（恒 `>= 0`）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSizingPlan {
    pub target_notional_usd: f64,
    pub reference_price: f64,
    #[serde(default)]
    pub price_tick: f64,
    pub contract_size: f64,
    pub qty_step: f64,
    #[serde(default)]
    pub min_qty: f64,
    #[serde(default)]
    pub min_notional: f64,
    #[serde(default)]
    pub raw_contracts: f64,
    #[serde(default)]
    pub raw_base_qty: f64,
    pub rounded_contracts: f64,
    pub rounded_base_qty: f64,
    pub actual_notional_usd: f64,
    pub rounding_delta_usd: f64,
    #[serde(default)]
    pub rounding_loss_bps: f64,
}

/// Product-facing name for the ticket-bound execution sizing contract.
pub type OrderSizingPlan = ExecutionSizingPlan;

/// 双腿共享同一基础资产数量的下单尺寸计划。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedExecutionSizingPlan {
    pub base_quantity: f64,
    pub long: ExecutionSizingPlan,
    pub short: ExecutionSizingPlan,
}

/// Stable failure classification for a ticket-bound instrument/sizing pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSizingContractError {
    InstrumentMissing,
    SizingMissing,
    VenueMismatch,
    SymbolMismatch,
    SizingBlocked(SizingBlock),
    PlanMismatch,
}

impl OrderSizingContractError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InstrumentMissing => "INSTRUMENT_SPEC_MISSING",
            Self::SizingMissing => "ORDER_SIZING_PLAN_MISSING",
            Self::VenueMismatch => "INSTRUMENT_VENUE_MISMATCH",
            Self::SymbolMismatch => "INSTRUMENT_SYMBOL_MISMATCH",
            Self::SizingBlocked(block) => block.code(),
            Self::PlanMismatch => "ORDER_SIZING_PLAN_MISMATCH",
        }
    }
}

impl fmt::Display for OrderSizingContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

fn is_finite_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

/// 从注册表条目 fail-closed 计算 sizing 计划：校验请求/规格 → 名义换算成合约张数
/// 并 **向下** 对齐 `qty_step` → 不足一张 [`SizingBlock::BelowOneStep`] → 还原真实
/// 名义与 `min_notional` 比较 → 不足 [`SizingBlock::BelowMinNotional`]。
pub fn plan_leg_sizing(
    target_notional_usd: f64,
    instrument: &VenueInstrument,
    reference_price: f64,
) -> Result<ExecutionSizingPlan, SizingBlock> {
    if !is_finite_positive(target_notional_usd) || !is_finite_positive(reference_price) {
        return Err(SizingBlock::NonFiniteRequest);
    }
    let spec =
        SizingSpec::from_instrument(instrument, reference_price).ok_or(SizingBlock::SpecMissing)?;
    plan_from_spec(target_notional_usd, &spec)
}

/// 按已确定的基础资产数量生成单腿合法尺寸。数量无法被场所步长精确表达时 fail-closed。
pub fn plan_leg_sizing_for_base_quantity(
    target_base_quantity: f64,
    instrument: &VenueInstrument,
    reference_price: f64,
) -> Result<ExecutionSizingPlan, SizingBlock> {
    if !is_finite_positive(target_base_quantity) || !is_finite_positive(reference_price) {
        return Err(SizingBlock::NonFiniteRequest);
    }
    let target_notional_usd = target_base_quantity * reference_price;
    if !is_finite_positive(target_notional_usd) {
        return Err(SizingBlock::NonFiniteRequest);
    }
    let plan = plan_leg_sizing(target_notional_usd, instrument, reference_price)?;
    if quantities_match(plan.rounded_base_qty, target_base_quantity) {
        Ok(plan)
    } else {
        Err(SizingBlock::PairQuantityMismatch)
    }
}

/// 在两腿各自的名义上限和官方数量步长内，寻找最大的共同基础资产数量。
///
/// 每轮只向下收敛，绝不放大任一腿的请求；常见十进制步长通常一到两轮收敛，
/// 异构且无法共同表达的规格会在有界轮数后 fail-closed。
pub fn plan_paired_leg_sizing(
    long_notional_cap_usd: f64,
    long_instrument: &VenueInstrument,
    long_reference_price: f64,
    short_notional_cap_usd: f64,
    short_instrument: &VenueInstrument,
    short_reference_price: f64,
) -> Result<PairedExecutionSizingPlan, SizingBlock> {
    if !is_finite_positive(long_notional_cap_usd)
        || !is_finite_positive(short_notional_cap_usd)
        || !is_finite_positive(long_reference_price)
        || !is_finite_positive(short_reference_price)
    {
        return Err(SizingBlock::NonFiniteRequest);
    }
    let mut target_base_quantity = (long_notional_cap_usd / long_reference_price)
        .min(short_notional_cap_usd / short_reference_price);
    for _ in 0..64 {
        let long = plan_leg_sizing(
            target_base_quantity * long_reference_price,
            long_instrument,
            long_reference_price,
        )?;
        let short = plan_leg_sizing(
            target_base_quantity * short_reference_price,
            short_instrument,
            short_reference_price,
        )?;
        let paired_quantity = long.rounded_base_qty.min(short.rounded_base_qty);
        if quantities_match(long.rounded_base_qty, short.rounded_base_qty) {
            let long = plan_leg_sizing_for_base_quantity(
                paired_quantity,
                long_instrument,
                long_reference_price,
            )?;
            let short = plan_leg_sizing_for_base_quantity(
                paired_quantity,
                short_instrument,
                short_reference_price,
            )?;
            return Ok(PairedExecutionSizingPlan {
                base_quantity: paired_quantity,
                long,
                short,
            });
        }
        if !is_finite_positive(paired_quantity)
            || !strictly_smaller_quantity(paired_quantity, target_base_quantity)
        {
            return Err(SizingBlock::PairQuantityMismatch);
        }
        target_base_quantity = paired_quantity;
    }
    Err(SizingBlock::PairQuantityMismatch)
}

/// Recompute and compare a ticket-bound sizing plan before confirmation or write submission.
pub fn validate_order_sizing_contract(
    venue: &str,
    symbol: &str,
    instrument: &VenueInstrument,
    sizing: ExecutionSizingPlan,
) -> Result<(), OrderSizingContractError> {
    if !crate::venues::venue_names_equal(&instrument.venue, venue) {
        return Err(OrderSizingContractError::VenueMismatch);
    }
    if !(instrument.canonical_symbol.eq_ignore_ascii_case(symbol)
        || instrument.native_symbol.eq_ignore_ascii_case(symbol))
    {
        return Err(OrderSizingContractError::SymbolMismatch);
    }
    let expected = plan_leg_sizing(
        sizing.target_notional_usd,
        instrument,
        sizing.reference_price,
    )
    .map_err(OrderSizingContractError::SizingBlocked)?;
    if !sizing_plans_match(expected, sizing) {
        return Err(OrderSizingContractError::PlanMismatch);
    }
    Ok(())
}

fn sizing_plans_match(expected: ExecutionSizingPlan, actual: ExecutionSizingPlan) -> bool {
    [
        (expected.target_notional_usd, actual.target_notional_usd),
        (expected.reference_price, actual.reference_price),
        (expected.price_tick, actual.price_tick),
        (expected.contract_size, actual.contract_size),
        (expected.qty_step, actual.qty_step),
        (expected.min_qty, actual.min_qty),
        (expected.min_notional, actual.min_notional),
        (expected.raw_contracts, actual.raw_contracts),
        (expected.raw_base_qty, actual.raw_base_qty),
        (expected.rounded_contracts, actual.rounded_contracts),
        (expected.rounded_base_qty, actual.rounded_base_qty),
        (expected.actual_notional_usd, actual.actual_notional_usd),
        (expected.rounding_delta_usd, actual.rounding_delta_usd),
        (expected.rounding_loss_bps, actual.rounding_loss_bps),
    ]
    .into_iter()
    .all(|(left, right)| approximately_equal(left, right))
}

fn approximately_equal(left: f64, right: f64) -> bool {
    if !(left.is_finite() && right.is_finite()) {
        return false;
    }
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= f64::EPSILON * 64.0 * scale
}

/// 从已归一化的 [`SizingSpec`] fail-closed 计算 sizing 计划。
pub fn plan_from_spec(
    target_notional_usd: f64,
    spec: &SizingSpec,
) -> Result<ExecutionSizingPlan, SizingBlock> {
    if !is_finite_positive(target_notional_usd) {
        return Err(SizingBlock::NonFiniteRequest);
    }
    let has_min_notional = is_finite_positive(spec.min_notional);
    let has_min_qty = is_finite_positive(spec.min_qty);
    if !(is_finite_positive(spec.reference_price)
        && is_finite_positive(spec.price_tick)
        && is_finite_positive(spec.contract_size)
        && is_finite_positive(spec.qty_step)
        && (has_min_notional || has_min_qty))
    {
        return Err(SizingBlock::SpecMissing);
    }
    let target_base_qty = target_notional_usd / spec.reference_price;
    let target_contracts = target_base_qty / spec.contract_size;
    let raw_steps = target_contracts / spec.qty_step;
    let floor_tolerance = f64::EPSILON * 64.0 * raw_steps.abs().max(1.0);
    let steps = (raw_steps + floor_tolerance).floor();
    let rounded_contracts = steps * spec.qty_step;
    if !is_finite_positive(rounded_contracts) {
        return Err(SizingBlock::BelowOneStep);
    }
    if has_min_qty && rounded_contracts + 1e-9 < spec.min_qty {
        return Err(SizingBlock::BelowMinQty);
    }
    let rounded_base_qty = rounded_contracts * spec.contract_size;
    let actual_notional_usd = rounded_base_qty * spec.reference_price;
    if !actual_notional_usd.is_finite() {
        return Err(SizingBlock::NonFiniteRequest);
    }
    if has_min_notional && actual_notional_usd + 1e-9 < spec.min_notional {
        return Err(SizingBlock::BelowMinNotional);
    }
    let rounding_delta_usd = (target_notional_usd - actual_notional_usd).max(0.0);
    let rounding_loss_bps = rounding_delta_usd / target_notional_usd * 10_000.0;
    Ok(ExecutionSizingPlan {
        target_notional_usd,
        reference_price: spec.reference_price,
        price_tick: spec.price_tick,
        contract_size: spec.contract_size,
        qty_step: spec.qty_step,
        min_qty: spec.min_qty,
        min_notional: spec.min_notional,
        raw_contracts: target_contracts,
        raw_base_qty: target_base_qty,
        rounded_contracts,
        rounded_base_qty,
        actual_notional_usd,
        rounding_delta_usd,
        rounding_loss_bps,
    })
}

fn quantities_match(left: f64, right: f64) -> bool {
    if !(left.is_finite() && right.is_finite()) {
        return false;
    }
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= 1e-12_f64.max(f64::EPSILON * 128.0 * scale)
}

fn strictly_smaller_quantity(candidate: f64, previous: f64) -> bool {
    candidate < previous && !quantities_match(candidate, previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument_registry::InstrumentAssetClass;
    use crate::instruments::{InstrumentListingStatus, InstrumentMetadataSource};

    fn linear_instrument() -> VenueInstrument {
        VenueInstrument {
            venue: "binance".to_owned(),
            native_symbol: "BTCUSDT".to_owned(),
            canonical_symbol: "BTC/USDT".to_owned(),
            display_symbol: "BTC-USDT Perp".to_owned(),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("perp".to_owned()),
            quote_asset: Some("USDT".to_owned()),
            settle_asset: Some("USDT".to_owned()),
            margin_asset: Some("USDT".to_owned()),
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(0.1),
            qty_step: Some(0.001),
            min_qty: None,
            min_notional: Some(5.0),
            listing_status: InstrumentListingStatus::Trading,
            funding_interval_ms: Some(28_800_000),
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some("https://api.binance.com/exchangeInfo".to_owned()),
            checked_at_ms: 1_700_000_000_000,
            schema_version: Some("v1".to_owned()),
        }
    }

    #[test]
    fn linear_full_spec_rounds_down_to_step() {
        // 1234 USD @ 30000 -> 0.041133.. base; step 0.001 -> 0.041.
        let plan = plan_leg_sizing(1234.0, &linear_instrument(), 30_000.0).expect("plan");
        assert!((plan.rounded_base_qty - 0.041).abs() < 1e-9);
        assert!((plan.rounded_contracts - 0.041).abs() < 1e-9);
        assert!((plan.actual_notional_usd - 1230.0).abs() < 1e-6);
        assert!((plan.rounding_delta_usd - 4.0).abs() < 1e-6);
        assert_eq!(plan.price_tick, 0.1);
        assert!((plan.raw_contracts - (1234.0 / 30_000.0)).abs() < 1e-9);
        assert!((plan.raw_base_qty - plan.raw_contracts).abs() < 1e-9);
        assert!((plan.rounding_loss_bps - (4.0 / 1234.0 * 10_000.0)).abs() < 1e-6);
    }

    #[test]
    fn contract_size_scales_base_qty_and_notional() {
        // Gate/KuCoin style: 1 contract = 10 base units, step in contracts.
        let mut inst = linear_instrument();
        inst.contract_size = Some(10.0);
        inst.qty_step = Some(1.0);
        inst.min_notional = Some(5.0);
        // 3500 USD @ 100 -> 35 base -> 3.5 contracts -> floor 3.
        let plan = plan_leg_sizing(3500.0, &inst, 100.0).expect("plan");
        assert!((plan.rounded_contracts - 3.0).abs() < 1e-9);
        assert!((plan.rounded_base_qty - 30.0).abs() < 1e-9);
        assert!((plan.actual_notional_usd - 3000.0).abs() < 1e-6);
        assert!((plan.rounding_delta_usd - 500.0).abs() < 1e-6);
    }

    #[test]
    fn below_one_step_is_blocked() {
        let mut inst = linear_instrument();
        inst.qty_step = Some(1.0);
        inst.contract_size = Some(1.0);
        inst.min_notional = Some(1.0);
        // 50 USD @ 100 -> 0.5 contracts -> floor 0 -> below one step.
        let err = plan_leg_sizing(50.0, &inst, 100.0).expect_err("block");
        assert_eq!(err, SizingBlock::BelowOneStep);
        assert_eq!(err.code(), "SIZING_BELOW_ONE_STEP");
    }

    #[test]
    fn below_min_notional_is_blocked() {
        let mut inst = linear_instrument();
        inst.qty_step = Some(0.001);
        inst.contract_size = Some(1.0);
        inst.min_notional = Some(100.0);
        // 50 USD @ 30000 -> tiny qty whose notional < 100.
        let err = plan_leg_sizing(50.0, &inst, 30_000.0).expect_err("block");
        assert_eq!(err, SizingBlock::BelowMinNotional);
    }

    #[test]
    fn non_finite_request_is_blocked() {
        let inst = linear_instrument();
        for (notional, price) in [
            (f64::NAN, 100.0),
            (f64::INFINITY, 100.0),
            (-1.0, 100.0),
            (0.0, 100.0),
            (1000.0, f64::NAN),
            (1000.0, 0.0),
            (1000.0, -5.0),
        ] {
            let err = plan_leg_sizing(notional, &inst, price).expect_err("block");
            assert_eq!(err, SizingBlock::NonFiniteRequest);
        }
    }

    #[test]
    fn non_official_non_trading_or_missing_field_is_spec_missing() {
        let mut cases = vec![linear_instrument(); 5];
        cases[0].source = InstrumentMetadataSource::CachedSnapshot;
        cases[1].listing_status = InstrumentListingStatus::Suspended;
        cases[2].qty_step = None;
        cases[3].min_notional = None;
        cases[4].price_tick = None;
        for inst in cases {
            assert_eq!(
                plan_leg_sizing(1000.0, &inst, 100.0).expect_err("block"),
                SizingBlock::SpecMissing
            );
        }
    }

    #[test]
    fn below_min_qty_is_blocked() {
        // OKX 风格：只给最小张数 minSz、不给最小名义。
        let mut inst = linear_instrument();
        inst.qty_step = Some(1.0);
        inst.contract_size = Some(1.0);
        inst.min_notional = None;
        inst.min_qty = Some(5.0);
        // 300 USD @ 100 -> 3 contracts (floor) < min_qty 5 -> blocked.
        let err = plan_leg_sizing(300.0, &inst, 100.0).expect_err("block");
        assert_eq!(err, SizingBlock::BelowMinQty);
        assert_eq!(err.code(), "SIZING_BELOW_MIN_QTY");
    }

    #[test]
    fn min_qty_only_spec_sizes_successfully() {
        // 只有 min_qty（无 min_notional）也能正常出 sizing 计划。
        let mut inst = linear_instrument();
        inst.qty_step = Some(1.0);
        inst.contract_size = Some(1.0);
        inst.min_notional = None;
        inst.min_qty = Some(5.0);
        // 800 USD @ 100 -> 8 contracts (floor) >= min_qty 5 -> ok.
        let plan = plan_leg_sizing(800.0, &inst, 100.0).expect("plan");
        assert!((plan.rounded_contracts - 8.0).abs() < 1e-9);
        assert!((plan.actual_notional_usd - 800.0).abs() < 1e-6);
    }

    #[test]
    fn exact_min_qty_is_allowed() {
        let mut inst = linear_instrument();
        inst.qty_step = Some(1.0);
        inst.contract_size = Some(1.0);
        inst.min_notional = None;
        inst.min_qty = Some(5.0);
        // 500 USD @ 100 -> exactly 5 contracts == min_qty.
        let plan = plan_leg_sizing(500.0, &inst, 100.0).expect("plan");
        assert!((plan.rounded_contracts - 5.0).abs() < 1e-9);
    }

    #[test]
    fn from_instrument_defaults_contract_size_to_one() {
        let mut inst = linear_instrument();
        inst.contract_size = None;
        let spec = SizingSpec::from_instrument(&inst, 100.0).expect("spec");
        assert!((spec.contract_size - 1.0).abs() < 1e-9);
    }

    #[test]
    fn exact_min_notional_is_allowed() {
        let mut inst = linear_instrument();
        inst.qty_step = Some(1.0);
        inst.contract_size = Some(1.0);
        inst.min_notional = Some(100.0);
        // 100 USD @ 100 -> exactly 1 contract -> notional 100 == min.
        let plan = plan_leg_sizing(100.0, &inst, 100.0).expect("plan");
        assert!((plan.actual_notional_usd - 100.0).abs() < 1e-9);
        assert!((plan.rounding_delta_usd).abs() < 1e-9);
    }

    #[test]
    fn ticket_contract_rejects_wrong_identity_and_tampered_plan() {
        let instrument = linear_instrument();
        let sizing = plan_leg_sizing(1234.0, &instrument, 30_000.0).expect("plan");

        assert_eq!(
            validate_order_sizing_contract("okx", "BTC/USDT", &instrument, sizing),
            Err(OrderSizingContractError::VenueMismatch)
        );
        assert_eq!(
            validate_order_sizing_contract("binance", "ETH/USDT", &instrument, sizing),
            Err(OrderSizingContractError::SymbolMismatch)
        );
        let mut roundtripped = sizing;
        roundtripped.rounding_loss_bps += f64::EPSILON * sizing.rounding_loss_bps * 8.0;
        assert_eq!(
            validate_order_sizing_contract("binance", "BTC/USDT", &instrument, roundtripped),
            Ok(())
        );
        let mut tampered = sizing;
        tampered.rounded_base_qty = 99.0;
        assert_eq!(
            validate_order_sizing_contract("binance", "BTC/USDT", &instrument, tampered),
            Err(OrderSizingContractError::PlanMismatch)
        );
    }

    #[test]
    fn paired_sizing_uses_one_base_quantity_across_different_prices_and_steps() {
        let long = linear_instrument();
        let mut short = linear_instrument();
        short.qty_step = Some(0.01);

        let paired = plan_paired_leg_sizing(1_234.0, &long, 30_000.0, 1_234.0, &short, 30_100.0)
            .expect("paired plan");

        assert!((paired.base_quantity - 0.04).abs() < 1e-12);
        assert!((paired.long.rounded_base_qty - paired.base_quantity).abs() < 1e-12);
        assert!((paired.short.rounded_base_qty - paired.base_quantity).abs() < 1e-12);
        assert_ne!(
            paired.long.actual_notional_usd,
            paired.short.actual_notional_usd
        );
    }

    #[test]
    fn exact_base_quantity_rejects_a_leg_that_cannot_express_it() {
        let mut instrument = linear_instrument();
        instrument.qty_step = Some(0.01);

        assert_eq!(
            plan_leg_sizing_for_base_quantity(0.041, &instrument, 30_000.0),
            Err(SizingBlock::PairQuantityMismatch)
        );
    }
}
