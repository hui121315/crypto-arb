use super::*;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner, WalletClaims};
use std::sync::Arc;

impl OnchainTokenApprovalRunStore {
    pub(crate) fn with_wallet_claims(mut self, shared: Arc<WalletClaims>) -> Self {
        self.wallet_claims = shared;
        self.restore_wallet_claims();
        self
    }

    pub(super) fn restore_wallet_claims(&self) {
        let claims = (|| {
            let mut claims = Vec::new();
            for row in self.rows.lock().values() {
                if let Some(hold) = hold(row)? {
                    claims.push((Owner::new(Module::Approval, &row.response.run_id), hold));
                }
            }
            Ok(claims)
        })();
        self.wallet_claims.restore(
            Module::Approval,
            claims,
            self.path.as_ref().and_then(|_| self.readiness().err()),
        );
    }
}

pub(super) fn hold(row: &ApprovalRecord) -> Result<Option<Hold>, String> {
    let unresolved = row.response.transaction_ids.iter().any(|hash| {
        !row.confirmed.contains(hash)
            && !row.response.fee_receipts.iter().any(|r| {
                r.basis.transaction_id == *hash
                    && r.status == ReceiptStatus::ReviewRequired
                    && complete_cost(r)
            })
    });
    if unresolved
        || matches!(
            row.response.status,
            Status::AwaitingFinality | Status::FinalityUnresolved
        )
    {
        Hold::wallet(&row.plan.chain, &row.plan.wallet_address, None).map(Some)
    } else {
        Ok(None)
    }
}
