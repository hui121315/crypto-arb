use super::*;
use rust_decimal::Decimal;
use shared_types::{
    OnchainCrossChainRecoveryAuthorizeRequest as Authorize, OnchainCrossChainRecoveryPlan as Plan,
    OnchainCrossChainRecoveryPlanStatus as Status, OnchainCrossChainRecoveryPreview as Preview,
    OnchainUnsignedTransaction, ONCHAIN_RECOVERY_RESERVATION_PHRASE,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredPlan {
    pub plan: Plan,
    transaction: OnchainUnsignedTransaction,
}

fn plan_id(preview: &Preview, transaction: &OnchainUnsignedTransaction) -> Result<String, String> {
    let mut preview = preview.clone();
    preview.plan_id = None;
    let bytes = serde_json::to_vec(&(preview, transaction)).map_err(|e| e.to_string())?;
    let hash = common::signing::hmac_sha256_hex(b"crossline-recovery-plan-v1", &bytes);
    Ok(format!("recovery-plan-{}", &hash[..24]))
}

fn source_matches(preview: &Preview, run: &OnchainCrossChainRun) -> Result<(), String> {
    if run.run_id != preview.source_run_id
        || run.updated_at_ms != preview.source_run_updated_at_ms
        || !matches!(
            run.status,
            OnchainCrossChainRunStatus::Paused
                | OnchainCrossChainRunStatus::Failed
                | OnchainCrossChainRunStatus::Compensating
        )
    {
        return Err("原交易记录已变化或未停止，必须重新预检".into());
    }
    let disposition = run
        .accounting
        .as_ref()
        .and_then(|a| a.disposition.as_ref())
        .filter(|d| d.blockers.is_empty())
        .ok_or("原交易收支未核齐")?;
    let remaining = disposition
        .remaining_assets
        .get(preview.asset_index)
        .ok_or("剩余资产已变化")?;
    if !matches!(
        remaining.action,
        shared_types::OnchainCrossChainDispositionAction::QuoteSwap
            | shared_types::OnchainCrossChainDispositionAction::QuoteBridge
    ) {
        return Err("此资产无需交易或需要先核对钱包，不能生成资金处置授权".into());
    }
    let mut expected = remaining.change.clone();
    expected.amount_exact = preview.input.amount_exact.clone();
    let amount = accounting::raw_amount(&preview.input_amount_raw, preview.input.asset.decimals)?;
    let maximum =
        Decimal::from_str_exact(&remaining.change.amount_exact).map_err(|_| "本次剩余金额无效")?;
    if expected != preview.input
        || disposition.original_capital.as_ref() != Some(&preview.target)
        || amount <= Decimal::ZERO
        || amount > maximum
        || Decimal::from_str_exact(&preview.input.amount_exact).ok() != Some(amount)
    {
        return Err("处置金额、合约、链或钱包与本次剩余不符".into());
    }
    Ok(())
}

fn validate(stored: &StoredPlan) -> Result<(), String> {
    let plan = &stored.plan;
    let p = &plan.preview;
    let valid_until = p.valid_until_ms.ok_or("处置报价有效期缺失")?;
    let input = positive_raw(&p.input_amount_raw, "处置输入")?;
    let expected = positive_raw(
        p.expected_output_amount_raw
            .as_deref()
            .ok_or("预计到账缺失")?,
        "预计到账",
    )?;
    let minimum = positive_raw(
        p.minimum_output_amount_raw
            .as_deref()
            .ok_or("最低到账缺失")?,
        "最低到账",
    )?;
    if let OnchainUnsignedTransaction::EvmCall { chain_id, from, .. } = &stored.transaction {
        if crate::services::onchain_comparison::lifi::chain_id(&p.input.chain) != Some(*chain_id)
            || accounting::address(&p.input.chain, from)
                != accounting::address(&p.input.chain, &p.input.wallet)
        {
            return Err("处置交易的来源链或签名钱包不一致".into());
        }
    }
    if !p.quote_ready
        || p.submit_ready
        || !p.requires_live_authorization
        || !p.blockers.is_empty()
        || p.route_id.as_deref().is_none_or(str::is_empty)
        || p.provider != "lifi"
        || p.plan_id.as_deref() != Some(&plan.plan_id)
        || plan_id(p, &stored.transaction)? != plan.plan_id
        || p.balance_amount_raw
            .as_deref()
            .and_then(|v| v.parse::<u128>().ok())
            .is_none_or(|balance| balance < input)
        || minimum > expected
        || plan.created_at_ms <= 0
        || plan.updated_at_ms < plan.created_at_ms
        || valid_until <= plan.created_at_ms
        || p.quote_observed_at_ms
            .is_none_or(|t| t <= 0 || t > plan.created_at_ms)
        || p.balance_checked_at_ms.is_none_or(|t| {
            t <= 0 || t > plan.created_at_ms || valid_until > t.saturating_add(15_000)
        })
        || [p.fee_usd, p.gas_usd]
            .into_iter()
            .any(|v| v.is_none_or(|v| !v.is_finite() || v < 0.0))
    {
        return Err("处置计划金额、费用、余额、有效期或合同指纹不完整".into());
    }
    match (&plan.authorization, &plan.idempotency_key) {
        (None, None)
            if matches!(
                plan.status,
                Status::AwaitingAuthorization | Status::Cancelled
            ) => {}
        (Some(auth), Some(key))
            if matches!(plan.status, Status::Reserved | Status::Cancelled)
                && !key.trim().is_empty()
                && !auth.actor.trim().is_empty()
                && auth.authorized_at_ms >= plan.created_at_ms
                && auth.authorized_at_ms < valid_until
                && auth.valid_until_ms == valid_until
                && auth.confirmation_version == ONCHAIN_RECOVERY_RESERVATION_PHRASE => {}
        _ => return Err("处置计划授权状态不一致".into()),
    }
    Ok(())
}

fn same_wallet(chain: &str, wallet: &str, other_chain: &str, other_wallet: &str) -> bool {
    chain == other_chain
        && accounting::address(chain, wallet) == accounting::address(chain, other_wallet)
}

pub(super) fn normal_run_uses_wallet(run: &OnchainCrossChainRun, chain: &str, wallet: &str) -> bool {
    let read_only_recheck = run.legs.iter().any(|leg| {
        run.active_position == Some(leg.position) && leg.recovery_started_at_ms.is_some()
    });
    let running = run.status == OnchainCrossChainRunStatus::Running
        || !read_only_recheck
            && matches!(
                run.status,
                OnchainCrossChainRunStatus::AwaitingSourceFinality
                    | OnchainCrossChainRunStatus::AwaitingDestinationEvidence
            );
    let complete = |receipt: Option<&shared_types::OnchainWalletReceipt>| {
        receipt.is_some_and(|r| {
            r.status == shared_types::OnchainChainSettlementStatus::Complete && r.problem.is_none()
        })
    };
    // Pausing does not release a wallet whose submitted transaction is still unresolved.
    let unresolved = run.legs.iter().any(|leg| {
        (leg.attempts > 0 || leg.source_transaction_id.is_some())
            && (!complete(leg.source_receipt.as_ref())
                || leg.bridge_execution.is_some()
                    && !complete(leg.destination_receipt.as_ref())
                    && !complete(
                        leg.bridge_recovery
                            .as_ref()
                            .and_then(|r| r.receipt.as_ref()),
                    ))
    });
    if !running && !unresolved {
        return false;
    }
    run.build
        .swap_executions
        .iter()
        .chain(
            run.legs
                .iter()
                .filter_map(|leg| leg.swap_execution.as_ref()),
        )
        .any(|s| same_wallet(chain, wallet, &s.chain, &s.wallet_address))
        || run
            .build
            .bridge_executions
            .iter()
            .chain(
                run.legs
                    .iter()
                    .filter_map(|leg| leg.bridge_execution.as_ref()),
            )
            .any(|b| {
                run.build
                    .legs
                    .iter()
                    .find(|leg| leg.position == b.position)
                    .is_some_and(|leg| same_wallet(chain, wallet, &leg.from_chain, &b.from_address))
            })
}

fn projected(stored: &StoredPlan, now_ms: i64) -> Plan {
    let mut plan = stored.plan.clone();
    if plan.status != Status::Cancelled && plan.preview.valid_until_ms.is_none_or(|t| now_ms >= t) {
        plan.status = Status::Expired;
    }
    plan
}

impl OnchainCrossChainRunStore {
    pub(crate) fn save_recovery_plan(
        &self,
        mut preview: Preview,
        transaction: OnchainUnsignedTransaction,
        now_ms: i64,
    ) -> Result<Plan, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        let run = self
            .runs
            .get(&preview.source_run_id)
            .map(|run| run.value().clone())
            .ok_or("原运行不存在")?;
        source_matches(&preview, &run)?;
        let id = plan_id(&preview, &transaction)?;
        preview.plan_id = Some(id.clone());
        if let Some(old) = self.recovery_plans.get(&id) {
            return Ok(projected(&old, now_ms));
        }
        let stored = StoredPlan {
            transaction,
            plan: Plan {
                plan_id: id.clone(),
                preview,
                status: Status::AwaitingAuthorization,
                authorization: None,
                idempotency_key: None,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            },
        };
        validate(&stored)?;
        self.append_unlocked(&LogEntry {
            schema_version: SCHEMA_VERSION,
            build: None,
            run: None,
            recovery_plan: Some(stored.clone()),
        })?;
        self.recovery_plans.insert(id, stored.clone());
        self.prune_recovery_plans(now_ms);
        Ok(stored.plan)
    }

    pub(crate) fn reserve_recovery_plan(
        &self,
        request: &Authorize,
        actor: &str,
        now_ms: i64,
    ) -> Result<Plan, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        if request.confirmation != ONCHAIN_RECOVERY_RESERVATION_PHRASE
            || request.idempotency_key.trim().is_empty()
            || request.idempotency_key.len() > 200
        {
            return Err("必须明确确认本计划并提供有效幂等编号".into());
        }
        let mut stored = self
            .recovery_plans
            .get(&request.plan_id)
            .map(|p| p.clone())
            .ok_or("处置计划不存在或已归档，请重新预检")?;
        let run = self
            .runs
            .get(&stored.plan.preview.source_run_id)
            .map(|run| run.value().clone())
            .ok_or("原运行不存在")?;
        if run.authorization.actor != actor {
            return Err("仅原运行账户可以预留或取消此计划".into());
        }
        if self
            .recovery_keys
            .get(&request.idempotency_key)
            .is_some_and(|id| *id != request.plan_id)
        {
            return Err("幂等编号已经用于其他处置计划".into());
        }
        if stored.plan.idempotency_key.as_deref() == Some(&request.idempotency_key) {
            return Ok(projected(&stored, now_ms));
        }
        if stored.plan.status != Status::AwaitingAuthorization
            || stored.plan.idempotency_key.is_some()
            || stored
                .plan
                .preview
                .valid_until_ms
                .is_none_or(|deadline| now_ms >= deadline)
        {
            return Err("处置计划已预留、取消或过期，不能用新编号重复授权".into());
        }
        source_matches(&stored.plan.preview, &run)?;
        let input = &stored.plan.preview.input;
        self.ensure_recovery_wallet_available(
            &input.chain,
            &input.wallet,
            Some(&request.plan_id),
            now_ms,
        )?;
        if self.runs.iter().any(|other| {
            other.run_id != run.run_id
                && normal_run_uses_wallet(&other, &input.chain, &input.wallet)
        }) {
            return Err("该链上钱包仍有跨链步骤执行中，请先核验原交易".into());
        }
        stored.plan.status = Status::Reserved;
        stored.plan.authorization = Some(OnchainCrossChainAuthorizationEvidence {
            actor: actor.into(),
            authorized_at_ms: now_ms,
            valid_until_ms: stored.plan.preview.valid_until_ms.unwrap(),
            confirmation_version: ONCHAIN_RECOVERY_RESERVATION_PHRASE.into(),
        });
        stored.plan.idempotency_key = Some(request.idempotency_key.clone());
        stored.plan.updated_at_ms = now_ms;
        validate(&stored)?;
        self.append_unlocked(&LogEntry {
            schema_version: SCHEMA_VERSION,
            build: None,
            run: None,
            recovery_plan: Some(stored.clone()),
        })?;
        self.recovery_keys
            .insert(request.idempotency_key.clone(), request.plan_id.clone());
        self.recovery_plans
            .insert(request.plan_id.clone(), stored.clone());
        Ok(stored.plan)
    }

    pub(crate) fn cancel_recovery_plan(
        &self,
        plan_id: &str,
        actor: &str,
        now_ms: i64,
    ) -> Result<Plan, String> {
        let _guard = self.ledger_lock.lock();
        self.readiness()?;
        let mut stored = self
            .recovery_plans
            .get(plan_id)
            .map(|p| p.clone())
            .ok_or("处置计划不存在或已归档")?;
        let run = self
            .runs
            .get(&stored.plan.preview.source_run_id)
            .map(|run| run.value().clone())
            .ok_or("原运行不存在")?;
        if run.authorization.actor != actor {
            return Err("仅原运行账户可以预留或取消此计划".into());
        }
        if matches!(
            projected(&stored, now_ms).status,
            Status::Cancelled | Status::Expired
        ) {
            return Ok(projected(&stored, now_ms));
        }
        stored.plan.status = Status::Cancelled;
        stored.plan.updated_at_ms = now_ms;
        validate(&stored)?;
        self.append_unlocked(&LogEntry {
            schema_version: SCHEMA_VERSION,
            build: None,
            run: None,
            recovery_plan: Some(stored.clone()),
        })?;
        self.recovery_plans.insert(plan_id.into(), stored.clone());
        Ok(stored.plan)
    }

    pub(super) fn ensure_recovery_wallet_available(
        &self,
        chain: &str,
        wallet: &str,
        except: Option<&str>,
        now_ms: i64,
    ) -> Result<(), String> {
        if let Some(stored) = self.recovery_plans.iter().find(|stored| {
            Some(stored.plan.plan_id.as_str()) != except
                && stored.plan.reservation_active(now_ms)
                && same_wallet(
                    chain,
                    wallet,
                    &stored.plan.preview.input.chain,
                    &stored.plan.preview.input.wallet,
                )
        }) {
            return Err(format!(
                "该链上钱包已被处置计划 {} 预留；请先取消或等待报价过期",
                stored.plan.plan_id
            ));
        }
        Ok(())
    }

    pub(super) fn recovery_plan_rows(&self, now_ms: i64) -> Vec<Plan> {
        let mut plans = self
            .recovery_plans
            .iter()
            .map(|p| projected(&p, now_ms))
            .collect::<Vec<_>>();
        plans.sort_by_key(|p| {
            (
                if p.reservation_active(now_ms) { 0 } else { 1 },
                std::cmp::Reverse(p.updated_at_ms),
            )
        });
        plans
    }

    pub(super) fn prune_recovery_plans(&self, now_ms: i64) {
        while self.recovery_plans.len() > 128 {
            let candidate = self
                .recovery_plans
                .iter()
                .filter(|p| !p.plan.reservation_active(now_ms))
                .min_by_key(|p| p.plan.updated_at_ms)
                .map(|p| p.plan.plan_id.clone());
            let Some(id) = candidate else {
                break;
            };
            self.recovery_plans.remove(&id);
        }
    }
}

