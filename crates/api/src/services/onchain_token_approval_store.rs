use dashmap::DashMap;
use shared_types::{OnchainComparisonConfig, OnchainTokenApprovalBuildResponse};

const MAX_STORED_APPROVALS: usize = 64;

#[derive(Debug, Clone)]
struct StoredApproval {
    response: OnchainTokenApprovalBuildResponse,
    config: OnchainComparisonConfig,
    claimed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedTokenApproval {
    pub(crate) response: OnchainTokenApprovalBuildResponse,
    pub(crate) config: OnchainComparisonConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApprovalClaimError {
    Missing,
    Expired,
    AlreadyClaimed(String),
}

#[derive(Debug, Default)]
pub(crate) struct OnchainTokenApprovalStore {
    approvals: DashMap<String, StoredApproval>,
}

impl OnchainTokenApprovalStore {
    pub(crate) fn insert(
        &self,
        response: OnchainTokenApprovalBuildResponse,
        config: OnchainComparisonConfig,
        now_ms: i64,
    ) {
        self.prune(now_ms);
        self.approvals.insert(
            response.approval_id.clone(),
            StoredApproval {
                response,
                config,
                claimed_by: None,
            },
        );
        if self.approvals.len() > MAX_STORED_APPROVALS {
            self.remove_oldest();
        }
    }

    pub(crate) fn claim(
        &self,
        approval_id: &str,
        run_id: &str,
        now_ms: i64,
    ) -> Result<ClaimedTokenApproval, ApprovalClaimError> {
        let mut entry = self
            .approvals
            .get_mut(approval_id)
            .ok_or(ApprovalClaimError::Missing)?;
        if entry.response.valid_until_ms < now_ms {
            drop(entry);
            self.approvals.remove(approval_id);
            return Err(ApprovalClaimError::Expired);
        }
        if let Some(existing) = entry.claimed_by.as_ref() {
            return Err(ApprovalClaimError::AlreadyClaimed(existing.clone()));
        }
        entry.claimed_by = Some(run_id.to_owned());
        Ok(ClaimedTokenApproval {
            response: entry.response.clone(),
            config: entry.config.clone(),
        })
    }

    pub(crate) fn release(&self, approval_id: &str, run_id: &str) {
        let Some(mut entry) = self.approvals.get_mut(approval_id) else {
            return;
        };
        if entry.claimed_by.as_deref() == Some(run_id) {
            entry.claimed_by = None;
        }
    }

    pub(crate) fn finish(&self, approval_id: &str, run_id: &str) {
        let remove = self
            .approvals
            .get(approval_id)
            .is_some_and(|entry| entry.claimed_by.as_deref() == Some(run_id));
        if remove {
            self.approvals.remove(approval_id);
        }
    }

    fn prune(&self, now_ms: i64) {
        self.approvals
            .retain(|_, entry| entry.response.valid_until_ms >= now_ms);
    }

    fn remove_oldest(&self) {
        let oldest = self
            .approvals
            .iter()
            .min_by_key(|entry| entry.response.built_at_ms)
            .map(|entry| entry.key().clone());
        if let Some(approval_id) = oldest {
            self.approvals.remove(&approval_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use shared_types::{OnchainComparisonDirection, OnchainTokenApprovalBuildResponse};

    use super::*;

    fn approval(valid_until_ms: i64) -> OnchainTokenApprovalBuildResponse {
        OnchainTokenApprovalBuildResponse {
            approval_id: "approval-1".to_owned(),
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            provider: "zeroex_swap_v2".to_owned(),
            chain: "base".to_owned(),
            wallet_address: "0x1111111111111111111111111111111111111111".to_owned(),
            token_address: "0x2222222222222222222222222222222222222222".to_owned(),
            token_symbol: "USDC".to_owned(),
            token_decimals: 6,
            spender: "0x3333333333333333333333333333333333333333".to_owned(),
            required_amount_raw: "1000000".to_owned(),
            current_allowance_raw: "0".to_owned(),
            transactions: Vec::new(),
            built_at_ms: 10,
            valid_until_ms,
            official_docs_url: "https://eips.ethereum.org/EIPS/eip-20".to_owned(),
            approval_required: false,
            submit_ready: false,
            blockers: Vec::new(),
        }
    }

    #[test]
    fn claim_is_one_time_and_expired_plans_are_removed() {
        let store = OnchainTokenApprovalStore::default();
        store.insert(approval(20), OnchainComparisonConfig::default(), 10);
        assert!(store.claim("approval-1", "run-1", 15).is_ok());
        assert_eq!(
            store.claim("approval-1", "run-2", 15),
            Err(ApprovalClaimError::AlreadyClaimed("run-1".to_owned()))
        );
        store.release("approval-1", "run-1");
        assert!(store.claim("approval-1", "run-2", 15).is_ok());
        store.finish("approval-1", "run-2");
        assert_eq!(
            store.claim("approval-1", "run-3", 15),
            Err(ApprovalClaimError::Missing)
        );

        store.insert(approval(20), OnchainComparisonConfig::default(), 10);
        assert_eq!(
            store.claim("approval-1", "run-4", 21),
            Err(ApprovalClaimError::Expired)
        );
    }
}
