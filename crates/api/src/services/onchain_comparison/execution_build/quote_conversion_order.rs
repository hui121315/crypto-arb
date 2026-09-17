use shared_types::{
    plan_leg_sizing, plan_leg_sizing_for_base_quantity, OnchainCexOrderPlan,
    OnchainComparisonConfig, OnchainComparisonDirection, OnchainQuoteConversionEvidence,
    OnchainQuoteConversionOrderPlan, OnchainQuoteConversionSequence, OrderBookInfo, OrderSide,
    VenueInstrument,
};

#[derive(Clone, Copy)]
enum AmountTarget {
    Input(f64),
    Output(f64),
}

struct TargetRequest<'a> {
    config: &'a OnchainComparisonConfig,
    evidence: &'a OnchainQuoteConversionEvidence,
    instrument: &'a VenueInstrument,
    book: &'a OrderBookInfo,
    sequence: OnchainQuoteConversionSequence,
    from_asset: &'a str,
    to_asset: &'a str,
    target: AmountTarget,
    client_order_id: String,
}

pub(super) struct PlanRequest<'a> {
    pub(super) config: &'a OnchainComparisonConfig,
    pub(super) direction: OnchainComparisonDirection,
    pub(super) evidence: &'a OnchainQuoteConversionEvidence,
    pub(super) instrument: &'a VenueInstrument,
    pub(super) book: &'a OrderBookInfo,
    pub(super) primary_quote_amount: f64,
    pub(super) client_order_id: String,
}

pub(super) fn plan(request: PlanRequest<'_>) -> Result<OnchainQuoteConversionOrderPlan, String> {
    let PlanRequest {
        config,
        direction,
        evidence,
        instrument,
        book,
        primary_quote_amount,
        client_order_id,
    } = request;
    validate_identity(config, evidence, instrument, book)?;
    let fee_rate = (config.cex_taker_fee_bps.max(0.0) / 10_000.0).clamp(0.0, 0.25);
    let (sequence, from_asset, to_asset, target) = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => (
            OnchainQuoteConversionSequence::AfterPrimaryCex,
            evidence.cex_quote.as_str(),
            evidence.onchain_quote.as_str(),
            // Keep the next conversion's input-side fee inside the primary net proceeds.
            AmountTarget::Input(primary_quote_amount * (1.0 - fee_rate) / (1.0 + fee_rate)),
        ),
        OnchainComparisonDirection::BuyCexSellOnchain => (
            OnchainQuoteConversionSequence::BeforePrimaryCex,
            evidence.onchain_quote.as_str(),
            evidence.cex_quote.as_str(),
            AmountTarget::Output(primary_quote_amount * (1.0 + fee_rate) / (1.0 - fee_rate)),
        ),
    };
    plan_target(TargetRequest {
        config,
        evidence,
        instrument,
        book,
        sequence,
        from_asset,
        to_asset,
        target,
        client_order_id,
    })
}

