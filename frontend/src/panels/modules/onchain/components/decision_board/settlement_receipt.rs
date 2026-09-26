use leptos::prelude::*;
use shared_types::{OnchainCexSettlement, OnchainCexSettlementStatus, OrderSide};

pub(super) fn chain_receipt(receipt: shared_types::OnchainChainSettlement) -> impl IntoView {
    let complete = receipt.status == shared_types::OnchainChainSettlementStatus::Complete;
    let title = match receipt.status {
        shared_types::OnchainChainSettlementStatus::Complete => "链上收支已核算",
        shared_types::OnchainChainSettlementStatus::Pending => "链上收支待核算",
        shared_types::OnchainChainSettlementStatus::ReviewRequired => "链上收支待复核",
    };
    let input = receipt
        .input_amount_raw
        .as_deref()
        .map(|raw| {
            raw_display(
                raw,
                receipt.basis.assets.input.decimals,
                &receipt.basis.assets.input.symbol,
            )
        })
        .unwrap_or_else(|| "待确认".into());
    let output = receipt
        .output_amount_raw
        .as_deref()
        .map(|raw| {
            raw_display(
                raw,
                receipt.basis.assets.output.decimals,
                &receipt.basis.assets.output.symbol,
            )
        })
        .unwrap_or_else(|| "待确认".into());
    let fee = receipt
        .network_cost
        .as_ref()
        .and_then(|cost| {
            cost.total_fee_exact
                .as_ref()
                .map(|amount| format!("{amount} {}", cost.asset))
        })
        .unwrap_or_else(|| "待确认".into());
    let payer = receipt.network_cost.as_ref().map(|cost| cost.payer.clone());
    let extra = receipt
        .additional_native_change_raw
        .as_deref()
        .filter(|raw| *raw != "0")
        .and_then(|raw| {
            shared_types::onchain_chain_preset(&receipt.basis.chain).map(|chain| {
                let (sign, amount) = raw
                    .strip_prefix('-')
                    .map(|v| ("-", v))
                    .unwrap_or(("+", raw));
                format!(
                    "其他原生币变化 {sign}{}",
                    raw_display(
                        amount,
                        if chain.id == "solana" { 9 } else { 18 },
                        chain.base_token
                    )
                )
            })
        });
    view! {
        <div class="onchain-cex-settlement" aria-label="链上实际收支">
            <strong class=if complete { "is-positive" } else { "is-warning" }>{title}</strong>
            <dl>
                <div><dt>"实际支出"</dt><dd class="num">{input}</dd></div>
                <div><dt>"实际到账"</dt><dd class="num">{output}</dd></div>
            </dl>
            <span>{format!("网络费 {fee}")}</span>
            {extra.map(|value| view! { <span>{value}</span> })}
            <details><summary>"链上核算明细"</summary>
                {payer.map(|payer| view! { <span>{format!("网络费付款地址 {payer}")}</span> })}
                {receipt.block_ref.map(|block| view! { <span>{format!("区块 / Slot {block}")}</span> })}
                {receipt.problem.map(|problem| view! { <span class="is-warning">{problem}</span> })}
            </details>
        </div>
    }
}

pub(super) fn chain_input_adjustment(
    adjustment: shared_types::OnchainChainInputAdjustment,
) -> impl IntoView {
    let changed = adjustment.original_input_amount_raw != adjustment.submitted_input_amount_raw;
    let original = raw_display(
        &adjustment.original_input_amount_raw,
        adjustment.decimals,
        &adjustment.asset,
    );
    let submitted = raw_display(
        &adjustment.submitted_input_amount_raw,
        adjustment.decimals,
        &adjustment.asset,
    );
    view! {
        <div class="onchain-cex-settlement" aria-label="链上输入核对">
            <strong class=if changed { "is-warning" } else { "is-positive" }>
                {if changed { "按净到账调整链上卖出" } else { "净到账已覆盖链上输入" }}
            </strong>
            <dl>
                <div><dt>"原计划"</dt><dd class="num">{original}</dd></div>
                <div><dt>"本次链上输入"</dt><dd class="num">{submitted}</dd></div>
            </dl>
            <span>{format!("交易所 净到账 {} {}", adjustment.cex_net_received, adjustment.asset)}</span>
            {(adjustment.residual_base_amount != "0").then(|| view! { <span class="is-warning">{format!("预计双端净余量 {} {}", adjustment.residual_base_amount, adjustment.asset)}</span> })}
        </div>
    }
}

