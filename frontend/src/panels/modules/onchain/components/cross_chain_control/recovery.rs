use super::*;
use shared_types::{OnchainChainSettlementStatus as ReceiptStatus, OnchainCrossChainRecovery};

pub(super) fn summary(report: &OnchainCrossChainRecovery) -> impl IntoView {
    let receipt = report.receipt.as_ref();
    let asset = receipt.and_then(|receipt| receipt.basis.assets.first());
    let raw = receipt
        .and_then(|receipt| receipt.asset_changes_raw.first())
        .and_then(Option::as_deref);
    let status = match receipt {
        Some(receipt)
            if receipt.status == ReceiptStatus::Complete
                && raw
                    .and_then(|raw| raw.parse::<i128>().ok())
                    .is_some_and(|amount| amount > 0) =>
        {
            "钱包已核实到账"
        }
        Some(receipt) if receipt.status == ReceiptStatus::Pending && raw.is_some() => {
            "数量已读取 · 收支待核齐"
        }
        _ => "钱包到账未确认",
    };
    let claimed = report.reported_amount_raw.as_deref().map_or_else(
        || "未提供".into(),
        |raw| {
            asset.map_or_else(
                || format!("{raw} 最小单位 · 精度待核"),
                |asset| raw_amount_label(raw, asset.decimals, &asset.symbol),
            )
        },
    );
    let actual = raw.zip(asset).map_or_else(
        || "尚未确认".into(),
        |(raw, asset)| raw_amount_label(raw, asset.decimals, &asset.symbol),
    );
    let chain = receipt
        .map(|receipt| chain_label(&receipt.basis.chain).to_string())
        .or_else(|| {
            report
                .receiving_chain_id
                .map(|id| format!("链 ID {id}，待核对"))
        })
        .unwrap_or_else(|| "尚未提供，不能假定原链".into());
    let wallet = receipt
        .map(|receipt| receipt.basis.wallet.clone())
        .or_else(|| report.reported_receiver.clone())
        .unwrap_or_else(|| "尚未确认".into());
    let token = asset
        .map(|asset| asset.address.clone())
        .or_else(|| report.receiving_token.clone())
        .unwrap_or_else(|| "尚未提供".into());
    let hash = receipt
        .map(|receipt| receipt.basis.transaction_id.clone())
        .or_else(|| report.receiving_transaction_id.clone())
        .unwrap_or_else(|| "尚未提供".into());
    view! {
        <details class="cross-chain-accounting-detail is-warning" open>
            <summary>{format!("退款 / 异常到账 · {status}")}</summary>
            <dl class="cross-chain-leg-amounts">
                <div><dt>"桥报告"</dt><dd>{format!("{} · {}", report.provider_status, report.substatus.as_deref().unwrap_or("未知"))}</dd></div>
                <div><dt>"桥报告数量"</dt><dd>{claimed}</dd></div>
                <div><dt>"钱包净到账"</dt><dd>{actual}</dd></div>
            </dl>
            <dl class="cross-chain-transactions">
                <div><dt>"接收链"</dt><dd>{chain}</dd></div>
                <div><dt>{if receipt.is_some() { "核对钱包" } else { "桥报告接收地址" }}</dt><dd><code>{wallet}</code></dd></div>
                <div><dt>{if asset.is_some() { "核对合约" } else { "桥报告合约" }}</dt><dd><code>{token}</code></dd></div>
                <div><dt>"接收交易"</dt><dd><code>{hash}</code></dd></div>
            </dl>
            <div class="cross-chain-receipts">{wallet_receipt("异常到账收支", receipt)}</div>
            <p class="cross-chain-notice">"原套利路径已停止；按确认资产重新规划，不自动再次转账。"</p>
        </details>
    }
}