fn plan_target(request: TargetRequest<'_>) -> Result<OnchainQuoteConversionOrderPlan, String> {
    let TargetRequest {
        config,
        evidence,
        instrument,
        book,
        sequence,
        from_asset,
        to_asset,
        target,
        client_order_id,
    } = request;
    let (market_base, market_quote) = crate::services::spot::split_spot_pair(&evidence.symbol)
        .ok_or_else(|| "Quote 换算交易对不是明确的 Base/Quote 格式".to_owned())?;
    let side = conversion_side(from_asset, to_asset, &market_base, &market_quote)?;
    let levels = match side {
        OrderSide::Buy => &book.asks,
        OrderSide::Sell => &book.bids,
    };
    let reference_price = match side {
        OrderSide::Buy => book.best_ask(),
        OrderSide::Sell => book.best_bid(),
    }
    .ok_or_else(|| "Quote 换算盘口缺少最优价".to_owned())?;
    let requested_base = match (side, target) {
        (OrderSide::Sell, AmountTarget::Input(amount))
        | (OrderSide::Buy, AmountTarget::Output(amount)) => amount,
        (OrderSide::Buy, AmountTarget::Input(amount))
        | (OrderSide::Sell, AmountTarget::Output(amount)) => {
            base_quantity_for_quote(levels, amount, reference_price, config.slippage_bps, side)?
        }
    };
    let must_reach_output = matches!(target, AmountTarget::Output(_));
    let requested_base = if must_reach_output {
        ceil_base_quantity(requested_base, instrument)?
    } else {
        requested_base
    };
    let sizing = if must_reach_output {
        plan_leg_sizing_for_base_quantity(requested_base, instrument, reference_price)
    } else {
        plan_leg_sizing(
            requested_base * reference_price,
            instrument,
            reference_price,
        )
    }
    .map_err(|block| format!("Quote 换算数量无法按官方步长构建：{}", block.code()))?;
    let quote_amount = super::consume_book(
        levels,
        sizing.rounded_base_qty,
        reference_price,
        config.slippage_bps,
        side,
    )?;
    let (planned_from_amount, planned_to_amount) = match side {
        OrderSide::Buy => (quote_amount, sizing.rounded_base_qty),
        OrderSide::Sell => (sizing.rounded_base_qty, quote_amount),
    };
    match target {
        AmountTarget::Input(limit) if planned_from_amount > limit * (1.0 + 1e-9) => {
            return Err("Quote 换算计划会超出可用输入资金".to_owned());
        }
        AmountTarget::Output(required) if planned_to_amount + 1e-9 < required => {
            return Err("Quote 换算计划按官方步长取整后仍不足以覆盖主订单".to_owned());
        }
        AmountTarget::Input(_) | AmountTarget::Output(_) => {}
    }
    Ok(OnchainQuoteConversionOrderPlan {
        sequence,
        from_asset: from_asset.to_owned(),
        to_asset: to_asset.to_owned(),
        planned_from_amount,
        planned_to_amount,
        order: OnchainCexOrderPlan {
            venue: config.cex_venue.clone(),
            native_symbol: instrument.native_symbol.clone(),
            client_order_id,
            side,
            base_quantity: sizing.rounded_base_qty,
            reference_price,
            estimated_quote_amount: quote_amount,
            instrument_spec: instrument.clone(),
            sizing_plan: sizing,
        },
    })
}

pub(super) fn replan(
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    original: &OnchainQuoteConversionOrderPlan,
    instrument: &VenueInstrument,
    book: &OrderBookInfo,
    primary_quote_amount: f64,
) -> Result<OnchainQuoteConversionOrderPlan, String> {
    let (cex_quote, onchain_quote) = match original.sequence {
        OnchainQuoteConversionSequence::BeforePrimaryCex => {
            (original.to_asset.clone(), original.from_asset.clone())
        }
        OnchainQuoteConversionSequence::AfterPrimaryCex => {
            (original.from_asset.clone(), original.to_asset.clone())
        }
    };
    let evidence = OnchainQuoteConversionEvidence {
        venue: original.order.venue.clone(),
        symbol: instrument.display_symbol.clone(),
        source: "ws_push".to_owned(),
        cex_quote,
        onchain_quote,
        source_bid: book.best_bid().unwrap_or_default(),
        source_ask: book.best_ask().unwrap_or_default(),
        cex_to_onchain_bid: 0.0,
        cex_to_onchain_ask: 0.0,
        cex_to_onchain_capacity: 0.0,
        onchain_to_cex_capacity: 0.0,
        freshness_ms: 0,
        observed_at_ms: book.timestamp,
    };
    let updated = plan(PlanRequest {
        config,
        direction,
        evidence: &evidence,
        instrument,
        book,
        primary_quote_amount,
        client_order_id: original.order.client_order_id.clone(),
    })?;
    let replanned = replan_after_fill(ReplanAfterFillRequest {
        config,
        direction,
        original: &updated,
        instrument,
        book,
        filled_from_amount: 0.0,
        filled_to_amount: 0.0,
        client_order_id: updated.order.client_order_id.clone(),
    })?
    .ok_or_else(|| "Quote 换汇计划没有剩余数量".to_owned())?;
    if replanned.sequence != original.sequence
        || !replanned
            .from_asset
            .eq_ignore_ascii_case(&original.from_asset)
        || !replanned.to_asset.eq_ignore_ascii_case(&original.to_asset)
    {
        return Err("提交前 Quote 换汇方向与原构建计划不一致".to_owned());
    }
    Ok(replanned)
}