fn raw_display(raw: &str, decimals: u8, asset: &str) -> String {
    let Some(value) = raw.parse::<u128>().ok().filter(|_| decimals <= 38) else {
        return "待核对".into();
    };
    if decimals == 0 {
        return format!("{value} {asset}");
    }
    let digits = format!("{value:0>width$}", width = usize::from(decimals) + 1);
    let (integer, fraction) = digits.split_at(digits.len() - usize::from(decimals));
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        format!("{integer} {asset}")
    } else {
        format!("{integer}.{fraction} {asset}")
    }
}

pub(super) fn recovery_residual(
    residual: shared_types::OnchainCexRecoveryResidual,
) -> impl IntoView {
    let (tone, label) = if residual.amount.as_deref() == Some("0") {
        ("is-positive", "回滚净数量已归零".to_owned())
    } else if let Some(shortfall) = residual
        .amount
        .as_deref()
        .and_then(|amount| amount.strip_prefix('-'))
    {
        (
            "is-warning",
            format!("回滚后还缺 {shortfall} {}", residual.asset),
        )
    } else if let Some(amount) = residual.amount {
        (
            "is-warning",
            format!("回滚后剩余 {amount} {}", residual.asset),
        )
    } else {
        ("is-warning", "回滚净数量待核算".into())
    };
    view! { <div class="onchain-cex-settlement" aria-label="回滚剩余资产"><strong class=tone>{label}</strong></div> }
}

pub(super) fn receipt(settlement: Option<OnchainCexSettlement>, has_fill: bool) -> impl IntoView {
    settlement.map(render).or_else(|| {
        has_fill.then(|| {
            view! { <div class="onchain-cex-settlement is-warning">"到账核算待补齐"</div> }
                .into_any()
        })
    })
}

fn render(settlement: OnchainCexSettlement) -> AnyView {
    receipt_content(settlement).into_any()
}

