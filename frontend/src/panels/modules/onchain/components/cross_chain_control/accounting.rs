use super::*;
use crate::panels::modules::timestamp::{local_date_hm, local_hms};
use shared_types::{
    OnchainCrossChainAccounting, OnchainCrossChainFlowKind, OnchainExecutionAccountingStatus,
};

fn signed(amount: &str) -> String {
    if amount.starts_with('-') || amount == "0" {
        amount.into()
    } else {
        format!("+{amount}")
    }
}

fn tone(amount: &str) -> &'static str {
    if amount.starts_with('-') {
        "is-danger"
    } else {
        "is-positive"
    }
}

fn time_label(ms: i64) -> String {
    local_hms(ms).unwrap_or_else(|| "未知".into())
}

pub(super) fn summary(accounting: &OnchainCrossChainAccounting) -> impl IntoView {
    let status = match accounting.status {
        OnchainExecutionAccountingStatus::PendingReceipts => "部分收支 · 处理结果未齐",
        OnchainExecutionAccountingStatus::PendingValuation => "原币已核算 · 汇率待齐",
        OnchainExecutionAccountingStatus::Valued => "四步收支已核算",
    };
    let valuation = (accounting.status == OnchainExecutionAccountingStatus::Valued)
        .then_some(accounting.usd_value.as_ref())
        .flatten();
    let amount = valuation.map(|v| signed(&v.net_usd_exact));
    let value_tone = valuation.map_or("", |v| tone(&v.net_usd_exact));
    let assets = accounting.net_assets.iter().map(|row| {
        view! {
            <div class="cross-chain-net-asset">
                <div><strong>{row.asset.symbol.clone()}</strong><small>{chain_label(&row.chain)}</small></div>
                <span class=tone(&row.amount_exact)>{signed(&row.amount_exact)}</span>
                <details>
                    <summary>"资产位置"</summary>
                    <dl><div><dt>"钱包"</dt><dd><code>{row.wallet.clone()}</code></dd></div>
                        <div><dt>"合约"</dt><dd><code>{row.asset.address.clone()}</code></dd></div></dl>
                </details>
            </div>
        }
    }).collect_view();
    let flows = accounting.flows.iter().map(|flow| {
        let kind = match flow.kind {
            OnchainCrossChainFlowKind::Swap => "兑换",
            OnchainCrossChainFlowKind::Bridge => "桥款",
            OnchainCrossChainFlowKind::Recovery => "退款 / 异常到账",
            OnchainCrossChainFlowKind::NetworkFee => "链费",
            OnchainCrossChainFlowKind::OtherNativeChange => "额外原生币变化",
        };
        view! {
            <li><div><strong>{format!("第 {} 步 · {kind}", flow.position)}</strong>
                <span>{format!("{} {} · {}", signed(&flow.change.amount_exact), flow.change.asset.symbol, chain_label(&flow.change.chain))}</span></div>
                <dl><div><dt>"钱包"</dt><dd><code>{flow.change.wallet.clone()}</code></dd></div>
                    <div><dt>"合约"</dt><dd><code>{flow.change.asset.address.clone()}</code></dd></div>
                    <div><dt>"交易"</dt><dd><code>{flow.transaction_id.clone()}</code></dd></div></dl>
            </li>
        }
    }).collect_view();
    let rates = valuation.map(|v| {
        let rows = v.rates.iter().map(|rate| view! {
            <li>{format!("{} {} · 买一 {} / 卖一 {} USD · WS · {}", rate.venue.to_uppercase(), rate.symbol, rate.usd_bid, rate.usd_ask, time_label(rate.observed_at_ms))}</li>
        }).collect_view();
        view! { <p>{format!("折算时间：{}", local_date_hm(v.valued_at_ms).unwrap_or_else(|| "未知".into()))}</p><ul>{rows}</ul> }
    });
    let external = accounting.external_flows.iter().map(|flow| view! {
        <li><div><strong>{match flow.kind { shared_types::OnchainExecutionCashFlowKind::ApprovalFee => "授权费", _ => "补库费" }}</strong>
            <span>{format!("{} {}", signed(&flow.amount_exact), flow.asset)}</span></div>
            <dl><div><dt>"资产位置"</dt><dd><code>{flow.location.clone()}</code></dd></div>
                <div><dt>"交易依据"</dt><dd><code>{flow.source_id.clone()}</code></dd></div></dl>
        </li>
    }).collect_view();
    let problems = accounting
        .problems
        .iter()
        .map(|p| view! { <li>{p.clone()}</li> })
        .collect_view();
    view! {
        <section class="cross-chain-accounting" aria-label="本次资产净变动">
            <header><div><h3>"本次资产净变动"</h3><span>{status}</span></div>
                <div class="cross-chain-net-value"><small>"已记录收支折合"</small>
                    <strong class=value_tone>{amount.map_or_else(|| "暂不计总额".into(), |amount| format!("{amount} USD"))}</strong></div>
            </header>
            {accounting.net_assets.is_empty().then(|| view! { <p>"尚无非零净变动"</p> })}
            <div class="cross-chain-net-assets">{assets}</div>
            <p class="cross-chain-accounting-scope">{if accounting.external_flows.is_empty() { "路径处理结果收支（含已核实异常到账）；无非零已归集独立费用。不代表完整交易利润。" } else { "美元折算已扣下列独立费用；上方为路径钱包变化（含异常到账），不重复扣款。未选费用未包含，不代表完整交易利润。" }}</p>
            {(!accounting.external_flows.is_empty()).then(|| view! { <details class="cross-chain-accounting-detail"><summary>{format!("独立费用 · {} 笔", accounting.external_flows.len())}</summary><ol>{external}</ol></details> })}
            <details class="cross-chain-accounting-detail"><summary>{format!("收支明细 · {} 笔", accounting.flows.len())}</summary>
                <ol>{flows}</ol>{rates}
            </details>
            {(!accounting.problems.is_empty()).then(|| view! { <details class="cross-chain-accounting-detail is-warning"><summary>"待核项目"</summary><ul>{problems}</ul></details> })}
        </section>
    }
}