pub(super) struct ReplanAfterFillRequest<'a> {
    pub(super) config: &'a OnchainComparisonConfig,
    pub(super) direction: OnchainComparisonDirection,
    pub(super) original: &'a OnchainQuoteConversionOrderPlan,
    pub(super) instrument: &'a VenueInstrument,
    pub(super) book: &'a OrderBookInfo,
    pub(super) filled_from_amount: f64,
    pub(super) filled_to_amount: f64,
    pub(super) client_order_id: String,
}

pub(super) fn replan_after_fill(
    request: ReplanAfterFillRequest<'_>,
) -> Result<Option<OnchainQuoteConversionOrderPlan>, String> {
    let ReplanAfterFillRequest {
        config,
        direction,
        original,
        instrument,
        book,
        filled_from_amount,
        filled_to_amount,
        client_order_id,
    } = request;
    let evidence = evidence_for_replan(original, book);
    validate_identity(config, &evidence, instrument, book)?;
    let expected_sequence = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            OnchainQuoteConversionSequence::AfterPrimaryCex
        }
        OnchainComparisonDirection::BuyCexSellOnchain => {
            OnchainQuoteConversionSequence::BeforePrimaryCex
        }
    };
    if original.sequence != expected_sequence {
        return Err("剩余 Quote 换汇顺序与套利方向不一致".to_owned());
    }
    if progress_complete(original, filled_from_amount, filled_to_amount)? {
        return Ok(None);
    }
    let target = match original.sequence {
        OnchainQuoteConversionSequence::AfterPrimaryCex => {
            AmountTarget::Input((original.planned_from_amount - filled_from_amount).max(0.0))
        }
        OnchainQuoteConversionSequence::BeforePrimaryCex => {
            AmountTarget::Output((original.planned_to_amount - filled_to_amount).max(0.0))
        }
    };
    let replanned = plan_target(TargetRequest {
        config,
        evidence: &evidence,
        instrument,
        book,
        sequence: original.sequence,
        from_asset: &original.from_asset,
        to_asset: &original.to_asset,
        target,
        client_order_id,
    })?;
    if filled_from_amount + replanned.planned_from_amount
        > original.planned_from_amount * (1.0 + 1e-9)
    {
        return Err("Quote 换汇补单会超出原计划的累计输入金额；需按新价格重新核验".into());
    }
    Ok(Some(replanned))
}

pub(super) fn progress_complete(
    original: &OnchainQuoteConversionOrderPlan,
    filled_from_amount: f64,
    filled_to_amount: f64,
) -> Result<bool, String> {
    if !original.planned_from_amount.is_finite()
        || original.planned_from_amount <= 0.0
        || !original.planned_to_amount.is_finite()
        || original.planned_to_amount <= 0.0
    {
        return Err("Quote 换汇原计划金额非法".into());
    }
    if !filled_from_amount.is_finite()
        || filled_from_amount < 0.0
        || filled_from_amount > original.planned_from_amount * (1.0 + 1e-9)
        || !filled_to_amount.is_finite()
        || filled_to_amount < 0.0
        || (filled_from_amount == 0.0) != (filled_to_amount == 0.0)
    {
        return Err("Quote 换汇累计成交金额非法".to_owned());
    }
    // A better buy price can fill the intended base quantity without spending the full budget.
    Ok(match (original.sequence, original.order.side) {
        (OnchainQuoteConversionSequence::BeforePrimaryCex, _)
        | (OnchainQuoteConversionSequence::AfterPrimaryCex, OrderSide::Buy) => {
            filled_to_amount >= original.planned_to_amount * (1.0 - 1e-9)
        }
        (OnchainQuoteConversionSequence::AfterPrimaryCex, OrderSide::Sell) => {
            filled_from_amount >= original.planned_from_amount * (1.0 - 1e-9)
        }
    })
}

