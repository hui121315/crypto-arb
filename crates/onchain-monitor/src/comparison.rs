use shared_types::{OnchainCexComparison, OnchainComparisonDirection, OnchainComparisonQuality};

#[derive(Debug, Clone, Copy)]
pub struct QuoteInputs {
    pub onchain_sell_price: f64,
    pub onchain_buy_price: f64,
    pub cex_bid: f64,
    pub cex_ask: f64,
    pub cex_fee_bps: f64,
    pub quote_conversion_fee_bps: f64,
    pub slippage_bps: f64,
    pub gas_usd: f64,
    pub buy_onchain_sell_cex_cost_notional_usd: f64,
    pub buy_onchain_sell_cex_observable_notional_usd: f64,
    pub buy_cex_sell_onchain_cost_notional_usd: f64,
    pub buy_cex_sell_onchain_observable_notional_usd: f64,
}

pub fn compare_quotes(input: QuoteInputs) -> Option<Vec<OnchainCexComparison>> {
    Some(vec![
        comparison(
            OnchainComparisonDirection::BuyOnchainSellCex,
            input.onchain_buy_price,
            input.cex_bid,
            input,
            input.buy_onchain_sell_cex_cost_notional_usd,
            input.buy_onchain_sell_cex_observable_notional_usd,
        )?,
        comparison(
            OnchainComparisonDirection::BuyCexSellOnchain,
            input.onchain_sell_price,
            input.cex_ask,
            input,
            input.buy_cex_sell_onchain_cost_notional_usd,
            input.buy_cex_sell_onchain_observable_notional_usd,
        )?,
    ])
}

pub fn classify_quality(
    age_ms: i64,
    max_age_ms: i64,
    min_liquidity_usd: f64,
    min_net_spread_bps: f64,
    rows: &[OnchainCexComparison],
) -> OnchainComparisonQuality {
    let min_net_spread_bps = min_net_spread_bps.max(0.0);
    if age_ms > max_age_ms {
        OnchainComparisonQuality::Stale
    } else if rows
        .iter()
        .all(|row| row.net_spread_bps <= 0.0 || row.net_spread_bps < min_net_spread_bps)
    {
        OnchainComparisonQuality::NoNetProfit
    } else if rows.iter().any(|row| {
        row.net_spread_bps > 0.0
            && row.net_spread_bps >= min_net_spread_bps
            && row.observable_notional_usd.is_finite()
            && row.observable_notional_usd >= min_liquidity_usd
    }) {
        OnchainComparisonQuality::Fresh
    } else {
        OnchainComparisonQuality::LowLiquidity
    }
}

