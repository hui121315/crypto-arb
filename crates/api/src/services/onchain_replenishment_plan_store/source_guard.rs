use super::*;
use shared_types::{OnchainReplenishmentLeg, OnchainTransferDirection};

#[derive(Debug, Clone, PartialEq, Eq)]
enum FundingSource {
    Spot { venue: String, asset: String },
    Chain { chain: String, wallet: String },
}

impl FundingSource {
    fn from_leg(leg: &OnchainReplenishmentLeg) -> Option<Self> {
        match leg.direction {
            OnchainTransferDirection::WithdrawToChain => {
                let venue = shared_types::normalized_venue_name(&leg.venue);
                let asset = leg.asset.trim().to_ascii_uppercase();
                if venue.is_empty() || asset.is_empty() {
                    return None;
                }
                // All replenishment withdrawal adapters use the configured Spot account.
                Some(Self::Spot { venue, asset })
            }
            OnchainTransferDirection::DepositToCex => {
                let preset = shared_types::onchain_chain_preset(&leg.chain)?;
                let wallet = leg.source_address.as_deref()?.trim();
                if wallet.is_empty() {
                    return None;
                }
                let wallet = if preset.chain_id.is_some() {
                    wallet.to_ascii_lowercase()
                } else {
                    wallet.to_owned()
                };
                // Different tokens still share Gas and the wallet's transaction sequence.
                Some(Self::Chain {
                    chain: preset.id.into(),
                    wallet,
                })
            }
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Spot { venue, asset } => {
                format!("{} 现货账户 {asset}", venue.to_ascii_uppercase())
            }
            Self::Chain { chain, .. } => format!("{chain} 来源钱包"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceActivity {
    source: Option<FundingSource>,
    run_id: String,
    client_transfer_id: String,
    status: OnchainReplenishmentTransferStatus,
}

#[derive(Debug)]
pub(crate) struct ReplenishmentSubmissionSnapshot(Vec<SourceActivity>);

impl ReplenishmentSubmissionSnapshot {
    pub(crate) fn ensure_available(
        &self,
        leg: &OnchainReplenishmentLeg,
    ) -> Result<(), ReplenishmentSubmitClaimError> {
        let source =
            FundingSource::from_leg(leg).ok_or(ReplenishmentSubmitClaimError::InvalidLeg)?;
        if let Some(row) = self.0.iter().find(|row| {
            row.source.as_ref().is_none_or(|value| value == &source)
                && !matches!(
                    row.status,
                    OnchainReplenishmentTransferStatus::DestinationCredited
                        | OnchainReplenishmentTransferStatus::Failed
                )
        }) {
            return Err(ReplenishmentSubmitClaimError::SourceBusy(format!(
                "{} 尚有未核清补库记录 {}（转账 {}）；本次未发送资金请求。请先核验原转账，不能换计划重复发送",
                source.label(), row.run_id, row.client_transfer_id
            )));
        }
        Ok(())
    }
}

impl OnchainReplenishmentPlanStore {
    // Capture before balance/nonce reads; compare atomically with the durable claim.
    pub(crate) fn submission_snapshot(&self) -> ReplenishmentSubmissionSnapshot {
        let _guard = self.ledger_lock.lock();
        self.submission_snapshot_unlocked()
    }

    fn submission_snapshot_unlocked(&self) -> ReplenishmentSubmissionSnapshot {
        let mut rows = Vec::new();
        for run in &self.runs {
            for transfer in &run.transfers {
                rows.push(SourceActivity {
                    source: run
                        .plan
                        .legs
                        .get(transfer.leg_index as usize)
                        .and_then(FundingSource::from_leg),
                    run_id: run.run_id.clone(),
                    client_transfer_id: transfer.client_transfer_id.clone(),
                    status: transfer.status,
                });
            }
        }
        rows.sort_by(|a, b| {
            (&a.run_id, &a.client_transfer_id).cmp(&(&b.run_id, &b.client_transfer_id))
        });
        ReplenishmentSubmissionSnapshot(rows)
    }

    pub(super) fn check_submission_source(
        &self,
        leg: &OnchainReplenishmentLeg,
        before: &ReplenishmentSubmissionSnapshot,
    ) -> Result<(), ReplenishmentSubmitClaimError> {
        let source =
            FundingSource::from_leg(leg).ok_or(ReplenishmentSubmitClaimError::InvalidLeg)?;
        let after = self.submission_snapshot_unlocked();
        after.ensure_available(leg)?;
        let relevant =
            |row: &&SourceActivity| row.source.as_ref().is_none_or(|value| value == &source);
        let current = after.0.iter().filter(relevant).collect::<Vec<_>>();
        if before.0.iter().filter(relevant).collect::<Vec<_>>() != current {
            return Err(ReplenishmentSubmitClaimError::SourceChanged);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
