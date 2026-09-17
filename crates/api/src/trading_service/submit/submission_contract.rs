use super::*;

pub(super) fn compile_submission_intent(
    mut intent: OrderIntent,
    context: &OrderSubmissionContext,
) -> TradingResult<OrderIntent> {
    match (&context.instrument_spec, context.sizing_plan) {
        (None, None) => Ok(intent),
        (Some(instrument), Some(sizing)) => {
            validate_product(context.product, instrument, &intent)?;
            validate_contract(&intent, instrument, sizing)?;
            intent.symbol.clone_from(&instrument.native_symbol);
            intent.quantity = sizing.rounded_base_qty;
            Ok(intent)
        }
        _ => Err(contract_error(
            &intent,
            "instrument spec and sizing plan must be supplied together",
        )),
    }
}

fn validate_product(
    product: shared_types::FeeProduct,
    instrument: &shared_types::InstrumentSpec,
    intent: &OrderIntent,
) -> TradingResult<()> {
    let actual = instrument.product_type.as_deref().unwrap_or_default();
    let matches = match product {
        shared_types::FeeProduct::Spot => actual.eq_ignore_ascii_case("spot"),
        shared_types::FeeProduct::Perp => {
            actual.eq_ignore_ascii_case("perp") || actual.eq_ignore_ascii_case("perpetual")
        }
        shared_types::FeeProduct::Margin | shared_types::FeeProduct::Unknown => false,
    };
    if matches {
        Ok(())
    } else {
        Err(contract_error(
            intent,
            &format!("PRODUCT_ROUTE_MISMATCH expected={product:?} instrument={actual:?}"),
        ))
    }
}

fn validate_contract(
    intent: &OrderIntent,
    instrument: &shared_types::InstrumentSpec,
    sizing: shared_types::OrderSizingPlan,
) -> TradingResult<()> {
    shared_types::validate_order_sizing_contract(
        &intent.exchange,
        &intent.symbol,
        instrument,
        sizing,
    )
    .map_err(|error| contract_error(intent, error.code()))
}

fn contract_error(intent: &OrderIntent, message: &str) -> TradingError {
    exchange::ExchangeError::Api {
        exchange: intent.exchange.clone(),
        code: "INSTRUMENT_SIZING_CONTRACT_INVALID".to_owned(),
        message: format!("{} {}: {message}", intent.exchange, intent.symbol),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        InstrumentAssetClass, InstrumentListingStatus, InstrumentMetadataSource, MarginMode,
        OrderSide, OrderSource, OrderType, TimeInForce,
    };

    #[test]
    fn ticket_contract_compiles_native_symbol_and_rounded_quantity() -> Result<(), String> {
        let instrument = instrument();
        let sizing = shared_types::plan_leg_sizing(1_234.0, &instrument, 30_000.0)
            .map_err(|error| format!("{error:?}"))?;
        let context = OrderSubmissionContext {
            product: shared_types::FeeProduct::Perp,
            instrument_spec: Some(instrument),
            sizing_plan: Some(sizing),
            ..OrderSubmissionContext::default()
        };

        let compiled =
            compile_submission_intent(intent(), &context).map_err(|error| error.to_string())?;

        assert_eq!(compiled.symbol, "BTCUSDC");
        assert_eq!(compiled.quantity, 0.041);
        Ok(())
    }

    #[test]
    fn ticket_contract_rejects_tampered_sizing() -> Result<(), String> {
        let instrument = instrument();
        let mut sizing = shared_types::plan_leg_sizing(1_234.0, &instrument, 30_000.0)
            .map_err(|error| format!("{error:?}"))?;
        sizing.rounded_base_qty = 99.0;
        let context = OrderSubmissionContext {
            product: shared_types::FeeProduct::Perp,
            instrument_spec: Some(instrument),
            sizing_plan: Some(sizing),
            ..OrderSubmissionContext::default()
        };

        let Err(error) = compile_submission_intent(intent(), &context) else {
            return Err("tampered sizing contract unexpectedly compiled".to_owned());
        };

        assert!(error.to_string().contains("ORDER_SIZING_PLAN_MISMATCH"));
        Ok(())
    }

    #[test]
    fn ticket_contract_rejects_partial_context() -> Result<(), String> {
        let context = OrderSubmissionContext {
            instrument_spec: Some(instrument()),
            sizing_plan: None,
            ..OrderSubmissionContext::default()
        };

        let Err(error) = compile_submission_intent(intent(), &context) else {
            return Err("partial sizing context unexpectedly compiled".to_owned());
        };

        assert!(error.to_string().contains("must be supplied together"));
        Ok(())
    }

    fn instrument() -> shared_types::InstrumentSpec {
        shared_types::InstrumentSpec {
            venue: "binance".to_owned(),
            native_symbol: "BTCUSDC".to_owned(),
            canonical_symbol: "BTC".to_owned(),
            display_symbol: "BTC-USDC Perp".to_owned(),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("perp".to_owned()),
            quote_asset: Some("USDC".to_owned()),
            settle_asset: Some("USDC".to_owned()),
            margin_asset: Some("USDC".to_owned()),
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(0.1),
            qty_step: Some(0.001),
            min_qty: Some(0.001),
            min_notional: Some(5.0),
            listing_status: InstrumentListingStatus::Trading,
            funding_interval_ms: Some(28_800_000),
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some("/fapi/v1/exchangeInfo".to_owned()),
            checked_at_ms: 1_700_000_000_000,
            schema_version: Some("binance-usdm-v1".to_owned()),
        }
    }

    fn intent() -> OrderIntent {
        OrderIntent {
            id: "order-1".to_owned(),
            source: OrderSource::ArbitragePreview,
            strategy: None,
            mode: ExecutionMode::Live,
            exchange: "binance".to_owned(),
            symbol: "BTC".to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 1_234.0 / 30_000.0,
            price: Some(30_000.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Gtc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-1".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        }
    }
}
