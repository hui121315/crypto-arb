use crate::state::AppState;
use rust_decimal::{
    prelude::{FromPrimitive, ToPrimitive},
    Decimal,
};
use shared_types::{
    OnchainReplenishmentCostValuation, OnchainReplenishmentNetworkCost, OnchainReplenishmentRun,
    OnchainReplenishmentWithdrawalCost, OnchainUsdValuation,
};
use std::collections::BTreeSet;

const MAX_VALUATION_AGE_MS: i64 = 30_000;

pub(crate) fn value_network_fee(
    cost: &OnchainReplenishmentNetworkCost,
    quote: Option<OnchainUsdValuation>,
    now_ms: i64,
) -> Result<OnchainReplenishmentCostValuation, String> {
    value_fee(cost.total_fee_exact.as_deref(), &cost.asset, quote, now_ms)
}

pub(crate) fn value_withdrawal_fee(
    cost: &OnchainReplenishmentWithdrawalCost,
    quote: Option<OnchainUsdValuation>,
    now_ms: i64,
) -> Result<OnchainReplenishmentCostValuation, String> {
    if !cost.confirmed
        || cost.source.trim().is_empty()
        || cost.asset.trim().is_empty()
        || cost.observed_at_ms <= 0
        || cost.observed_at_ms > now_ms
    {
        return Err("提币费尚未取得已确认回执，不能折算美元".into());
    }
    value_fee(Some(&cost.fee_exact), &cost.asset, quote, now_ms)
}

fn value_fee(
    fee: Option<&str>,
    asset: &str,
    quote: Option<OnchainUsdValuation>,
    now_ms: i64,
) -> Result<OnchainReplenishmentCostValuation, String> {
    let fee = fee
        .and_then(|value| Decimal::from_str_exact(value).ok())
        .filter(|fee| *fee >= Decimal::ZERO)
        .ok_or("实扣费用缺失或精度无效，不能折算美元")?;
    if fee == Decimal::ZERO {
        return Ok(OnchainReplenishmentCostValuation {
            usd_amount_exact: "0".into(),
            quote: None,
            valued_at_ms: now_ms,
        });
    }
    let quote = quote.ok_or("费用美元汇率尚未就绪")?;
    super::usd_valuation::rate(Some(&quote), asset, MAX_VALUATION_AGE_MS, now_ms)
        .ok_or("费用汇率不是同一资产的新鲜 WS 报价")?;
    let pair = onchain_monitor::normalized_pair_symbol(&quote.symbol);
    if quote.venue.trim().is_empty()
        || quote.source != "ws_push"
        || (pair != onchain_monitor::normalized_pair_symbol(&format!("{asset}/USD"))
            && pair != onchain_monitor::normalized_pair_symbol(&format!("USD/{asset}")))
    {
        return Err("费用估值缺少真实 USD 现货交易对，不能默认稳定币等于美元".into());
    }
    // Cost uses the replacement ask, not sale proceeds at the bid. This is a
    // dated USD valuation of native fees, not an assertion of a USD cash debit.
    let usd = Decimal::from_f64(quote.usd_ask)
        .and_then(|ask| fee.checked_mul(ask))
        .filter(|value| *value > Decimal::ZERO)
        .ok_or("费用美元折算溢出或低于有效精度")?;
    Ok(OnchainReplenishmentCostValuation {
        usd_amount_exact: usd.normalize().to_string(),
        quote: Some(quote),
        valued_at_ms: now_ms,
    })
}

pub(crate) fn network_fee_usd(cost: &OnchainReplenishmentNetworkCost) -> Result<f64, String> {
    let value = cost
        .usd_valuation
        .as_ref()
        .ok_or("实扣网络费尚未留存美元折算证据")?;
    let verified = value_network_fee(cost, value.quote.clone(), value.valued_at_ms)?;
    if verified != *value {
        return Err("网络费美元折算与已保存的原币费用或汇率不一致".into());
    }
    valuation_amount(value)
}

pub(crate) fn withdrawal_fee_usd(cost: &OnchainReplenishmentWithdrawalCost) -> Result<f64, String> {
    let value = cost
        .usd_valuation
        .as_ref()
        .ok_or("实扣提币费尚未留存美元折算证据")?;
    let verified = value_withdrawal_fee(cost, value.quote.clone(), value.valued_at_ms)?;
    if verified != *value {
        return Err("提币费美元折算与已保存的原币费用或汇率不一致".into());
    }
    valuation_amount(value)
}