fn evidence_for_replan(
    original: &OnchainQuoteConversionOrderPlan,
    book: &OrderBookInfo,
) -> OnchainQuoteConversionEvidence {
    let (cex_quote, onchain_quote) = match original.sequence {
        OnchainQuoteConversionSequence::BeforePrimaryCex => {
            (original.to_asset.clone(), original.from_asset.clone())
        }
        OnchainQuoteConversionSequence::AfterPrimaryCex => {
            (original.from_asset.clone(), original.to_asset.clone())
        }
    };
    OnchainQuoteConversionEvidence {
        venue: original.order.venue.clone(),
        symbol: original.order.instrument_spec.display_symbol.clone(),
        source: "ws_push".to_owned(),
        cex_quote,
        onchain_quote,
        source_bid: book.best_bid().unwrap_or_default(),
        source_ask: book.best_ask().unwrap_or_default(),
        cex_to_onchain_bid: 0.0,
        cex_to_onchain_ask: 0.0,
        cex_to_onchain_capacity: 0.0,
        onchain_to_cex_capacity: 0.0,
        freshness_ms: 0,
        observed_at_ms: book.timestamp,
    }
}

fn validate_identity(
    config: &OnchainComparisonConfig,
    evidence: &OnchainQuoteConversionEvidence,
    instrument: &VenueInstrument,
    book: &OrderBookInfo,
) -> Result<(), String> {
    if !evidence.venue.eq_ignore_ascii_case(&config.cex_venue)
        || !book.exchange.eq_ignore_ascii_case(&config.cex_venue)
        || !book.symbol.eq_ignore_ascii_case(&evidence.symbol)
        || !instrument
            .display_symbol
            .eq_ignore_ascii_case(&evidence.symbol)
    {
        return Err("Quote 换算的 venue、交易对、规格或盘口身份不一致".to_owned());
    }
    Ok(())
}

fn conversion_side(
    from_asset: &str,
    to_asset: &str,
    market_base: &str,
    market_quote: &str,
) -> Result<OrderSide, String> {
    if from_asset.eq_ignore_ascii_case(market_base) && to_asset.eq_ignore_ascii_case(market_quote) {
        Ok(OrderSide::Sell)
    } else if from_asset.eq_ignore_ascii_case(market_quote)
        && to_asset.eq_ignore_ascii_case(market_base)
    {
        Ok(OrderSide::Buy)
    } else {
        Err("Quote 换算资产方向与官方交易对不一致".to_owned())
    }
}

fn base_quantity_for_quote(
    levels: &[[f64; 2]],
    quote_target: f64,
    best_price: f64,
    slippage_bps: f64,
    side: OrderSide,
) -> Result<f64, String> {
    if !quote_target.is_finite() || quote_target <= 0.0 {
        return Err("Quote 换算目标金额非法".to_owned());
    }
    let tolerance = (slippage_bps / 10_000.0).clamp(0.0, 1.0);
    let limit = match side {
        OrderSide::Buy => best_price * (1.0 + tolerance),
        OrderSide::Sell => best_price * (1.0 - tolerance),
    };
    let mut remaining_quote = quote_target;
    let mut base = 0.0;
    for [price, available_base] in levels.iter().copied() {
        let valid =
            price.is_finite() && available_base.is_finite() && price > 0.0 && available_base > 0.0;
        let within_limit = match side {
            OrderSide::Buy => price <= limit,
            OrderSide::Sell => price >= limit,
        };
        if !valid || !within_limit {
            continue;
        }
        let available_quote = price * available_base;
        let take_quote = remaining_quote.min(available_quote);
        base += take_quote / price;
        remaining_quote -= take_quote;
        if remaining_quote <= quote_target * 1e-9 {
            return Ok(base);
        }
    }
    Err("Quote 换算盘口在价格保护范围内不足".to_owned())
}

