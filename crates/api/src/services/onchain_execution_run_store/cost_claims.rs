use crate::services::onchain_comparison::approval_allocation;
use shared_types::{
    OnchainExecutionApprovalCost as ApprovalCost, OnchainExecutionReplenishmentCost as Cost,
    OnchainExecutionSubmitResponse as Run,
};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub(super) struct CostClaims {
    runs: BTreeMap<String, Vec<Cost>>,
    approvals: BTreeMap<String, Vec<ApprovalCost>>,
    owners: BTreeMap<String, String>,
    cross_chain: BTreeMap<String, super::CrossChainCostClaim>,
}

impl CostClaims {
    pub(super) fn approval_available(
        &self,
        costs: &[ApprovalCost],
        execution: Option<&str>,
    ) -> Result<(), String> {
        for cost in costs {
            for key in approval_allocation::keys(cost) {
                if let Some(owner) = self.owners.get(&key) {
                    if Some(owner.as_str()) != execution {
                        return Err(format!(
                            "授权 {} 的费用已归入执行 {owner}，不能重复归集",
                            cost.run.run_id
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn approval_owner(&self, id: &str) -> Option<String> {
        self.owners.get(&format!("approval-run:{id}")).cloned()
    }

    pub(super) fn available(&self, costs: &[Cost], execution: Option<&str>) -> Result<(), String> {
        for cost in costs {
            for key in keys(cost) {
                if let Some(owner) = self.owners.get(&key) {
                    if Some(owner.as_str()) != execution {
                        return Err(format!(
                            "补库 {} 的费用已归入执行 {owner}，不能重复归集",
                            cost.run_id
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate(&self, run: &Run) -> Result<(), String> {
        self.validate_costs(&run.run_id, &run.replenishment_costs, &run.approval_costs)
    }

    fn validate_costs(&self, owner: &str, replenishments: &[Cost], approvals: &[ApprovalCost]) -> Result<(), String> {
        approval_allocation::fees(approvals)?;
        crate::services::onchain_comparison::replenishment_allocation::fees(
            replenishments,
        )?;
        if self
            .runs
            .get(owner)
            .is_some_and(|old| old != replenishments)
        {
            return Err("已保存执行的补库费用归属不可删除、替换或追加".into());
        }
        if self
            .approvals
            .get(owner)
            .is_some_and(|old| old != approvals)
        {
            return Err("已保存执行的授权费用归属不可删除、替换或追加".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for key in replenishments.iter().flat_map(keys).chain(
            approvals
                .iter()
                .flat_map(approval_allocation::keys),
        ) {
            if !seen.insert(key) {
                return Err("同一链上交易不能同时归入补库与授权费用".into());
            }
        }
        self.approval_available(approvals, Some(owner))?;
        self.available(replenishments, Some(owner))
    }

    pub(super) fn record(&mut self, run: &Run) {
        self.record_costs(&run.run_id, &run.replenishment_costs, &run.approval_costs);
    }

    fn record_costs(&mut self, owner: &str, replenishments: &[Cost], approvals: &[ApprovalCost]) {
        if self.runs.contains_key(owner) {
            return;
        }
        for cost in replenishments {
            for key in keys(cost) {
                self.owners.insert(key, owner.to_owned());
            }
        }
        for cost in approvals {
            for key in approval_allocation::keys(cost) {
                self.owners.insert(key, owner.to_owned());
            }
        }
        self.approvals
            .insert(owner.to_owned(), approvals.to_vec());
        self.runs
            .insert(owner.to_owned(), replenishments.to_vec());
    }

    pub(super) fn validate_cross_chain(&self, claim: &super::CrossChainCostClaim) -> Result<bool, String> {
        if claim.run_id.is_empty() || claim.build_id.is_empty() {
            return Err("跨链费用归属标识缺失".into());
        }
        if let Some(old) = self.cross_chain.get(&claim.run_id) {
            if old != claim { return Err("跨链费用归属不可替换、删除或追加".into()); }
            return Ok(true);
        }
        self.validate_costs(&format!("cross-chain:{}", claim.run_id), &claim.replenishment_costs, &claim.approval_costs)?;
        Ok(false)
    }

    pub(super) fn record_cross_chain(&mut self, claim: &super::CrossChainCostClaim) {
        self.record_costs(&format!("cross-chain:{}", claim.run_id), &claim.replenishment_costs, &claim.approval_costs);
        self.cross_chain.insert(claim.run_id.clone(), claim.clone());
    }
}

fn keys(cost: &Cost) -> impl Iterator<Item = String> + '_ {
    std::iter::once(format!("run:{}", cost.run_id))
        .chain(cost.transfer_ids.iter().map(|id| format!("transfer:{id}")))
}

#[cfg(test)]
mod tests;
