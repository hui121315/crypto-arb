use axum::http::StatusCode;
use common::AppError;
use exchange::client_order_id_policy;
use shared_types::{
    problem::codes, venue_names_equal, ArbitrageOpportunityDto, ExecutionMode, FeeProduct,
    HedgeExecutionParams, HedgeLegRole, HedgePreviewRequest, OpportunityLegMarketEvidence,
    OrderCompilePlan, OrderIntent, OrderSide, OrderSource, OrderType, StrategyKind,
    VenueSymbolCapability,
};

#[path = "intent/compile.rs"]
mod compile;
use compile::{compile_spec, compiler_time_in_force_options, order_plan_blockers};

pub(super) fn validate_preview(id: &str, req: &mut HedgePreviewRequest) -> Result<(), AppError> {
    if req.opportunity_id.trim().is_empty() {
        req.opportunity_id = id.to_owned();
    } else if req.opportunity_id != id {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_PREVIEW_OPPORTUNITY_MISMATCH,
            "request opportunityId does not match the preview path",
        )
        .with_details(serde_json::json!({
            "pathOpportunityId": id,
            "requestOpportunityId": req.opportunity_id,
        })));
    }
    require_positive_finite(req.capital_usd, "capitalUsd")?;
    require_positive_finite(req.leverage, "leverage")?;
    if let Some(params) = req.execution_params.as_ref() {
        validate_execution_params(params)?;
    }
    validate_optional_positive(req.long_price, "longPrice")?;
    validate_optional_positive(req.short_price, "shortPrice")?;
    validate_optional_positive(req.long_notional_usd, "longNotionalUsd")?;
    validate_optional_positive(req.short_notional_usd, "shortNotionalUsd")?;
    Ok(())
}

pub(super) fn leg_price(fallback: Option<f64>, field: &str) -> Result<f64, AppError> {
    fallback
        .filter(|price| price.is_finite() && *price > 0.0)
        .ok_or_else(|| AppError::BadRequest(format!("{field} reference price is required")))
}

pub(crate) fn build_leg(input: LegBuild<'_>) -> OrderIntent {
    let spec = IntentLegSpec::from_input(&input);
    let role = if input.is_long {
        HedgeLegRole::Long
    } else {
        HedgeLegRole::Short
    };
    let order_type = order_type(input.params);
    let client_order_id = stable_client_order_id(input.key, spec.label);
    let client_order_id_policy = Some(client_order_id_policy(&spec.exchange, &client_order_id));
    OrderIntent {
        id: format!("{}-{}", input.key, spec.label),
        source: OrderSource::ArbitragePreview,
        strategy: input.strategy,
        mode: input.mode,
        exchange: spec.exchange,
        symbol: spec.symbol,
        side: spec.side,
        order_type,
        quantity: input.quantity,
        price: Some(limit_price(
            input.price,
            input.is_long,
            input.params.limit_offset_bps,
        )),
        slippage_tolerance_bps: market_slippage_tolerance_bps(input.params),
        reduce_only: false,
        time_in_force: input.params.time_in_force,
        post_only: matches!(order_type, OrderType::PostOnly),
        margin_mode: input.params.margin_mode,
        leverage: leg_leverage(input.opp, role, input.params.leverage),
        client_order_id,
        client_order_id_policy,
        created_at_ms: common::time::now_ms(),
    }
}

pub(crate) fn compile_order_plan(input: &LegBuild<'_>, intent: &OrderIntent) -> OrderCompilePlan {
    let role = if input.is_long {
        HedgeLegRole::Long
    } else {
        HedgeLegRole::Short
    };
    let product = leg_product(input.opp, role);
    let spec = compile_spec(intent, input.params.market_order_style, product);
    let client_order_id_policy = intent
        .client_order_id_policy
        .clone()
        .unwrap_or_else(|| client_order_id_policy(&intent.exchange, &intent.client_order_id));
    let payload_price = spec.payload_price(intent.price);
    let blockers = order_plan_blockers(spec.blockers, &client_order_id_policy);
    OrderCompilePlan {
        role,
        exchange: intent.exchange.clone(),
        symbol: intent.symbol.clone(),
        client_order_id_policy,
        product,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: order_type(input.params),
        effective_order_type: spec.effective_order_type,
        requested_time_in_force: input.params.time_in_force,
        effective_time_in_force: spec.effective_time_in_force,
        available_order_types: Vec::new(),
        available_time_in_force: compiler_time_in_force_options(intent, product),
        available_margin_modes: Vec::new(),
        venue_capability: VenueSymbolCapability::default(),
        market_order_style: input.params.market_order_style,
        venue_order_kind: spec.venue_order_kind,
        payload_price_policy: spec.payload_price_policy,
        reference_price: Some(input.price),
        protection_price: intent.price,
        payload_price,
        slippage_tolerance_bps: intent.slippage_tolerance_bps,
        summary: spec.summary,
        blockers,
    }
}