fn ceil_base_quantity(quantity: f64, instrument: &VenueInstrument) -> Result<f64, String> {
    if !quantity.is_finite() || quantity <= 0.0 {
        return Err("Quote 换算目标数量非法".into());
    }
    let contract_size = instrument.contract_size.unwrap_or(1.0);
    let qty_step = instrument
        .qty_step
        .filter(|step| step.is_finite() && *step > 0.0)
        .ok_or_else(|| "Quote 换算规格缺少有效数量步长".to_owned())?;
    if !contract_size.is_finite() || contract_size <= 0.0 {
        return Err("Quote 换算规格缺少有效合约乘数".to_owned());
    }
    let base_step = qty_step * contract_size;
    let raw_steps = quantity / base_step;
    if !base_step.is_finite() || base_step <= 0.0 || !raw_steps.is_finite() {
        return Err("Quote 换算数量步长溢出".into());
    }
    // Match execution_sizing's floating-point tolerance; exact steps must not gain another lot.
    let tolerance = f64::EPSILON * 64.0 * raw_steps.abs().max(1.0);
    let rounded = (raw_steps - tolerance).ceil() * base_step;
    if !rounded.is_finite() || rounded <= 0.0 {
        return Err("Quote 换算取整后的数量非法".into());
    }
    Ok(rounded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{InstrumentAssetClass, InstrumentListingStatus, InstrumentMetadataSource};

    fn config() -> OnchainComparisonConfig {
        OnchainComparisonConfig {
            cex_venue: "kraken".to_owned(),
            cex_symbol: "PUPS/USD".to_owned(),
            cex_taker_fee_bps: 10.0,
            slippage_bps: 20.0,
            ..OnchainComparisonConfig::default()
        }
    }

    fn evidence() -> OnchainQuoteConversionEvidence {
        OnchainQuoteConversionEvidence {
            venue: "kraken".to_owned(),
            symbol: "USDC/USD".to_owned(),
            source: "ws_push".to_owned(),
            cex_quote: "USD".to_owned(),
            onchain_quote: "USDC".to_owned(),
            source_bid: 0.999,
            source_ask: 1.001,
            cex_to_onchain_bid: 1.0 / 1.001,
            cex_to_onchain_ask: 1.0 / 0.999,
            cex_to_onchain_capacity: 10_000.0,
            onchain_to_cex_capacity: 10_000.0,
            freshness_ms: 1,
            observed_at_ms: 1,
        }
    }

    fn instrument() -> VenueInstrument {
        VenueInstrument {
            venue: "kraken".to_owned(),
            native_symbol: "USDCUSD".to_owned(),
            canonical_symbol: "USDC".to_owned(),
            display_symbol: "USDC/USD".to_owned(),
            asset_class: InstrumentAssetClass::Crypto,
            product_type: Some("spot".to_owned()),
            quote_asset: Some("USD".to_owned()),
            settle_asset: None,
            margin_asset: None,
            contract_size: Some(1.0),
            execution_supported: true,
            price_tick: Some(0.0001),
            qty_step: Some(0.01),
            min_qty: Some(0.01),
            min_notional: Some(1.0),
            listing_status: InstrumentListingStatus::Trading,
            funding_interval_ms: None,
            builder_dex: None,
            source: InstrumentMetadataSource::OfficialEndpoint,
            source_url: Some("https://api.kraken.com/0/public/AssetPairs".to_owned()),
            checked_at_ms: 1,
            schema_version: Some("kraken-spot-v1".to_owned()),
        }
    }

    fn book() -> OrderBookInfo {
        OrderBookInfo {
            symbol: "USDC/USD".to_owned(),
            exchange: "kraken".to_owned(),
            bids: vec![[0.999, 1_000.0]],
            asks: vec![[1.001, 1_000.0]],
            timestamp: 1,
        }
    }

    #[test]
    fn compiled_conversion_economics_account_for_unspent_fee_reserves_in_both_directions() {
        use super::super::executable_economics;
        use onchain_monitor::{compare_quotes, ProviderQuote, QuoteInputs};

        let mut config = config();
        config.base_decimals = 0;
        config.quote_decimals = 0;
        config.gas_usd = 0.01;
        let mut instrument = instrument();
        instrument.qty_step = Some(0.00000001);
        instrument.min_qty = Some(0.00000001);

        for rate in [0.5, 0.999, 1.0, 1.001, 2.0] {
            let usd_rate = 1.0 / rate;
            let mut book = book();
            book.bids = vec![[1.0 / rate, 1_000.0]];
            book.asks = book.bids.clone();
            let rows = compare_quotes(QuoteInputs {
                onchain_sell_price: 100.0,
                onchain_buy_price: 100.0,
                cex_bid: 100.0 * rate,
                cex_ask: 100.0 * rate,
                cex_fee_bps: config.cex_taker_fee_bps,
                quote_conversion_fee_bps: config.cex_taker_fee_bps,
                slippage_bps: config.slippage_bps,
                gas_usd: config.gas_usd,
                buy_onchain_sell_cex_cost_notional_usd: 100.0 * usd_rate,
                buy_onchain_sell_cex_observable_notional_usd: 100.0 * usd_rate,
                buy_cex_sell_onchain_cost_notional_usd: 100.0 * rate * usd_rate,
                buy_cex_sell_onchain_observable_notional_usd: 100.0 * rate * usd_rate,
            })
            .unwrap();

            for row in rows {
                let conversion = plan(PlanRequest {
                    config: &config,
                    direction: row.direction,
                    evidence: &evidence(),
                    instrument: &instrument,
                    book: &book,
                    primary_quote_amount: 100.0,
                    client_order_id: "economics-conversion".to_owned(),
                })
                .expect("real conversion compiler");
                let mut primary = conversion.order.clone();
                primary.base_quantity = 1.0;
                primary.estimated_quote_amount = 100.0;
                let buy_chain = row.direction == OnchainComparisonDirection::BuyOnchainSellCex;
                let quote = ProviderQuote {
                    input_address: if buy_chain { "quote" } else { "base" }.to_owned(),
                    output_address: if buy_chain { "base" } else { "quote" }.to_owned(),
                    input_amount_raw: if buy_chain { "100" } else { "1" }.to_owned(),
                    output_amount_raw: if buy_chain { "1" } else { "100" }.to_owned(),
                    router: None,
                };
                let result = executable_economics(
                    &config,
                    row.direction,
                    &quote,
                    &primary,
                    Some(&conversion),
                    usd_rate,
                )
                .expect("firm build economics");
                let expected = row.observable_notional_usd * row.net_spread_bps / 10_000.0;
                // Monitoring assumes all proceeds are converted. The compiled lot leaves a
                // reserve in the source asset, so that unconverted amount incurs no swap fee.
                let fee_rate = config.cex_taker_fee_bps / 10_000.0;
                let saved_conversion_fee = if buy_chain {
                    let retained = 100.0 * (1.0 - fee_rate) - conversion.planned_from_amount;
                    retained
                        * (conversion.planned_to_amount / conversion.planned_from_amount)
                        * fee_rate
                        * usd_rate
                } else {
                    0.0
                };
                assert!(
                    (result.net_profit_usd - expected - saved_conversion_fee).abs() < 1e-6,
                    "{:?}, rate={rate}, build={}, monitor={expected}",
                    row.direction,
                    result.net_profit_usd
                );

                for bad_amount in [0.0, -1.0, f64::NAN, f64::INFINITY] {
                    let mut invalid = conversion.clone();
                    invalid.planned_from_amount = bad_amount;
                    assert!(executable_economics(
                        &config,
                        row.direction,
                        &quote,
                        &primary,
                        Some(&invalid),
                        usd_rate,
                    )
                    .is_err());
                }
            }
        }
    }

    #[test]
    fn inverse_pair_builds_after_primary_buy_without_overspending_proceeds() {
        let config = config();
        let evidence = evidence();
        let instrument = instrument();
        let book = book();
        let plan = plan(PlanRequest {
            config: &config,
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            evidence: &evidence,
            instrument: &instrument,
            book: &book,
            primary_quote_amount: 100.0,
            client_order_id: "conversion".to_owned(),
        })
        .expect("USD proceeds should buy USDC");
        assert_eq!(
            plan.sequence,
            OnchainQuoteConversionSequence::AfterPrimaryCex
        );
        assert_eq!(plan.order.side, OrderSide::Buy);
        assert!(
            plan.planned_from_amount * (1.0 + config.cex_taker_fee_bps / 10_000.0) <= 99.9 + 1e-9
        );
        assert!(plan.planned_to_amount > 0.0);
    }

    #[test]
    fn inverse_pair_builds_before_primary_sell_that_covers_required_quote() {
        let config = config();
        let evidence = evidence();
        let instrument = instrument();
        let book = book();
        let plan = plan(PlanRequest {
            config: &config,
            direction: OnchainComparisonDirection::BuyCexSellOnchain,
            evidence: &evidence,
            instrument: &instrument,
            book: &book,
            primary_quote_amount: 100.0,
            client_order_id: "conversion".to_owned(),
        })
        .expect("USDC should sell into enough USD for the primary buy");
        assert_eq!(
            plan.sequence,
            OnchainQuoteConversionSequence::BeforePrimaryCex
        );
        assert_eq!(plan.order.side, OrderSide::Sell);
        assert!(plan.planned_to_amount >= 100.0 * 1.001 / 0.999);
    }

    #[test]
    fn replan_preserves_identity_but_uses_the_latest_book() {
        let config = config();
        let evidence = evidence();
        let instrument = instrument();
        let original_book = book();
        let original = plan(PlanRequest {
            config: &config,
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            evidence: &evidence,
            instrument: &instrument,
            book: &original_book,
            primary_quote_amount: 100.0,
            client_order_id: "stable-client-id".to_owned(),
        })
        .expect("original plan");
        let mut latest_book = original_book;
        latest_book.asks = vec![[1.002, 1_000.0]];
        latest_book.timestamp = 2;
        let latest = replan(
            &config,
            OnchainComparisonDirection::BuyOnchainSellCex,
            &original,
            &instrument,
            &latest_book,
            100.0,
        )
        .expect("latest plan");
        assert_eq!(latest.order.client_order_id, "stable-client-id");
        assert_eq!(latest.order.reference_price, 1.002);
        assert_eq!(latest.sequence, original.sequence);
        assert_eq!(latest.from_asset, original.from_asset);
        assert_eq!(latest.to_asset, original.to_asset);
    }

    #[test]
    fn post_conversion_retry_only_uses_the_unspent_input() {
        let config = config();
        let evidence = evidence();
        let instrument = instrument();
        let book = book();
        let original = plan(PlanRequest {
            config: &config,
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            evidence: &evidence,
            instrument: &instrument,
            book: &book,
            primary_quote_amount: 100.0,
            client_order_id: "first".to_owned(),
        })
        .expect("original");
        let filled_from = original.planned_from_amount / 2.0;
        let filled_to = original.planned_to_amount / 2.0;
        let retry = replan_after_fill(ReplanAfterFillRequest {
            config: &config,
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            original: &original,
            instrument: &instrument,
            book: &book,
            filled_from_amount: filled_from,
            filled_to_amount: filled_to,
            client_order_id: "retry".to_owned(),
        })
        .expect("retry plan")
        .expect("remaining input");
        assert!(retry.planned_from_amount < original.planned_from_amount);
        assert!(retry.planned_from_amount <= original.planned_from_amount / 2.0 + 0.02);
        assert_eq!(retry.order.client_order_id, "retry");
    }

    #[test]
    fn pre_conversion_retry_only_targets_the_missing_output() {
        let config = config();
        let evidence = evidence();
        let instrument = instrument();
        let book = book();
        let original = plan(PlanRequest {
            config: &config,
            direction: OnchainComparisonDirection::BuyCexSellOnchain,
            evidence: &evidence,
            instrument: &instrument,
            book: &book,
            primary_quote_amount: 100.0,
            client_order_id: "first".to_owned(),
        })
        .expect("original");
        let filled_from = (original.planned_from_amount / 2.0 * 100.0).floor() / 100.0;
        let filled_to = filled_from * 0.999;
        let retry = replan_after_fill(ReplanAfterFillRequest {
            config: &config,
            direction: OnchainComparisonDirection::BuyCexSellOnchain,
            original: &original,
            instrument: &instrument,
            book: &book,
            filled_from_amount: filled_from,
            filled_to_amount: filled_to,
            client_order_id: "retry".to_owned(),
        })
        .expect("retry plan")
        .expect("missing output");
        assert!(retry.planned_to_amount < original.planned_to_amount);
        assert!(retry.planned_to_amount >= original.planned_to_amount / 2.0 - 0.02);
    }

    fn original(direction: OnchainComparisonDirection) -> OnchainQuoteConversionOrderPlan {
        plan(PlanRequest {
            config: &config(),
            direction,
            evidence: &evidence(),
            instrument: &instrument(),
            book: &book(),
            primary_quote_amount: 100.0,
            client_order_id: "original".into(),
        })
        .unwrap()
    }

    #[test]
    fn residual_conversion_stops_before_exceeding_the_original_total_input() {
        let direction = OnchainComparisonDirection::BuyCexSellOnchain;
        let original = original(direction);
        let filled_from = 50.0;
        let filled_to = filled_from * 0.999;
        let mut latest = book();
        latest.bids = vec![[0.98, 1_000.0]];
        let error = replan_after_fill(ReplanAfterFillRequest {
            config: &config(),
            direction,
            original: &original,
            instrument: &instrument(),
            book: &latest,
            filled_from_amount: filled_from,
            filled_to_amount: filled_to,
            client_order_id: "retry".into(),
        })
        .unwrap_err();
        assert!(error.contains("累计输入金额"));

        latest.bids = vec![[1.001, 1_000.0]];
        let better = replan_after_fill(ReplanAfterFillRequest {
            config: &config(),
            direction,
            original: &original,
            instrument: &instrument(),
            book: &latest,
            filled_from_amount: filled_from,
            filled_to_amount: filled_to,
            client_order_id: "retry".into(),
        })
        .unwrap()
        .unwrap();
        assert!(filled_from + better.planned_from_amount <= original.planned_from_amount);
    }

    #[test]
    fn favorable_conversion_overdelivery_is_not_an_invalid_receipt() {
        let direction = OnchainComparisonDirection::BuyCexSellOnchain;
        let original = original(direction);
        let complete = replan_after_fill(ReplanAfterFillRequest {
            config: &config(),
            direction,
            original: &original,
            instrument: &instrument(),
            book: &book(),
            filled_from_amount: original.planned_from_amount * 0.99,
            filled_to_amount: original.planned_to_amount * 1.01,
            client_order_id: "unused".into(),
        })
        .unwrap();
        assert!(complete.is_none());
    }

    #[test]
    fn favorable_full_buy_does_not_submit_another_order_to_spend_saved_change() {
        let original = original(OnchainComparisonDirection::BuyOnchainSellCex);
        assert_eq!(original.order.side, OrderSide::Buy);
        assert!(progress_complete(
            &original,
            original.planned_from_amount * 0.99,
            original.planned_to_amount
        )
        .unwrap());
        assert!(!progress_complete(
            &original,
            original.planned_from_amount * 0.5,
            original.planned_to_amount * 0.5
        )
        .unwrap());
    }

    #[test]
    fn conversion_progress_rejects_invalid_plans_and_cumulative_cashflows() {
        let original = original(OnchainComparisonDirection::BuyCexSellOnchain);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut invalid = original.clone();
            invalid.planned_from_amount = bad;
            assert!(progress_complete(&invalid, 0.0, 0.0).is_err());
            invalid = original.clone();
            invalid.planned_to_amount = bad;
            assert!(progress_complete(&invalid, 0.0, 0.0).is_err());
        }
        for (from, to) in [
            (f64::NAN, 1.0),
            (1.0, f64::INFINITY),
            (-1.0, 1.0),
            (0.0, 1.0),
            (1.0, 0.0),
            (
                original.planned_from_amount * 1.01,
                original.planned_to_amount,
            ),
        ] {
            assert!(progress_complete(&original, from, to).is_err());
        }
        assert!(!progress_complete(&original, 0.0, 0.0).unwrap());
    }

    #[test]
    fn completed_conversion_cannot_bypass_the_original_direction_or_market() {
        let original = original(OnchainComparisonDirection::BuyCexSellOnchain);
        let mut wrong_book = book();
        wrong_book.exchange = "binance".into();
        for (direction, book) in [
            (OnchainComparisonDirection::BuyOnchainSellCex, book()),
            (OnchainComparisonDirection::BuyCexSellOnchain, wrong_book),
        ] {
            assert!(replan_after_fill(ReplanAfterFillRequest {
                config: &config(),
                direction,
                original: &original,
                instrument: &instrument(),
                book: &book,
                filled_from_amount: original.planned_from_amount,
                filled_to_amount: original.planned_to_amount,
                client_order_id: "unused".into(),
            })
            .is_err());
        }
    }

    #[test]
    fn conversion_rounding_does_not_buy_an_extra_lot_for_float_noise() {
        assert!(
            (ceil_base_quantity(50.160000000000004, &instrument()).unwrap() - 50.16).abs() < 1e-12
        );
        assert!((ceil_base_quantity(50.1600001, &instrument()).unwrap() - 50.17).abs() < 1e-12);
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(ceil_base_quantity(invalid, &instrument()).is_err());
        }
    }
}