fn comparison(
    direction: OnchainComparisonDirection,
    onchain_price: f64,
    cex_price: f64,
    input: QuoteInputs,
    cost_notional_usd: f64,
    observable_notional_usd: f64,
) -> Option<OnchainCexComparison> {
    let valid_price = onchain_price.is_finite()
        && onchain_price > 0.0
        && cex_price.is_finite()
        && cex_price > 0.0;
    let valid_costs = input.cex_fee_bps.is_finite()
        && (0.0..10_000.0).contains(&input.cex_fee_bps)
        && input.quote_conversion_fee_bps.is_finite()
        && (0.0..10_000.0).contains(&input.quote_conversion_fee_bps)
        && input.slippage_bps.is_finite()
        && (0.0..=10_000.0).contains(&input.slippage_bps)
        && input.gas_usd.is_finite()
        && input.gas_usd >= 0.0;
    let valid_notional = cost_notional_usd.is_finite()
        && cost_notional_usd > 0.0
        && observable_notional_usd.is_finite()
        && observable_notional_usd >= 0.0;
    if !(valid_price && valid_costs && valid_notional) {
        return None;
    }
    let gross_spread_bps = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            (cex_price / onchain_price - 1.0) * 10_000.0
        }
        OnchainComparisonDirection::BuyCexSellOnchain => {
            (onchain_price / cex_price - 1.0) * 10_000.0
        }
    };
    let cex_fee_rate = input.cex_fee_bps / 10_000.0;
    let conversion_fee_rate = input.quote_conversion_fee_bps / 10_000.0;
    let (fee_cost_ratio, cex_notional_ratio) = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            let proceeds_ratio = cex_price / onchain_price;
            // Sell fees consume proceeds; the conversion trades only what remains.
            let fee_ratio = cex_fee_rate + (1.0 - cex_fee_rate) * conversion_fee_rate;
            (proceeds_ratio * fee_ratio, proceeds_ratio)
        }
        OnchainComparisonDirection::BuyCexSellOnchain => {
            // Fund the primary buy plus its fee after paying the conversion fee.
            (
                (cex_fee_rate + conversion_fee_rate) / (1.0 - conversion_fee_rate),
                1.0,
            )
        }
    };
    let observable_notional_usd = observable_notional_usd.min(cost_notional_usd);
    let gas_notional = if observable_notional_usd > 0.0 {
        observable_notional_usd
    } else {
        cost_notional_usd
    };
    // A smaller observable trade still pays the whole transaction's fixed gas cost.
    let gas_bps = input.gas_usd / gas_notional * 10_000.0;
    let total_cost_bps =
        fee_cost_ratio * 10_000.0 + cex_notional_ratio * input.slippage_bps + gas_bps;
    let net_spread_bps = gross_spread_bps - total_cost_bps;
    if ![gross_spread_bps, gas_bps, total_cost_bps, net_spread_bps]
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }
    Some(OnchainCexComparison {
        direction,
        onchain_price,
        cex_price,
        gross_spread_bps,
        cex_fee_bps: input.cex_fee_bps,
        quote_conversion_fee_bps: input.quote_conversion_fee_bps,
        slippage_bps: input.slippage_bps,
        gas_usd: input.gas_usd,
        gas_bps,
        total_cost_bps,
        net_spread_bps,
        observable_notional_usd,
        executable: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cashflow_input() -> QuoteInputs {
        QuoteInputs {
            onchain_sell_price: 100.0,
            onchain_buy_price: 100.0,
            cex_bid: 100.0,
            cex_ask: 100.0,
            cex_fee_bps: 0.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 0.0,
            gas_usd: 0.0,
            buy_onchain_sell_cex_cost_notional_usd: 100.0,
            buy_onchain_sell_cex_observable_notional_usd: 100.0,
            buy_cex_sell_onchain_cost_notional_usd: 100.0,
            buy_cex_sell_onchain_observable_notional_usd: 100.0,
        }
    }

    #[test]
    fn sequential_fees_reject_profit_that_linear_subtraction_would_accept() {
        let rows = compare_quotes(QuoteInputs {
            onchain_sell_price: 102.01,
            cex_bid: 102.01,
            cex_fee_bps: 100.0,
            quote_conversion_fee_bps: 100.0,
            ..cashflow_input()
        })
        .expect("valid fee-bearing round trip");
        let expected = [102.01 * 0.99 * 0.99 - 100.0, 102.01 - 100.0 * 1.01 / 0.99];
        for (row, profit) in rows.iter().zip(expected) {
            assert!(row.gross_spread_bps > 200.0);
            assert!((row.net_spread_bps - profit * 100.0).abs() < 1e-9);
            assert!(row.net_spread_bps < 0.0);
        }
        assert_eq!(
            classify_quality(0, 1_000, 1.0, 0.0, &rows),
            OnchainComparisonQuality::NoNetProfit
        );
    }

    #[test]
    fn sale_fees_and_slippage_reserve_follow_sale_proceeds_not_entry_cost() {
        let row = compare_quotes(QuoteInputs {
            cex_bid: 120.0,
            cex_fee_bps: 100.0,
            slippage_bps: 50.0,
            ..cashflow_input()
        })
        .unwrap()
        .remove(0);
        let expected_profit = 120.0 - 100.0 - 1.2 - 0.6;
        assert!((row.net_spread_bps - expected_profit * 100.0).abs() < 1e-9);
    }

    #[test]
    fn smaller_observable_trade_still_pays_full_gas() {
        let input = QuoteInputs {
            onchain_sell_price: 101.0,
            cex_bid: 101.0,
            gas_usd: 0.2,
            ..cashflow_input()
        };
        let full = compare_quotes(input).unwrap();
        let small = compare_quotes(QuoteInputs {
            buy_onchain_sell_cex_observable_notional_usd: 10.0,
            buy_cex_sell_onchain_observable_notional_usd: 10.0,
            ..input
        })
        .unwrap();
        for (full, small) in full.iter().zip(small.iter()) {
            assert!(full.net_spread_bps > 0.0);
            let profit = small.observable_notional_usd * small.net_spread_bps / 10_000.0;
            assert!((profit - (0.1 - 0.2)).abs() < 1e-9);
            assert!(small.net_spread_bps < 0.0);
        }
    }

    #[test]
    fn invalid_fee_and_arithmetic_overflow_never_produce_profit_rows() {
        for fee in [f64::NAN, f64::INFINITY, -1.0, 10_000.0, 20_000.0] {
            assert!(compare_quotes(QuoteInputs {
                cex_fee_bps: fee,
                ..cashflow_input()
            })
            .is_none());
            assert!(compare_quotes(QuoteInputs {
                quote_conversion_fee_bps: fee,
                ..cashflow_input()
            })
            .is_none());
        }
        assert!(compare_quotes(QuoteInputs {
            onchain_buy_price: f64::MIN_POSITIVE,
            cex_bid: f64::MAX,
            ..cashflow_input()
        })
        .is_none());
    }

    #[test]
    fn costs_can_remove_apparent_profit_and_output_stays_observational() {
        let rows = compare_quotes(QuoteInputs {
            onchain_sell_price: 101.0,
            onchain_buy_price: 99.0,
            cex_bid: 100.0,
            cex_ask: 100.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 20.0,
            gas_usd: 1.0,
            buy_onchain_sell_cex_cost_notional_usd: 100.0,
            buy_onchain_sell_cex_observable_notional_usd: 100.0,
            buy_cex_sell_onchain_cost_notional_usd: 250.0,
            buy_cex_sell_onchain_observable_notional_usd: 250.0,
        })
        .expect("valid inputs should produce comparison rows");
        assert!(rows.iter().all(|row| !row.executable));
        assert!(rows
            .iter()
            .all(|row| row.net_spread_bps < row.gross_spread_bps));
        assert_eq!(rows[0].observable_notional_usd, 100.0);
        assert_eq!(rows[1].observable_notional_usd, 250.0);
    }

    #[test]
    fn missing_bbo_size_never_turns_gas_into_an_impossible_spread() {
        let rows = compare_quotes(QuoteInputs {
            onchain_sell_price: 101.0,
            onchain_buy_price: 99.0,
            cex_bid: 100.0,
            cex_ask: 100.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 10.0,
            gas_usd: 0.01,
            buy_onchain_sell_cex_cost_notional_usd: 100.0,
            buy_onchain_sell_cex_observable_notional_usd: 0.0,
            buy_cex_sell_onchain_cost_notional_usd: 100.0,
            buy_cex_sell_onchain_observable_notional_usd: 0.0,
        })
        .expect("zero observable depth still has a valid cost notional");

        assert!(rows.iter().all(|row| row.gas_bps == 1.0));
        assert!(rows.iter().all(|row| row.net_spread_bps.is_finite()));
        assert!(rows.iter().all(|row| row.observable_notional_usd == 0.0));
        assert_eq!(
            classify_quality(1, 10, 100.0, 20.0, &rows),
            OnchainComparisonQuality::LowLiquidity
        );
    }

    #[test]
    fn quality_matrix_orders_freshness_liquidity_and_profit_gates() {
        let profitable = vec![OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyCexSellOnchain,
            onchain_price: 101.0,
            cex_price: 100.0,
            gross_spread_bps: 100.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 5.0,
            gas_usd: 0.5,
            gas_bps: 5.0,
            total_cost_bps: 20.0,
            net_spread_bps: 80.0,
            observable_notional_usd: 1_000.0,
            executable: false,
        }];
        assert_eq!(
            classify_quality(11, 10, 100.0, 20.0, &profitable),
            OnchainComparisonQuality::Stale
        );
        let mut shallow_profitable = profitable.clone();
        shallow_profitable[0].observable_notional_usd = 50.0;
        assert_eq!(
            classify_quality(1, 10, 100.0, 20.0, &shallow_profitable),
            OnchainComparisonQuality::LowLiquidity
        );
        let mut unprofitable = profitable.clone();
        unprofitable[0].net_spread_bps = -1.0;
        assert_eq!(
            classify_quality(1, 10, 100.0, 20.0, &unprofitable),
            OnchainComparisonQuality::NoNetProfit
        );
        assert_eq!(
            classify_quality(1, 10, 100.0, 20.0, &profitable),
            OnchainComparisonQuality::Fresh
        );

        let mut mixed = shallow_profitable;
        let mut deep_unprofitable = profitable[0].clone();
        deep_unprofitable.net_spread_bps = -1.0;
        mixed.push(deep_unprofitable);
        assert_eq!(
            classify_quality(1, 10, 100.0, 20.0, &mixed),
            OnchainComparisonQuality::LowLiquidity
        );

        let mut below_threshold = profitable;
        below_threshold[0].net_spread_bps = 19.99;
        assert_eq!(
            classify_quality(1, 10, 100.0, 20.0, &below_threshold),
            OnchainComparisonQuality::NoNetProfit
        );
        below_threshold[0].net_spread_bps = 0.0;
        assert_eq!(
            classify_quality(1, 10, 100.0, 0.0, &below_threshold),
            OnchainComparisonQuality::NoNetProfit
        );
    }
}