pub(super) fn validate_replay(
    stored: &StoredPlan,
    plans: &BTreeMap<String, StoredPlan>,
    runs: &BTreeMap<String, OnchainCrossChainRun>,
    keys: &BTreeMap<String, String>,
) -> Result<(), String> {
    validate(stored)?;
    let plan = &stored.plan;
    let old = plans.get(&plan.plan_id);
    let mut run = runs
        .get(&plan.preview.source_run_id)
        .cloned()
        .ok_or("缺少原运行")?;
    accounting::project(&mut run);
    if plan
        .authorization
        .as_ref()
        .is_some_and(|auth| auth.actor != run.authorization.actor)
        || plan
            .idempotency_key
            .as_ref()
            .is_some_and(|key| keys.get(key).is_some_and(|id| id != &plan.plan_id))
    {
        return Err("授权账户或幂等编号冲突".into());
    }
    if plan.status == Status::Reserved {
        let when = plan.authorization.as_ref().unwrap().authorized_at_ms;
        let input = &plan.preview.input;
        if plans.values().any(|other| {
            other.plan.plan_id != plan.plan_id
                && other.plan.reservation_active(when)
                && same_wallet(
                    &input.chain,
                    &input.wallet,
                    &other.plan.preview.input.chain,
                    &other.plan.preview.input.wallet,
                )
        }) || runs.values().any(|other| {
            other.run_id != run.run_id && normal_run_uses_wallet(other, &input.chain, &input.wallet)
        }) {
            return Err("日志中出现同一跨链钱包的冲突预留".into());
        }
    }
    match old {
        None if plan.status == Status::AwaitingAuthorization => source_matches(&plan.preview, &run),
        Some(old) if old == stored => Ok(()),
        Some(old)
            if old.plan.preview == plan.preview
                && old.transaction == stored.transaction
                && old.plan.created_at_ms == plan.created_at_ms
                && plan.updated_at_ms >= old.plan.updated_at_ms
                && (old.plan.status == Status::AwaitingAuthorization
                    && plan.status == Status::Reserved
                    || matches!(
                        old.plan.status,
                        Status::AwaitingAuthorization | Status::Reserved
                    ) && plan.status == Status::Cancelled)
                && (old.plan.authorization.is_none()
                    || old.plan.authorization == plan.authorization)
                && (old.plan.idempotency_key.is_none()
                    || old.plan.idempotency_key == plan.idempotency_key) =>
        {
            if plan.status == Status::Reserved {
                source_matches(&plan.preview, &run)
            } else {
                Ok(())
            }
        }
        _ => Err("处置合同被改写、取消后重新授权或状态倒退".into()),
    }
}

pub(super) fn validate_run_reservations(
    run: &OnchainCrossChainRun,
    plans: &BTreeMap<String, StoredPlan>,
) -> Result<(), String> {
    if plans.values().any(|stored| {
        stored.plan.preview.source_run_id != run.run_id
            && stored.plan.reservation_active(run.updated_at_ms)
            && normal_run_uses_wallet(
                run,
                &stored.plan.preview.input.chain,
                &stored.plan.preview.input.wallet,
            )
    }) {
        return Err("跨链提交与处置钱包预留冲突".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
