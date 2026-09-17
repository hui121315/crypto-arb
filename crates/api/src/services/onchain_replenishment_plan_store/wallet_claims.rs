use super::*;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner, WalletClaims, WalletScope};
use std::{collections::BTreeSet, sync::Arc};

impl OnchainReplenishmentPlanStore {
    pub(crate) fn with_wallet_claims(mut self, shared: Arc<WalletClaims>) -> Self {
        self.wallet_claims = shared;
        self.restore_wallet_claims();
        self
    }

    pub(super) fn restore_wallet_claims(&self) {
        let claims = (|| {
            let mut claims = Vec::new();
            for run in &self.runs {
                if let Some(hold) = hold(&run)? {
                    claims.push((Owner::new(Module::Replenishment, &run.run_id), hold));
                }
            }
            Ok(claims)
        })();
        self.wallet_claims.restore(
            Module::Replenishment,
            claims,
            self.path.as_ref().and_then(|_| self.readiness().err()),
        );
    }

    pub(super) fn is_read_only_wallet_update(&self, next: &OnchainReplenishmentRun) -> bool {
        let Some(prior) = self.runs.get(&next.run_id) else {
            return false;
        };
        next.read_only_recovery
            && next.plan == prior.plan
            && next.authorization == prior.authorization
            && next.idempotency_key == prior.idempotency_key
            && next.created_at_ms == prior.created_at_ms
            && next.transfers.len() == prior.transfers.len()
            && next
                .transfers
                .iter()
                .zip(&prior.transfers)
                .all(|(next, prior)| {
                    next.leg_index == prior.leg_index
                        && next.client_transfer_id == prior.client_transfer_id
                        && next.submission_attempted_at_ms == prior.submission_attempted_at_ms
                        && prior
                            .transaction_id
                            .as_ref()
                            .is_none_or(|hash| next.transaction_id.as_ref() == Some(hash))
                        && prior
                            .provider_transfer_id
                            .as_ref()
                            .is_none_or(|id| next.provider_transfer_id.as_ref() == Some(id))
                })
    }
}

pub(super) fn hold(run: &OnchainReplenishmentRun) -> Result<Option<Hold>, String> {
    let mut wallets = BTreeSet::new();
    for transfer in &run.transfers {
        if matches!(
            transfer.status,
            OnchainReplenishmentTransferStatus::DestinationCredited
                | OnchainReplenishmentTransferStatus::Failed
        ) {
            continue;
        }
        let leg = run
            .plan
            .legs
            .get(transfer.leg_index as usize)
            .ok_or("补库记录缺少原始来源计划")?;
        if leg.direction == shared_types::OnchainTransferDirection::DepositToCex {
            wallets.insert(WalletScope::new(
                &leg.chain,
                leg.source_address
                    .as_deref()
                    .ok_or("链上补库来源钱包缺失")?,
            )?);
        }
    }
    Ok((!wallets.is_empty()).then_some(Hold {
        wallets,
        expires_at_ms: None,
    }))
}

#[cfg(test)]
mod tests;
