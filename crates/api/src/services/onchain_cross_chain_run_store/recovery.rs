use super::*;
use crate::services::onchain_comparison::lifi::{address_matches, chain_id, token_matches};
use shared_types::{
    OnchainChainSettlementStatus as ReceiptStatus, OnchainCrossChainRecovery,
    OnchainExecutionToken, OnchainWalletReceiptBasis,
};

pub(crate) fn scope(
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    report: &OnchainCrossChainRecovery,
) -> Result<(String, String, String), String> {
    let bridge = leg
        .bridge_execution
        .as_ref()
        .ok_or("异常到账缺少原桥合同")?;
    let route = run
        .build
        .legs
        .iter()
        .find(|r| r.position == leg.position)
        .ok_or("原链路径缺失")?;
    if !matches!(
        (report.provider_status.as_str(), report.substatus.as_deref()),
        ("DONE", Some("PARTIAL" | "REFUNDED")) | ("FAILED", _)
    ) {
        return Err("不是可核验的桥异常终态".into());
    }
    if report.provider_transaction_id.as_deref() != Some(&bridge.transaction_id)
        || report.sending_chain_id != Some(bridge.from_chain_id)
        || report
            .sending_transaction_id
            .as_deref()
            .zip(leg.source_transaction_id.as_deref())
            .is_none_or(|(reported, saved)| !address_matches(bridge.from_chain_id, reported, saved))
        || chain_id(&route.from_chain) != Some(bridge.from_chain_id)
        || chain_id(&route.to_chain) != Some(bridge.to_chain_id)
    {
        return Err("桥异常报告与原交易编号、源哈希或链路径不一致，不能据此记账".into());
    }
    let receiving_chain = report
        .receiving_chain_id
        .ok_or("桥未提供异常到账链，不能假定退回源链")?;
    if report.receiving_token_chain_id != Some(receiving_chain) {
        return Err("异常到账代币的链身份缺失或不符".into());
    }
    let (chain, wallet) = if receiving_chain == bridge.to_chain_id {
        (&route.to_chain, &bridge.to_address)
    } else if receiving_chain == bridge.from_chain_id {
        (&route.from_chain, &bridge.from_address)
    } else {
        return Err("异常到账不在原路径的链上，需人工核对资产位置".into());
    };
    if report
        .reported_receiver
        .as_deref()
        .is_some_and(|receiver| !address_matches(receiving_chain, receiver, wallet))
    {
        return Err("桥报告接收钱包与原路径的钱包不符".into());
    }
    let hash = report
        .receiving_transaction_id
        .as_deref()
        .filter(|hash| !hash.trim().is_empty())
        .ok_or("桥报告尚未提供异常到账交易哈希")?;
    if run
        .legs
        .iter()
        .flat_map(|leg| {
            [
                leg.source_receipt.as_ref(),
                leg.destination_receipt.as_ref(),
            ]
        })
        .flatten()
        .any(|receipt| {
            receipt.basis.chain == *chain
                && address_matches(receiving_chain, &receipt.basis.wallet, wallet)
                && address_matches(receiving_chain, &receipt.basis.transaction_id, hash)
        })
    {
        return Err(
            "该钱包交易已计入原路径收支，不重复记录异常到账；需核对同笔交易内的资产变化".into(),
        );
    }
    let token = report
        .receiving_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or("桥未提供异常到账合约，不能用原计划币种代替")?;
    Ok((chain.clone(), wallet.clone(), token.into()))
}

pub(crate) fn basis(
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    report: &OnchainCrossChainRecovery,
) -> Result<OnchainWalletReceiptBasis, String> {
    let (chain, wallet, address) = scope(run, leg, report)?;
    let resolution = report
        .token_resolution
        .as_ref()
        .ok_or("异常到账合约精度尚未读取")?;
    let id = chain_id(&chain).ok_or("异常到账链未登记")?;
    if resolution.chain != chain
        || !token_matches(id, &resolution.address, &address)
        || resolution.decimals > 28
        || resolution.observed_at_ms <= 0
        || resolution.precision_source.is_empty()
        || resolution.precision_evidence_url.is_empty()
        || resolution.identity.as_ref().is_some_and(|identity| {
            identity.chain != chain
                || !token_matches(id, &identity.address, &address)
                || identity.decimals != resolution.decimals
        })
    {
        return Err("异常到账合约的精度证据不一致，不能折算数量".into());
    }
    Ok(OnchainWalletReceiptBasis {
        chain,
        wallet,
        transaction_id: report.receiving_transaction_id.clone().unwrap(),
        require_sender: false,
        assets: vec![OnchainExecutionToken {
            symbol: resolution
                .identity
                .as_ref()
                .map_or_else(|| "TOKEN".into(), |identity| identity.symbol.clone()),
            address: resolution.address.clone(),
            decimals: resolution.decimals,
        }],
    })
}