fn valuation_amount(value: &OnchainReplenishmentCostValuation) -> Result<f64, String> {
    let amount = value
        .usd_amount_exact
        .parse::<Decimal>()
        .map_err(|_| "费用美元金额无效")?;
    amount
        .to_f64()
        .filter(|value| {
            value.is_finite() && *value >= 0.0 && (*value > 0.0 || amount == Decimal::ZERO)
        })
        .ok_or_else(|| "费用美元金额超出支持范围".into())
}

pub(super) fn missing_assets(runs: &[OnchainReplenishmentRun]) -> Vec<String> {
    runs.iter()
        .flat_map(|run| &run.transfers)
        .flat_map(|transfer| {
            let network = transfer
                .network_cost
                .as_ref()
                .filter(|cost| cost.usd_valuation.is_none())
                .map(|cost| (&cost.asset, cost.total_fee_exact.as_deref()));
            let withdrawal = transfer
                .withdrawal_cost
                .as_ref()
                .filter(|cost| cost.confirmed && cost.usd_valuation.is_none())
                .map(|cost| (&cost.asset, Some(cost.fee_exact.as_str())));
            [network, withdrawal].into_iter().flatten()
        })
        .filter(|(_, fee)| {
            fee.and_then(|s| Decimal::from_str_exact(s).ok())
                .is_some_and(|n| n > Decimal::ZERO)
        })
        .map(|(asset, _)| asset.trim().to_ascii_uppercase())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn waiting_for_valuation(run: &OnchainReplenishmentRun) -> bool {
    run.transfers.iter().any(|transfer| {
        transfer
            .network_cost
            .as_ref()
            .is_some_and(|cost| cost.total_fee_exact.is_some() && cost.usd_valuation.is_none())
            || transfer
                .withdrawal_cost
                .as_ref()
                .is_some_and(|cost| cost.confirmed && cost.usd_valuation.is_none())
    })
}

pub(super) fn fill_from_ws(
    state: &AppState,
    mut run: OnchainReplenishmentRun,
) -> Result<OnchainReplenishmentRun, String> {
    let snapshot = state.onchain_monitor().snapshot();
    let config = &snapshot.config;
    for index in 0..run.transfers.len() {
        if let Some(cost) = run.transfers[index]
            .withdrawal_cost
            .as_ref()
            .filter(|cost| cost.confirmed && cost.usd_valuation.is_none())
        {
            let now_ms = common::time::now_ms();
            let quote = super::usd_valuation::evidence(state, config, &cost.asset, now_ms).ok();
            if let Ok(value) = value_withdrawal_fee(cost, quote, now_ms) {
                run = state
                    .onchain_replenishment_plans()
                    .record_withdrawal_cost_valuation(&run.run_id, index as u32, value, now_ms)?;
            }
        }
        let Some(cost) = run.transfers[index]
            .network_cost
            .as_ref()
            .filter(|cost| cost.usd_valuation.is_none() && cost.total_fee_exact.is_some())
        else {
            continue;
        };
        let now_ms = common::time::now_ms();
        let quote = super::usd_valuation::evidence(state, config, &cost.asset, now_ms).ok();
        let Ok(value) = value_network_fee(cost, quote, now_ms) else {
            continue;
        };
        run = state
            .onchain_replenishment_plans()
            .record_network_cost_valuation(&run.run_id, index as u32, value, now_ms)?;
    }
    Ok(run)
}

#[cfg(test)]
pub(super) fn fixture(fee: &str, rate: f64, now_ms: i64) -> OnchainReplenishmentNetworkCost {
    let mut cost = OnchainReplenishmentNetworkCost {
        chain: "solana".into(),
        transaction_id: "signature".into(),
        block_ref: "120".into(),
        payer: "wallet".into(),
        asset: "SOL".into(),
        execution_fee_exact: Some(fee.into()),
        additional_fee_exact: Some("0".into()),
        total_fee_exact: Some(fee.into()),
        source: "Solana getTransaction".into(),
        observed_at_ms: now_ms,
        problem: None,
        usd_valuation: None,
    };
    cost.usd_valuation = Some(
        value_network_fee(
            &cost,
            Some(super::usd_valuation::fixture("SOL", rate, now_ms)),
            now_ms,
        )
        .unwrap(),
    );
    cost
}

#[cfg(test)]
mod withdrawal_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore;

    #[test]
    fn network_fee_valuation_uses_ask_and_preserves_subcent_precision_without_a_stablecoin_peg() {
        let mut cost = fixture("0.000005", 100.0, 1000);
        let mut quote = super::super::usd_valuation::fixture("SOL", 99.0, 1000);
        quote.usd_ask = 101.0;
        assert_eq!(
            value_network_fee(&cost, Some(quote), 1010)
                .unwrap()
                .usd_amount_exact,
            "0.000505"
        );
        cost.asset = "USDC".into();
        cost.total_fee_exact = Some("1".into());
        assert!(value_network_fee(&cost, None, 1000).is_err());
        let quote = super::super::usd_valuation::fixture("USDC", 0.8, 1000);
        assert_eq!(
            value_network_fee(&cost, Some(quote), 1000)
                .unwrap()
                .usd_amount_exact,
            "0.8"
        );
        cost.total_fee_exact = Some("0".into());
        assert_eq!(
            value_network_fee(&cost, None, 1000)
                .unwrap()
                .usd_amount_exact,
            "0"
        );
    }

    #[test]
    fn network_fee_valuation_rejects_stale_wrong_market_rest_and_incomplete_fees() {
        let mut cost = fixture("0.000005", 100.0, 1000);
        for case in [
            "asset", "pair", "source", "venue", "future", "stale", "ask", "crossed",
        ] {
            let mut quote = super::super::usd_valuation::fixture("SOL", 100.0, 1000);
            match case {
                "asset" => quote.asset = "ETH".into(),
                "pair" => quote.symbol = "SOL/USDT".into(),
                "source" => quote.source = "rest_baseline".into(),
                "venue" => quote.venue.clear(),
                "future" => quote.observed_at_ms = 1001,
                "stale" => quote.observed_at_ms = -30000,
                "ask" => quote.usd_ask = f64::NAN,
                "crossed" => quote.usd_ask = 1.0,
                _ => unreachable!(),
            }
            assert!(
                value_network_fee(&cost, Some(quote), 1000).is_err(),
                "{case}"
            );
        }
        for fee in [None, Some("-1".into()), Some("invalid".into())] {
            cost.total_fee_exact = fee;
            assert!(value_network_fee(
                &cost,
                Some(super::super::usd_valuation::fixture("SOL", 100.0, 1000)),
                1000
            )
            .is_err());
        }
    }

    #[test]
    fn network_fee_valuation_backfill_survives_restart_without_repricing_or_funds_state_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.jsonl");
        let mut config = common::config::AppConfig::default();
        config.storage.onchain_replenishment_ledger_path =
            Some(path.to_string_lossy().into_owned());
        let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../shared-types/fixtures/onchain_replenishment_locked.json"
        )))
        .unwrap();
        let cost = fixture("0.000005", 100.0, 1000);
        let value = cost.usd_valuation.clone().unwrap();
        run.transfers[0].network_cost = Some(cost.clone());
        run.transfers[0]
            .network_cost
            .as_mut()
            .unwrap()
            .usd_valuation = None;
        std::fs::write(
            &path,
            format!("{}\n", serde_json::json!({"schemaVersion":1,"run":run})),
        )
        .unwrap();
        let store = OnchainReplenishmentPlanStore::load(&config);
        let before = store.run(&run.run_id, 1000).unwrap();
        assert!(waiting_for_valuation(&before));
        let updated = store
            .record_network_cost_valuation(&run.run_id, 0, value.clone(), 1000)
            .unwrap();
        assert!(!waiting_for_valuation(&updated));
        assert_eq!(updated.status, before.status);
        assert_eq!(updated.transfers[0].status, before.transfers[0].status);
        assert_eq!(
            updated.transfers[0].transaction_id,
            before.transfers[0].transaction_id
        );
        assert_eq!(
            updated.transfers[0].last_checked_at_ms,
            before.transfers[0].last_checked_at_ms
        );
        assert_eq!(updated.plan, before.plan);
        let bytes = std::fs::read(&path).unwrap();
        let replay = store
            .record_network_cost_valuation(&run.run_id, 0, value.clone(), 1001)
            .unwrap();
        assert_eq!(replay, updated);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let mut changed = value.clone();
        changed.usd_amount_exact = "0.00001".into();
        assert!(store
            .record_network_cost_valuation(&run.run_id, 0, changed, 1001)
            .is_err());
        let repriced = value_network_fee(
            &cost,
            Some(super::super::usd_valuation::fixture("SOL", 10.0, 1001)),
            1001,
        )
        .unwrap();
        assert!(store
            .record_network_cost_valuation(&run.run_id, 0, repriced, 1001)
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let restored = OnchainReplenishmentPlanStore::load(&config)
            .run(&run.run_id, 1100)
            .unwrap();
        assert_eq!(restored.transfers, updated.transfers);
        assert!(missing_assets(&[restored]).is_empty());
    }
}
