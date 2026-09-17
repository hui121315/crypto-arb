use super::*;
use shared_types::{
    OnchainChainSettlementStatus as ReceiptStatus, OnchainExecutionToken, OnchainWalletReceipt,
    OnchainWalletReceiptBasis,
};

pub(crate) fn basis(
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    destination_transaction: Option<&str>,
) -> Result<OnchainWalletReceiptBasis, String> {
    let route = run
        .build
        .legs
        .iter()
        .find(|v| v.position == leg.position)
        .ok_or("跨链回执缺少已授权路径")?;
    let source_hash = leg
        .source_transaction_id
        .as_deref()
        .ok_or("尚未记录提交哈希")?;
    let token = |symbol: &str, address: &str, decimals| OnchainExecutionToken {
        symbol: symbol.into(),
        address: address.into(),
        decimals,
    };
    if let Some(swap) = &leg.swap_execution {
        if destination_transaction.is_some()
            || swap.chain != route.from_chain
            || swap.chain != route.to_chain
            || swap.kind != leg.kind
        {
            return Err("同链兑换回执与已授权路径不一致".into());
        }
        Ok(OnchainWalletReceiptBasis {
            chain: swap.chain.clone(),
            wallet: swap.wallet_address.clone(),
            transaction_id: source_hash.into(),
            require_sender: true,
            assets: vec![
                token(&route.from_asset, &swap.input_token, route.input_decimals),
                token(&route.to_asset, &swap.output_token, route.output_decimals),
            ],
        })
    } else if let Some(bridge) = &leg.bridge_execution {
        if bridge.kind != leg.kind {
            return Err("桥回执与已授权步骤不一致".into());
        }
        let (chain, wallet, hash, asset) = if let Some(hash) = destination_transaction {
            (
                &route.to_chain,
                &bridge.to_address,
                hash,
                token(&route.to_asset, &bridge.to_token, route.output_decimals),
            )
        } else {
            (
                &route.from_chain,
                &bridge.from_address,
                source_hash,
                token(&route.from_asset, &bridge.from_token, route.input_decimals),
            )
        };
        Ok(OnchainWalletReceiptBasis {
            chain: chain.clone(),
            wallet: wallet.clone(),
            transaction_id: hash.into(),
            assets: vec![asset],
            require_sender: destination_transaction.is_none(),
        })
    } else {
        Err("缺少已持久化的交易合同".into())
    }
}