fn receipt_content(settlement: OnchainCexSettlement) -> impl IntoView {
    let (from, to) = match settlement.basis.side {
        OrderSide::Buy => (&settlement.basis.quote_asset, &settlement.basis.base_asset),
        OrderSide::Sell => (&settlement.basis.base_asset, &settlement.basis.quote_asset),
    };
    let complete = settlement.status == OnchainCexSettlementStatus::Complete;
    let debit = settlement
        .debit_amount
        .as_deref()
        .map(|value| format!("{value} {from}"));
    let credit = settlement
        .credit_amount
        .as_deref()
        .map(|value| format!("{value} {to}"));
    let fees = settlement
        .fees
        .iter()
        .map(|fee| format!("{} {}", fee.amount, fee.asset))
        .collect::<Vec<_>>()
        .join(" + ");
    let fee_label = if complete && fees.is_empty() {
        "0".to_owned()
    } else {
        fees
    };
    let status = match settlement.status {
        OnchainCexSettlementStatus::Complete => "已核算",
        OnchainCexSettlementStatus::PendingFills => "成交明细待齐",
        OnchainCexSettlementStatus::PendingFees => "手续费待齐",
        OnchainCexSettlementStatus::Invalid => "核算待复核",
    };
    view! {
        <div class="onchain-cex-settlement" aria-label="成交核算">
            <strong class=if complete { "is-positive" } else { "is-warning" }>{status}</strong>
            {complete.then(|| view! {
                <dl>
                    <div><dt>"交易支出"</dt><dd class="num">{debit.unwrap_or_else(|| "待确认".into())}</dd></div>
                    <div><dt>"交易净入账"</dt><dd class="num">{credit.unwrap_or_else(|| "待确认".into())}</dd></div>
                </dl>
            })}
            <details>
                <summary>{format!("{} 笔成交 · 费用明细", settlement.fill_event_ids.len())}</summary>
                <span>{if fee_label.is_empty() { "实际费用待确认".to_owned() } else { format!("手续费 {fee_label}") }}</span>
                {settlement.problem.map(|problem| view! { <span class="is-warning">{problem}</span> })}
            </details>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{OnchainCexSettlementBasis, OnchainCexSettlementFee};

    #[test]
    fn chain_settlement_actual_receipt_and_missing_fee_are_not_estimated_profit() {
        let receipt: shared_types::OnchainChainSettlement = if let Ok(path) =
            std::env::var("CROSSLINE_CHAIN_RECEIPT_FIXTURE")
        {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        } else {
            serde_json::from_value(serde_json::json!({"basis":{"chain":"solana","wallet":"wallet","transactionId":"signature",
                "assets":{"input":{"symbol":"USDT","address":"mint-a","decimals":6},"output":{"symbol":"USDC","address":"mint-b","decimals":6}},
                "maximumInputRaw":"100000000","minimumOutputRaw":"104000000"},"status":"complete","inputAmountRaw":"99000000","outputAmountRaw":"105000000",
                "additionalNativeChangeRaw":"-9000","networkCost":{"chain":"solana","transactionId":"signature","blockRef":"120","payer":"wallet","asset":"SOL",
                "executionFeeExact":"0.000005","additionalFeeExact":"0","totalFeeExact":"0.000005","source":"fixture","observedAtMs":1,"problem":null,"usdValuation":null},
                "blockRef":"120","observedAtMs":1,"problem":null})).unwrap()
        };
        Owner::new().with(|| {
            let html = chain_receipt(receipt.clone()).to_html();
            assert!(html.contains("99 USDT"));
            assert!(html.contains("105 USDC"));
            assert!(html.contains("网络费 0.000005 SOL"));
            assert!(html.contains("其他原生币变化 -0.000009 SOL"));
            assert!(!html.contains("盈利"));
            let mut partial = receipt;
            partial.status = shared_types::OnchainChainSettlementStatus::Pending;
            partial.network_cost = None;
            partial.problem = Some("网络费仍待核算".into());
            let partial_html = chain_receipt(partial).to_html();
            assert!(partial_html.contains("网络费 待确认"));
            assert!(partial_html.contains("链上收支待核算"));
            assert!(!partial_html.contains("网络费 0 SOL"));
            if let Ok(path) = std::env::var("CROSSLINE_CHAIN_RECEIPT_HTML") {
                std::fs::write(path, format!("<section class=onchain-execution-legs><div class=onchain-execution-leg>{html}</div><div class=onchain-execution-leg>{partial_html}</div></section>")).unwrap();
            }
        });
    }

    #[test]
    fn primary_alignment_receipt_shows_original_and_submitted_without_float_rounding() {
        Owner::new().with(|| {
            let html = chain_input_adjustment(shared_types::OnchainChainInputAdjustment {
                original_input_amount_raw: "1000000000".into(), submitted_input_amount_raw: "999999999".into(),
                asset: "SOL".into(), decimals: 9, cex_order_id: "order-1".into(), cex_net_received: "0.9999999999".into(),
                residual_base_amount: "0.0000000009".into(), residual_cost_estimate_usd: 0.00000009,
            }).to_html();
            assert!(html.contains("按净到账调整链上卖出"));
            assert!(html.contains("0.999999999 SOL"));
            assert!(html.contains("交易所 净到账 0.9999999999 SOL"));
            assert!(html.contains("预计双端净余量 0.0000000009 SOL"));
            assert!(!html.contains("交易所 余量"));
            assert!(html.contains("1 SOL"));
            if let Ok(path) = std::env::var("CROSSLINE_ALIGNMENT_FIXTURE") {
                std::fs::write(path, format!("<section class=onchain-execution-legs><div class=onchain-execution-leg><span>02</span><strong>链上交易</strong><span>Solana</span><span>待确认</span><small>已重新询价</small><small>tx</small>{html}</div></section>")).unwrap();
            }
        });
        assert_eq!(raw_display("1", 18, "TOKEN"), "0.000000000000000001 TOKEN");
        assert_eq!(raw_display("-1", 9, "SOL"), "待核对");
    }

    #[test]
    fn cex_compensation_residual_remains_visible_without_rounding_away_dust() {
        Owner::new().with(|| {
            for (amount, label) in [
                ("0.00000001", "回滚后剩余 0.00000001 SOL"),
                ("-0.0004", "回滚后还缺 0.0004 SOL"),
                ("0", "回滚净数量已归零"),
            ] {
                let html = recovery_residual(shared_types::OnchainCexRecoveryResidual {
                    original_order_id: "order-1".into(),
                    asset: "SOL".into(),
                    amount: Some(amount.into()),
                })
                .to_html();
                assert!(html.contains(label));
                assert_eq!(html.contains("is-positive"), amount == "0");
            }
        });
    }

    #[test]
    fn cex_compensation_pending_residual_never_claims_flat_inventory() {
        Owner::new().with(|| {
            let html = recovery_residual(shared_types::OnchainCexRecoveryResidual {
                original_order_id: "order-1".into(),
                asset: "SOL".into(),
                amount: None,
            })
            .to_html();
            assert!(html.contains("回滚净数量待核算"));
            assert!(!html.contains("归零"));
        });
    }

    #[test]
    fn cex_compensation_visual_fixture() {
        let Ok(path) = std::env::var("CROSSLINE_COMPENSATION_FIXTURE") else {
            return;
        };
        Owner::new().with(|| {
            let mut original = sample();
            original.basis.side = OrderSide::Buy;
            original.debit_amount = Some("100".into());
            original.credit_amount = Some("0.9996".into());
            original.fees = vec![OnchainCexSettlementFee { asset: "SOL".into(), amount: "0.0004".into() }];
            let original = receipt_content(original).to_html();
            let mut reverse = sample();
            reverse.debit_amount = Some("0.999".into());
            reverse.credit_amount = Some("99.8001".into());
            reverse.fees = vec![OnchainCexSettlementFee { asset: "USD".into(), amount: "0.0999".into() }];
            let reverse = receipt_content(reverse).to_html();
            let residual = recovery_residual(shared_types::OnchainCexRecoveryResidual {
                original_order_id: "order-1".into(), asset: "SOL".into(), amount: Some("0.0006".into()),
            }).to_html();
            std::fs::write(path, format!("<section class=onchain-execution-legs><div class=onchain-execution-leg><span>01</span><strong>交易所 主单</strong><span>KRAKEN · SOL/USD</span><span>已完成</span><small>买入已成交</small><small>order-1</small>{original}</div><div class=onchain-execution-leg><span>02</span><strong>补偿单</strong><span>KRAKEN · SOL/USD</span><span>已完成</span><small>卖出已成交</small><small>reverse-1</small>{reverse}{residual}</div></section>")).unwrap();
        });
    }

    fn sample() -> OnchainCexSettlement {
        OnchainCexSettlement {
            basis: OnchainCexSettlementBasis {
                order_id: "order-1".into(),
                venue: "kraken".into(),
                symbol: "SOL/USD".into(),
                side: OrderSide::Sell,
                base_asset: "SOL".into(),
                quote_asset: "USD".into(),
                confirmed_quantity: 1.0,
            },
            status: OnchainCexSettlementStatus::Complete,
            gross_base_amount: Some("1".into()),
            gross_quote_amount: Some("112".into()),
            debit_amount: Some("1".into()),
            credit_amount: Some("111.888".into()),
            fees: vec![OnchainCexSettlementFee {
                asset: "USD".into(),
                amount: "0.112".into(),
            }],
            fill_event_ids: vec!["trade-1".into(), "trade-2".into()],
            observed_at_ms: Some(2),
            problem: None,
        }
    }

    #[test]
    fn cex_settlement_receipt_keeps_net_credit_and_fee_units_visible() {
        Owner::new().with(|| {
            let html = receipt_content(sample()).to_html();
            assert!(html.contains("111.888 USD"));
            assert!(html.contains("0.112 USD"));
            assert!(html.contains("交易净入账"));
            assert!(!html.contains("利润"));
        });
    }

    #[test]
    fn cex_settlement_pending_fee_never_renders_fake_zero_or_net_credit() {
        Owner::new().with(|| {
            let mut pending = sample();
            pending.status = OnchainCexSettlementStatus::PendingFees;
            pending.fees.clear();
            let html = receipt_content(pending).to_html();
            assert!(html.contains("手续费待齐"));
            assert!(html.contains("实际费用待确认"));
            assert!(!html.contains("111.888"));
            assert!(!html.contains("手续费 0"));
        });
    }

    #[test]
    fn cex_settlement_visual_fixture() {
        let Ok(path) = std::env::var("CROSSLINE_SETTLEMENT_FIXTURE") else {
            return;
        };
        Owner::new().with(|| {
            let complete = receipt_content(sample()).to_html();
            let mut pending = sample();
            pending.status = OnchainCexSettlementStatus::PendingFees;
            pending.fees.clear();
            pending.problem = Some("成交数量已对齐，等待实际手续费或扣费资产；不按零费用计算".into());
            let pending = receipt_content(pending).to_html();
            std::fs::write(path, format!("<section class=onchain-execution-legs><div class=onchain-execution-leg><span>01</span><strong>交易所 主单</strong><span>KRAKEN · SOL/USD</span><span>已完成</span><small>已成交</small><small>order-1</small>{complete}</div><div class=onchain-execution-leg><span>02</span><strong>Quote 换汇</strong><span>KRAKEN · USDC/USD</span><span>已完成</span><small>已成交</small><small>order-2</small>{pending}</div></section>")).unwrap();
        });
    }
}
