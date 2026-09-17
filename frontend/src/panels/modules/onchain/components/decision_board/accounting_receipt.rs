use leptos::prelude::*;
use shared_types::{
    OnchainExecutionAccounting, OnchainExecutionAccountingStatus as Status,
    OnchainExecutionCashFlowKind,
};

pub(super) fn receipt(
    accounting: Option<OnchainExecutionAccounting>,
    estimated: f64,
    replenishments: usize,
    approvals: usize,
) -> Option<impl IntoView> {
    accounting.map(|accounting| render(accounting, estimated, replenishments, approvals))
}

fn render(
    accounting: OnchainExecutionAccounting,
    estimated: f64,
    replenishments: usize,
    approvals: usize,
) -> impl IntoView {
    let scope = if replenishments == 0 {
        "仅本次执行 · 补库费用未归集 · 美元为核算时折算".to_owned()
    } else {
        format!("本次执行及 {replenishments} 笔已归集补库费用 · 美元为核算时折算")
    };
    let scope = format!(
        "{scope} · {}",
        if approvals == 0 {
            "授权费用未归集".into()
        } else {
            format!("{approvals} 笔已归集授权费用")
        }
    );
    let (title, tone) = match accounting.status {
        Status::PendingReceipts => ("成交收支待核算", "is-warning"),
        Status::PendingValuation => ("原币收支已核算 · 汇率待齐", "is-warning"),
        Status::Valued => ("成交收支已核算", "is-positive"),
    };
    let usd = accounting
        .usd_value
        .as_ref()
        .filter(|_| accounting.status == Status::Valued)
        .map(|value| signed(&value.net_usd_exact, "USD"))
        .unwrap_or_else(|| "待核算".into());
    let clock = accounting
        .usd_value
        .as_ref()
        .and_then(|value| crate::panels::modules::timestamp::local_date_hm(value.valued_at_ms));
    let expected = if estimated.is_finite() {
        format!("{estimated:+.2} USD")
    } else {
        "未知".into()
    };
    view! {
        <section class="onchain-cex-settlement onchain-execution-accounting" aria-label="执行收支汇总">
            <strong class=tone>{title}</strong>
            <dl>
                <div><dt>"构建时预计"</dt><dd class="num">{expected}</dd></div>
                <div><dt>"成交净变动折算"</dt><dd class="num">{usd}</dd></div>
            </dl>
            {(!accounting.net_assets.is_empty()).then(|| view! {
                <span>{format!("原币净变化 {}", accounting.net_assets.iter().map(|a| signed(&a.amount_exact, &a.asset)).collect::<Vec<_>>().join(" · "))}</span>
            })}
            <small>{scope}</small>
            {accounting.problems.into_iter().map(|problem| view! { <span class="is-warning">{problem}</span> }).collect_view()}
            <details><summary>"收支与汇率明细"</summary>
                {clock.map(|time| view! { <span>{format!("折算时间 {time}")}</span> })}
                {accounting.flows.into_iter().map(|flow| {
                    let kind = match flow.kind { OnchainExecutionCashFlowKind::Trade => "成交",
                        OnchainExecutionCashFlowKind::Fee => "手续费", OnchainExecutionCashFlowKind::ReplenishmentFee => "补库费用", OnchainExecutionCashFlowKind::ApprovalFee => "授权费用", OnchainExecutionCashFlowKind::OtherNativeChange => "其他原生币变化" };
                    let location = flow.location.strip_prefix("cex:").map(|venue| format!("{} 现货", venue.to_uppercase()))
                        .unwrap_or_else(|| flow.location.strip_prefix("chain:").unwrap_or(&flow.location).replace(':', " · 钱包 "));
                    view! { <span title=flow.source_id>{format!("{location} · {kind} · {}", signed(&flow.amount_exact, &flow.asset))}</span> }
                }).collect_view()}
                {accounting.usd_value.map(|value| value.rates.into_iter().map(|rate| {
                    let label = if rate.source == "same_currency" { "USD · 美元原币".to_owned() } else {
                        format!("{} · {} · 买价 {} / 卖价 {} · WS", rate.venue.to_uppercase(), rate.symbol, rate.usd_bid, rate.usd_ask)
                    };
                    view! { <span>{label}</span> }
                }).collect_view())}
            </details>
        </section>
    }
}

