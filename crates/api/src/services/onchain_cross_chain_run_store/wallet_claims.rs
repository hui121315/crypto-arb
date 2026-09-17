use super::*;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner, WalletClaims, WalletScope};
use std::{collections::BTreeSet, sync::Arc};

impl OnchainCrossChainRunStore {
    pub(crate) fn with_wallet_claims(mut self, shared: Arc<WalletClaims>) -> Self {
        self.wallet_claims = shared;
        self.restore_wallet_claims();
        self
    }

    pub(super) fn restore_wallet_claims(&self) {
        let claims = (|| {
            let mut claims = Vec::new();
            for row in &self.runs {
                if let Some(hold) = run_hold(&row)? {
                    claims.push((Owner::new(Module::CrossChain, &row.run_id), hold));
                }
            }
            for row in &self.recovery_plans {
                if let Some(hold) = plan_hold(&row.plan)? {
                    claims.push((Owner::new(Module::Recovery, &row.plan.plan_id), hold));
                }
            }
            Ok(claims)
        })();
        self.wallet_claims.restore(
            Module::CrossChain,
            claims,
            self.path.as_ref().and_then(|_| self.readiness().err()),
        );
    }
}

pub(super) fn change(entry: &LogEntry) -> Result<Option<(Owner, Option<Hold>, i64)>, String> {
    if let Some(run) = &entry.run {
        return Ok(Some((
            Owner::new(Module::CrossChain, &run.run_id),
            run_hold(run)?,
            run.updated_at_ms,
        )));
    }
    if let Some(row) = &entry.recovery_plan {
        return Ok(Some((
            Owner::new(Module::Recovery, &row.plan.plan_id),
            plan_hold(&row.plan)?,
            row.plan.updated_at_ms,
        )));
    }
    Ok(None)
}

fn plan_hold(plan: &shared_types::OnchainCrossChainRecoveryPlan) -> Result<Option<Hold>, String> {
    if plan.status != shared_types::OnchainCrossChainRecoveryPlanStatus::Reserved {
        return Ok(None);
    }
    let expiry = plan
        .authorization
        .as_ref()
        .map(|a| a.valid_until_ms)
        .ok_or("处置预留缺少到期证据")?;
    Hold::wallet(
        &plan.preview.input.chain,
        &plan.preview.input.wallet,
        Some(expiry),
    )
    .map(Some)
}

fn run_hold(run: &OnchainCrossChainRun) -> Result<Option<Hold>, String> {
    let mut candidates = Vec::new();
    for swap in run
        .build
        .swap_executions
        .iter()
        .chain(run.legs.iter().filter_map(|l| l.swap_execution.as_ref()))
    {
        candidates.push((swap.chain.as_str(), swap.wallet_address.as_str()));
    }
    for bridge in run
        .build
        .bridge_executions
        .iter()
        .chain(run.legs.iter().filter_map(|l| l.bridge_execution.as_ref()))
    {
        let leg = run
            .build
            .legs
            .iter()
            .find(|l| l.position == bridge.position)
            .ok_or("跨链占用记录缺少原始链身份")?;
        candidates.push((leg.from_chain.as_str(), bridge.from_address.as_str()));
    }
    let wallets = candidates
        .into_iter()
        .filter(|(chain, wallet)| recovery_plans::normal_run_uses_wallet(run, chain, wallet))
        .map(|(chain, wallet)| WalletScope::new(chain, wallet))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok((!wallets.is_empty()).then_some(Hold {
        wallets,
        expires_at_ms: None,
    }))
}