impl OnchainCrossChainRunStore {
    pub(crate) fn record_bridge_recovery(
        &self,
        run_id: &str,
        position: u8,
        mut report: OnchainCrossChainRecovery,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(
            run_id,
            |run| {
                if run.active_position != Some(position)
                    || !matches!(
                        run.status,
                        OnchainCrossChainRunStatus::AwaitingSourceFinality
                            | OnchainCrossChainRunStatus::AwaitingDestinationEvidence
                    )
                {
                    return Err("过期异常到账核验不得恢复其他步骤或已暂停运行".into());
                }
                let index = run
                    .legs
                    .iter()
                    .position(|leg| leg.position == position)
                    .ok_or("跨链步骤不存在")?;
                if report.observed_at_ms <= 0 || report.observed_at_ms > now_ms {
                    return Err("桥异常报告核验时间无效".into());
                }
                if let Some(receipt) = &report.receipt {
                    if receipt.basis != basis(run, &run.legs[index], &report)?
                        || receipt.asset_changes_raw.len() != 1
                    {
                        return Err("异常到账回执与原桥报告、钱包或合约不符".into());
                    }
                    if receipt.status != ReceiptStatus::Pending
                        && (receipt.block_ref.as_deref().is_none_or(str::is_empty)
                            || receipt
                                .observed_at_ms
                                .is_none_or(|time| time <= 0 || time > now_ms))
                    {
                        return Err("异常到账回执缺少区块或核验时间".into());
                    }
                    if receipt.status == ReceiptStatus::Complete
                        && (receipt.problem.is_some()
                            || receipt.asset_changes_raw[0]
                                .as_deref()
                                .is_none_or(|raw| raw.parse::<i128>().is_err())
                            || receipt.network_cost.as_ref().is_none_or(|fee| {
                                fee.problem.is_some()
                                    || fee.total_fee_exact.is_none()
                                    || fee.chain != receipt.basis.chain
                                    || fee.transaction_id != receipt.basis.transaction_id
                                    || Some(fee.block_ref.as_str()) != receipt.block_ref.as_deref()
                                    || fee.payer.is_empty()
                            }))
                    {
                        return Err("异常到账完整回执缺少费用或实际数量".into());
                    }
                }
                let leg = &mut run.legs[index];
                if let Some(old) = leg.bridge_recovery.as_ref().filter(|old| {
                    old.receipt
                        .as_ref()
                        .is_some_and(|r| r.status == ReceiptStatus::Complete)
                }) {
                    if old.receipt != report.receipt
                        || old.token_resolution != report.token_resolution
                        || old.receiving_transaction_id != report.receiving_transaction_id
                        || old.receiving_token != report.receiving_token
                    {
                        return Err("不得覆盖已确认的异常到账收支".into());
                    }
                    report = old.clone();
                } else if let Some(old_receipt) = leg
                    .bridge_recovery
                    .as_ref()
                    .and_then(|r| r.receipt.as_ref())
                {
                    if let Some(new) = report
                        .receipt
                        .as_mut()
                        .filter(|r| r.status == ReceiptStatus::Pending && r.block_ref.is_none())
                    {
                        if new.basis == old_receipt.basis {
                            let problem = new.problem.take();
                            *new = old_receipt.clone();
                            new.problem = problem;
                        }
                    }
                }
                let pending = report
                    .receipt
                    .as_ref()
                    .is_some_and(|r| r.status == ReceiptStatus::Pending)
                    || report.receipt.is_none() && report.problem.is_none();
                let problem = report.problem.clone().unwrap_or_else(|| {
                    if pending {
                        "桥报告异常终态，正在核对实际资产与费用".into()
                    } else {
                        "异常路径收支已保留；原套利路径停止，不视为盈利闭环".into()
                    }
                });
                leg.bridge_recovery = Some(report);
                leg.last_checked_at_ms = Some(now_ms);
                leg.problem = Some(problem.clone());
                run.problem = Some(problem);
                if pending {
                    leg.status = OnchainCrossChainLegRunStatus::SourceConfirmed;
                    run.status = OnchainCrossChainRunStatus::AwaitingDestinationEvidence;
                    run.next_action = "核验退款或部分到账，不再推进原套利路径".into();
                } else {
                    leg.status = OnchainCrossChainLegRunStatus::Paused;
                    run.status = OnchainCrossChainRunStatus::Paused;
                    run.next_action = "按已确认资产重新规划处置；未自动兑换、桥接或补偿转账".into();
                }
                Ok(())
            },
            now_ms,
        )
    }
}

pub(super) fn preserves_confirmed(old: &OnchainCrossChainRun, new: &OnchainCrossChainRun) -> bool {
    old.legs.iter().all(|old_leg| {
        let Some(report) = old_leg.bridge_recovery.as_ref().filter(|report| {
            report
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.status == ReceiptStatus::Complete)
        }) else {
            return true;
        };
        new.legs
            .iter()
            .find(|leg| leg.position == old_leg.position)
            .and_then(|leg| leg.bridge_recovery.as_ref())
            == Some(report)
    })
}

#[cfg(test)]
pub(crate) mod tests;