fn signed(value: &str, asset: &str) -> String {
    format!(
        "{}{value} {asset}",
        if value.starts_with('-') || value == "0" {
            ""
        } else {
            "+"
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_allocation_receipt_renders_actual_native_cost_and_negative_total() {
        let accounting = if let Ok(path) = std::env::var("CROSSLINE_APPROVAL_ACCOUNTING_FIXTURE") {
            let run: shared_types::OnchainExecutionSubmitResponse =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            run.accounting.unwrap()
        } else {
            serde_json::from_value(serde_json::json!({"status":"valued",
                "flows":[{"location":"chain:ethereum:0xwallet","sourceId":"0xhash","asset":"ETH","amountExact":"-0.000021","kind":"approval_fee"}],
                "netAssets":[{"asset":"ETH","amountExact":"-0.000021"}],
                "usdValue":{"netUsdExact":"-0.3941","valuedAtMs":1000,"rates":[]},"problems":[]})).unwrap()
        };
        Owner::new().with(|| {
            let html = render(accounting, 9.0, 0, 1).to_html();
            assert!(html.contains("-0.3941 USD"));
            assert!(html.contains("1 笔已归集授权费用"));
            assert!(html.contains("授权费用 · -0.000021 ETH"));
            assert!(!html.contains("授权费用未归集"));
            if let Ok(path) = std::env::var("CROSSLINE_APPROVAL_ACCOUNTING_HTML") {
                std::fs::write(path, html).unwrap();
            }
        });
    }

    #[test]
    fn replenishment_allocation_receipt_render_shows_included_fees_and_actual_total() {
        let run: shared_types::OnchainExecutionSubmitResponse = if let Ok(path) =
            std::env::var("CROSSLINE_REPLENISHMENT_ACCOUNTING_FIXTURE")
        {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        } else {
            serde_json::from_value(serde_json::json!({"runId":"fixture","buildId":"build","status":"completed","estimatedNetProfitUsd":9,
                "remainingExposureUsd":0,"message":"confirmed","startedAtMs":1,"updatedAtMs":2,
                "accounting":{"status":"valued","flows":[{"location":"cex:binance","sourceId":"withdrawal-1","asset":"USDC","amountExact":"-0.1","kind":"replenishment_fee"}],
                "netAssets":[{"asset":"USDC","amountExact":"4.9"}],"usdValue":{"netUsdExact":"5.0586","valuedAtMs":1000,"rates":[]},"problems":[]}})).unwrap()
        };
        Owner::new().with(|| {
            let html =
                render(run.accounting.unwrap(), run.estimated_net_profit_usd, 1, 0).to_html();
            assert!(html.contains("+5.0586 USD"));
            assert!(html.contains("1 笔已归集补库费用"));
            assert!(html.contains("补库费用 · -0.1 USDC"));
            assert!(!html.contains("补库费用未归集"));
            if let Ok(path) = std::env::var("CROSSLINE_REPLENISHMENT_ACCOUNTING_HTML") {
                std::fs::write(path, html).unwrap();
            }
        });
    }

    #[test]
    fn execution_accounting_render_separates_estimate_actual_fx_and_missing_fees() {
        let run: shared_types::OnchainExecutionSubmitResponse = if let Ok(path) =
            std::env::var("CROSSLINE_EXECUTION_ACCOUNTING_FIXTURE")
        {
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
        } else {
            serde_json::from_value(serde_json::json!({"runId":"fixture","buildId":"build","status":"completed","estimatedNetProfitUsd":9,
                "remainingExposureUsd":0,"message":"confirmed","startedAtMs":1,"updatedAtMs":2,
                "accounting":{"status":"valued","flows":[],"netAssets":[{"asset":"USD","amountExact":"-0.35"}],
                "usdValue":{"netUsdExact":"5.1486","valuedAtMs":1000,"rates":[]},"problems":[]}})).unwrap()
        };
        Owner::new().with(|| {
            let accounting = run.accounting.unwrap();
            let html = render(accounting.clone(), run.estimated_net_profit_usd, 0, 0).to_html();
            assert!(html.contains("+9.00 USD"));
            assert!(html.contains("+5.1486 USD"));
            assert!(html.contains("补库费用未归集"));
            let attributed =
                render(accounting.clone(), run.estimated_net_profit_usd, 2, 0).to_html();
            assert!(attributed.contains("2 笔已归集补库费用"));
            assert!(!attributed.contains("补库费用未归集"));
            assert!(html.contains("-0.35 USD"));
            let mut pending = accounting;
            pending.status = Status::PendingValuation;
            pending.usd_value = None;
            pending.problems = vec!["SOL/USD 汇率未就绪，不默认费用为零".into()];
            let unknown = render(pending.clone(), run.estimated_net_profit_usd, 0, 0).to_html();
            assert!(unknown.contains("汇率待齐"));
            assert!(!unknown.contains("+5.1486 USD"));
            pending.status = Status::PendingReceipts;
            pending.net_assets.clear();
            pending.flows.clear();
            pending.problems = vec!["CEX 实际手续费待确认".into()];
            let missing = render(pending, 9.0, 0, 0).to_html();
            assert!(missing.contains("成交收支待核算"));
            assert!(!missing.contains("+0 USD"));
            if let Ok(path) = std::env::var("CROSSLINE_EXECUTION_ACCOUNTING_HTML") {
                std::fs::write(path, format!("{html}{unknown}{missing}")).unwrap();
            }
        });
    }
}