fn leg_product(opp: &ArbitrageOpportunityDto, role: HedgeLegRole) -> FeeProduct {
    crate::services::hedge_ticket::fee_product_for(opp, role)
}

fn leg_leverage(opp: &ArbitrageOpportunityDto, role: HedgeLegRole, requested: f64) -> f64 {
    leverage_for_product(leg_product(opp, role), requested)
}

fn leverage_for_product(product: FeeProduct, requested: f64) -> f64 {
    if product == FeeProduct::Spot {
        1.0
    } else {
        requested
    }
}

struct IntentLegSpec {
    exchange: String,
    symbol: String,
    side: OrderSide,
    label: &'static str,
}

impl IntentLegSpec {
    fn from_input(input: &LegBuild<'_>) -> Self {
        if input.is_long {
            let exchange = input.opp.long_exchange.clone();
            Self {
                symbol: evidence_bound_symbol(
                    &exchange,
                    input.opp.long_leg_market_evidence.as_ref(),
                    &input.opp.symbol,
                ),
                exchange,
                side: OrderSide::Buy,
                label: "long",
            }
        } else {
            let exchange = input.opp.short_exchange.clone();
            Self {
                symbol: evidence_bound_symbol(
                    &exchange,
                    input.opp.short_leg_market_evidence.as_ref(),
                    &input.opp.symbol,
                ),
                exchange,
                side: OrderSide::Sell,
                label: "short",
            }
        }
    }
}

fn evidence_bound_symbol(
    exchange: &str,
    evidence: Option<&OpportunityLegMarketEvidence>,
    fallback: &str,
) -> String {
    evidence
        .filter(|evidence| venue_names_equal(&evidence.venue, exchange))
        .map(|evidence| evidence.symbol.trim())
        .filter(|symbol| !symbol.is_empty())
        .map_or_else(|| fallback.to_owned(), str::to_owned)
}

pub(crate) fn execution_mode_for_adapter(adapter: &str) -> ExecutionMode {
    match adapter {
        "live" => ExecutionMode::Live,
        _ => ExecutionMode::DryRun,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LegBuild<'a> {
    pub(crate) opp: &'a ArbitrageOpportunityDto,
    pub(crate) key: &'a str,
    pub(crate) is_long: bool,
    pub(crate) quantity: f64,
    pub(crate) price: f64,
    pub(crate) params: &'a HedgeExecutionParams,
    pub(crate) mode: ExecutionMode,
    pub(crate) strategy: Option<StrategyKind>,
}

fn validate_execution_params(params: &HedgeExecutionParams) -> Result<(), AppError> {
    require_positive_finite(params.capital_usd, "executionParams.capitalUsd")?;
    require_positive_finite(params.leverage, "executionParams.leverage")?;
    if !params.limit_offset_bps.is_finite() {
        return Err(AppError::BadRequest(
            "executionParams.limitOffsetBps must be finite".into(),
        ));
    }
    if params.limit_offset_bps.abs() > 500.0 {
        return Err(AppError::BadRequest(
            "executionParams.limitOffsetBps must be within +/-500".into(),
        ));
    }
    if matches!(params.order_type, OrderType::Market) && params.post_only {
        return Err(AppError::BadRequest(
            "executionParams.orderType market cannot be postOnly".into(),
        ));
    }
    Ok(())
}

fn require_positive_finite(value: f64, field: &str) -> Result<(), AppError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(AppError::BadRequest(format!(
            "{field} must be a positive finite number"
        )));
    }
    Ok(())
}

fn validate_optional_positive(value: Option<f64>, field: &str) -> Result<(), AppError> {
    if value.is_some_and(|value| !value.is_finite() || value <= 0.0) {
        return Err(AppError::BadRequest(format!("{field} must be positive")));
    }
    Ok(())
}

fn order_type(params: &HedgeExecutionParams) -> OrderType {
    if params.post_only {
        OrderType::PostOnly
    } else {
        params.order_type
    }
}

fn market_slippage_tolerance_bps(params: &HedgeExecutionParams) -> Option<f64> {
    matches!(order_type(params), OrderType::Market)
        .then_some(params.limit_offset_bps.abs())
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn limit_price(reference: f64, is_long: bool, offset_bps: f64) -> f64 {
    let signed_offset = if is_long { offset_bps } else { -offset_bps };
    reference * (1.0 + signed_offset / 10_000.0)
}

fn stable_client_order_id(key: &str, label: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes().chain([b':']).chain(label.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let side = if label == "long" { 'l' } else { 's' };
    format!("xl{hash:016x}{side}")
}

#[cfg(test)]
#[path = "intent_tests.rs"]
mod intent_tests;