impl OnchainCrossChainRunStore {
    pub(crate) fn record_wallet_receipt(
        &self,
        run_id: &str,
        expected_position: u8,
        mut receipt: OnchainWalletReceipt,
        destination: bool,
        provider_output_raw: Option<&str>,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(run_id, |run| {
            if run.active_position != Some(expected_position)
                || !matches!(run.status, OnchainCrossChainRunStatus::AwaitingSourceFinality
                    | OnchainCrossChainRunStatus::AwaitingDestinationEvidence) {
                return Err("过期回执不得推进其他步骤或恢复已暂停运行".into());
            }
            let index = run.legs.iter().position(|v| v.position == expected_position)
                .ok_or("跨链步骤不存在")?;
            let leg = &run.legs[index];
            if leg.bridge_recovery.is_some() {
                return Err("桥异常终态已记录，不能用正常到账回执继续原套利路径".into());
            }
            let expected = basis(run, leg, destination.then_some(receipt.basis.transaction_id.as_str()))?;
            if receipt.basis != expected || receipt.asset_changes_raw.len() != expected.assets.len() {
                return Err("回执的链、钱包、哈希或资产与持久化合同不一致".into());
            }
            if destination && leg.source_receipt.as_ref()
                .is_none_or(|v| v.status == ReceiptStatus::ReviewRequired || v.block_ref.is_none()
                    || leg.actual_input_amount_raw.is_none()) {
                return Err("源链实际扣款尚未确认".into());
            }
            if receipt.status != ReceiptStatus::Pending
                && (receipt.block_ref.as_deref().is_none_or(str::is_empty)
                    || receipt.observed_at_ms.is_none_or(|v| v <= 0 || v > now_ms)) {
                return Err("回执缺少有效区块与核验时间".into());
            }
            if receipt.status == ReceiptStatus::Complete && (receipt.problem.is_some()
                || receipt.asset_changes_raw.iter().any(Option::is_none)
                || receipt.network_cost.as_ref().is_none_or(|cost|
                    cost.total_fee_exact.is_none() || cost.problem.is_some()
                    || cost.chain != expected.chain || cost.transaction_id != expected.transaction_id
                    || Some(cost.block_ref.as_str()) != receipt.block_ref.as_deref()
                    || cost.payer.trim().is_empty())) {
                return Err("完整回执缺少实际费用或费用身份不一致".into());
            }
            let saved = if destination { &leg.destination_receipt } else { &leg.source_receipt };
            if receipt.status == ReceiptStatus::Pending && receipt.block_ref.is_none() {
                if let Some(old) = saved.as_ref().filter(|v| v.basis == receipt.basis && v.status == ReceiptStatus::Pending) {
                    let problem = receipt.problem.take();
                    receipt = old.clone();
                    receipt.problem = problem;
                }
            }
            let changes = receipt.asset_changes_raw.iter().map(|v|
                v.as_deref().map(str::parse::<i128>).transpose().map_err(|_| "回执数量非法".to_owned()))
                .collect::<Result<Vec<_>, _>>()?;
            let leg = &mut run.legs[index];
            let saved = if destination { &mut leg.destination_receipt } else { &mut leg.source_receipt };
            if saved.as_ref().is_some_and(|old| old.status == ReceiptStatus::Complete && old != &receipt) {
                return Err("不得改写已确认回执".into());
            }
            *saved = Some(receipt.clone());
            leg.receipt_checks = leg.receipt_checks.saturating_add(1);
            if destination { leg.bridge_reported_output_amount_raw = provider_output_raw.map(str::to_owned); }
            leg.last_checked_at_ms = Some(now_ms);
            leg.evidence_source = Some("canonical_wallet_receipt".into());
            let debit = (!destination).then(|| changes[0]).flatten();
            if let Some(debit) = debit.filter(|v| *v <= 0) {
                leg.actual_input_amount_raw = Some(debit.unsigned_abs().to_string());
            }
            let output = if destination { changes[0] } else { changes.get(1).copied().flatten() };
            if let Some(output) = output.filter(|v| *v >= 0) {
                leg.actual_output_amount_raw = Some(output.to_string());
                leg.destination_transaction_id = Some(receipt.basis.transaction_id.clone());
            }
            let maximum = leg.submitted_input_amount_raw.as_deref()
                .and_then(|v| v.parse::<u128>().ok()).ok_or("缺少提交数量上限")?;
            let minimum = leg.minimum_output_amount_raw.as_deref()
                .and_then(|v| v.parse::<u128>().ok()).ok_or("缺少最低到账约束")?;
            let amount_problem = if debit.is_some_and(|v| v >= 0 || v.unsigned_abs() > maximum) {
                Some("实际扣款方向不符或超过提交上限")
            } else if output.is_some_and(|v| v <= 0 || (v as u128) < minimum) {
                Some("实际到账低于已授权最低数量")
            } else if destination && output.is_some_and(|v| provider_output_raw
                .and_then(|raw| raw.parse::<i128>().ok()) != Some(v)) {
                Some("桥服务数量与钱包净到账不一致，需核对批量转账或扣费")
            } else { None };
            if receipt.status == ReceiptStatus::ReviewRequired || amount_problem.is_some() {
                let problem = receipt.problem.clone().or_else(|| amount_problem.map(str::to_owned))
                    .unwrap_or_else(|| "回执需要人工复核".into());
                leg.problem = Some(problem.clone());
                leg.status = OnchainCrossChainLegRunStatus::Paused;
                run.status = OnchainCrossChainRunStatus::Paused;
                run.problem = Some(problem);
                run.next_action = "已保留真实收支；只读复核后决定剩余资产处置，不重复提交".into();
            } else if receipt.status == ReceiptStatus::Pending || destination
                && leg.source_receipt.as_ref().is_none_or(|v| v.status != ReceiptStatus::Complete) {
                let problem = receipt.problem.clone().or_else(|| Some("目标资产已到账；源链费用仍待核清".into()));
                leg.problem = problem.clone();
                run.problem = problem;
                run.next_action = "已保留可确认数量；完整收支与费用仍待核验".into();
            } else if output.is_none() {
                leg.status = OnchainCrossChainLegRunStatus::SourceConfirmed;
                leg.problem = None;
                run.status = OnchainCrossChainRunStatus::AwaitingDestinationEvidence;
                run.problem = None;
                run.next_action = "源链扣款与链费已确认；等待目标钱包到账回执".into();
            } else {
                leg.status = OnchainCrossChainLegRunStatus::Completed;
                leg.problem = None;
                run.active_position = None;
                run.problem = None;
                if run.legs.iter().all(|v| v.status == OnchainCrossChainLegRunStatus::Completed) {
                    run.status = OnchainCrossChainRunStatus::Completed;
                    run.next_action = "资产路径已完成；各腿收支与链费已记录，尚不代表扣除全部成本后的美元盈利".into();
                } else {
                    run.status = OnchainCrossChainRunStatus::Running;
                    run.next_action = format!("第 {expected_position} 腿收支与费用已确认；下一腿按真实到账重新报价");
                }
            }
            Ok(())
        }, now_ms)
    }
}
