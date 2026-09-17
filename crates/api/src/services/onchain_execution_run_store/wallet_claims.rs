use super::*;
use crate::services::onchain_wallet_claims::{Hold, Module, WalletClaims};
use std::sync::Arc;

impl OnchainExecutionRunStore {
    pub(crate) fn with_wallet_claims(mut self, shared: Arc<WalletClaims>) -> Self {
        self.wallet_claims = shared;
        self.restore_wallet_claims();
        self
    }

    pub(super) fn restore_wallet_claims(&self) {
        let claims = self
            .checkpoints
            .lock()
            .values()
            .map(|c| {
                Hold::wallet(&c.build.chain, &c.build.wallet_address, None).map(|h| {
                    (
                        crate::services::onchain_wallet_claims::Owner::new(
                            Module::Execution,
                            &c.response.run_id,
                        ),
                        h,
                    )
                })
            })
            .collect();
        self.wallet_claims.restore(
            Module::Execution,
            claims,
            self.path.as_ref().and_then(|_| self.readiness().err()),
        );
    }

    pub(super) fn wallet_hold(&self, entry: &LogEntry) -> Result<Option<Hold>, String> {
        let response = entry
            .response
            .as_ref()
            .or_else(|| entry.checkpoint.as_ref().map(|c| &c.response))
            .ok_or("执行记录缺失")?;
        if !matches!(
            response.status,
            OnchainExecutionRunStatus::Executing
                | OnchainExecutionRunStatus::AwaitingChainFinality
                | OnchainExecutionRunStatus::FinalityUnresolved
        ) {
            return Ok(None);
        }
        let prior = self.checkpoints.lock();
        let checkpoint = entry
            .checkpoint
            .as_deref()
            .or_else(|| prior.get(&response.run_id))
            .ok_or("进行中的资金记录缺少原始钱包计划")?;
        if let Some(old) = prior.get(&response.run_id) {
            if old.build.build_id != checkpoint.build.build_id
                || old.build.chain != checkpoint.build.chain
                || old.build.wallet_address != checkpoint.build.wallet_address
            {
                return Err("执行中的原始钱包或计划不可替换".into());
            }
        }
        Hold::wallet(
            &checkpoint.build.chain,
            &checkpoint.build.wallet_address,
            None,
        )
        .map(Some)
    }
}
