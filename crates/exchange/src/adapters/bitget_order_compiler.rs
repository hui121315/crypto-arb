//! Bitget UTA order identity and constraint compiler.

use super::bitget_instruments::BitgetInstrumentSpec;
use super::bitget_uta_config::BitgetUtaCategory;
use super::spot_order_contract;
use crate::error::{ExchangeError, ExchangeResult};
use shared_types::instruments::InstrumentListingStatus;
use shared_types::{OrderIntent, OrderSide, OrderSubmissionContext, OrderType};

const NAME: &str = "bitget";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BitgetPositionMode {
    OneWay,
    Hedge,
}

impl BitgetPositionMode {
    pub(super) fn parse(raw: &str) -> ExchangeResult<Self> {
        match raw {
            "one_way_mode" => Ok(Self::OneWay),
            "hedge_mode" => Ok(Self::Hedge),
            _ => Err(validation_error(format!(
                "unsupported Bitget holdMode {raw:?}"
            ))),
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::OneWay => "one_way_mode",
            Self::Hedge => "hedge_mode",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CompiledBitgetOrder {
    pub(super) category: BitgetUtaCategory,
    pub(super) native_symbol: String,
    pub(super) quantity: f64,
    pub(super) pos_side: Option<&'static str>,
    pub(super) reduce_only: bool,
}

pub(super) fn compile_order(
    intent: &OrderIntent,
    spec: &BitgetInstrumentSpec,
    position_mode: BitgetPositionMode,
) -> ExchangeResult<CompiledBitgetOrder> {
    validate_execution_boundary(spec)?;
    validate_step(intent.quantity, spec.qty_step, "quantity")?;
    if intent.quantity + tolerance(spec.min_qty) < spec.min_qty {
        return Err(validation_error(format!(
            "quantity {} is below Bitget minOrderQty {} for {}",
            intent.quantity, spec.min_qty, spec.native_symbol
        )));
    }
    if matches!(intent.order_type, OrderType::Limit | OrderType::PostOnly) {
        let price = intent
            .price
            .ok_or_else(|| validation_error("limit order requires price".to_owned()))?;
        validate_step(price, spec.price_tick, "price")?;
        if spec
            .min_notional
            .is_some_and(|floor| intent.quantity * price + tolerance(floor) < floor)
        {
            return Err(validation_error(format!(
                "order notional is below Bitget minOrderAmount for {}",
                spec.native_symbol
            )));
        }
    }

    let pos_side = match position_mode {
        BitgetPositionMode::OneWay => None,
        BitgetPositionMode::Hedge => Some(match (intent.side, intent.reduce_only) {
            (OrderSide::Buy, false) | (OrderSide::Sell, true) => "long",
            (OrderSide::Sell, false) | (OrderSide::Buy, true) => "short",
        }),
    };
    Ok(CompiledBitgetOrder {
        category: spec.category,
        native_symbol: spec.native_symbol.clone(),
        quantity: intent.quantity,
        pos_side,
        reduce_only: position_mode == BitgetPositionMode::OneWay && intent.reduce_only,
    })
}

pub(super) fn compile_spot_order(
    intent: &OrderIntent,
    context: &OrderSubmissionContext,
) -> ExchangeResult<CompiledBitgetOrder> {
    let compiled = spot_order_contract::compile(NAME, intent, context)?;
    Ok(CompiledBitgetOrder {
        category: BitgetUtaCategory::Spot,
        native_symbol: compiled.native_symbol,
        quantity: intent.quantity,
        pos_side: None,
        reduce_only: false,
    })
}

fn validate_execution_boundary(spec: &BitgetInstrumentSpec) -> ExchangeResult<()> {
    if !spec.execution_supported {
        return Err(validation_error(format!(
            "{} is observation-only: inverse/Reality sizing is not supported",
            spec.native_symbol
        )));
    }
    if !matches!(
        spec.category,
        BitgetUtaCategory::UsdtFutures | BitgetUtaCategory::UsdcFutures | BitgetUtaCategory::Spot
    ) {
        return Err(validation_error(format!(
            "{} category is not an executable UTA product",
            spec.native_symbol
        )));
    }
    if spec.listing_status != InstrumentListingStatus::Trading {
        return Err(validation_error(format!(
            "{} is not online for execution",
            spec.native_symbol
        )));
    }
    Ok(())
}

fn validate_step(value: f64, step: f64, field: &str) -> ExchangeResult<()> {
    if !value.is_finite() || value <= 0.0 || !step.is_finite() || step <= 0.0 {
        return Err(validation_error(format!(
            "invalid {field}/step: value={value}, step={step}"
        )));
    }
    let units = value / step;
    if (units - units.round()).abs() > tolerance(units) {
        return Err(validation_error(format!(
            "Bitget {field} {value} is not aligned to step {step}"
        )));
    }
    Ok(())
}

fn tolerance(value: f64) -> f64 {
    value.abs().max(1.0) * 1e-9
}

fn validation_error(message: String) -> ExchangeError {
    ExchangeError::Api {
        exchange: NAME.to_owned(),
        code: "validation".to_owned(),
        message,
    }
}

#[cfg(test)]
#[path = "bitget_order_compiler_tests.rs"]
mod tests;
