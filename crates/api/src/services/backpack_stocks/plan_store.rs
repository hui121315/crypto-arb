use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner, WalletClaims};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};

mod settlement;
pub(super) mod recovery;

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u8,
    plan: StockExecutionPlan,
}

#[derive(Default)]
struct Inner {
    rows: BTreeMap<String, StockExecutionPlan>,
    problem: Option<String>,
    lock: Option<File>,
}

pub(super) struct PlanStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
    wallets: Arc<WalletClaims>,
}

impl PlanStore {
    pub(super) fn load(path: Option<PathBuf>, wallets: Arc<WalletClaims>) -> Self {
        let mut inner = Inner::default();
        if let Some(path) = path.as_deref() {
            let result = (|| -> Result<(), String> {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|_| "股票计划目录不可用")?;
                }
                let file = options(false)
                    .open(path.with_extension("lock"))
                    .map_err(|_| "股票计划锁不可用")?;
                file.try_lock_exclusive()
                    .map_err(|_| "股票计划日志已由另一实例占用")?;
                inner.lock = Some(file);
                read_rows(path, &mut inner.rows)
            })();
            inner.problem = result.err();
            let holds = inner
                .rows
                .values()
                .filter_map(|p| match hold(p) {
                    Ok(Some(h)) => Some(Ok((Owner::new(Module::Stocks, &p.plan_id), h))),
                    Ok(None) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect::<Result<Vec<_>, _>>();
            wallets.restore(Module::Stocks, holds, inner.problem.clone());
        }
        Self {
            path,
            inner: Mutex::new(inner),
            wallets,
        }
    }

    pub(super) fn records(&self) -> Vec<StockExecutionPlan> {
        let mut rows = self.inner.lock().rows.values().cloned().collect::<Vec<_>>();
        let now = common::time::now_ms();
        rows.sort_by_key(|p| {
            (
                !p.holds_funds(now),
                std::cmp::Reverse(p.terms.created_at_ms),
            )
        });
        rows.truncate(48);
        rows
    }

    pub(super) fn problem(&self) -> Option<String> {
        self.inner.lock().problem.clone()
    }

    pub(super) fn get(&self, id: &str) -> Result<StockExecutionPlan, String> {
        self.inner
            .lock()
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or_else(|| "股票计划不存在".into())
    }

    pub(super) fn accepted_rfqs(&self) -> Vec<StockRfq> {
        self.inner
            .lock()
            .rows
            .values()
            .filter_map(|p| p.rfq_acceptance.clone())
            .collect()
    }

    pub(super) fn accepted_rfq(&self, request_id: &str) -> Option<StockRfq> {
        self.inner
            .lock()
            .rows
            .values()
            .filter_map(|p| p.rfq_acceptance.as_ref())
            .find(|r| r.request.request_id == request_id)
            .cloned()
    }

    pub(super) fn begin_rfq(
        &self,
        id: &str,
        fingerprint: &str,
        record: StockRfq,
        now: i64,
    ) -> Result<(StockExecutionPlan, bool), String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let mut plan = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("股票计划不存在")?;
        if plan.terms.account_fingerprint != fingerprint {
            return Err("股票计划属于其他账户凭证".into());
        }
        if plan.rfq_acceptance.is_some() {
            return Ok((plan, false));
        }
        if plan.phase_at(now) != StockPlanPhase::Reserved
            || plan.cex_order.is_some()
            || now < plan.terms.created_at_ms
            || now >= plan.terms.market_valid_until_ms
        {
            return Err("股票计划已提交或报价过期，不能接受 RFQ".into());
        }
        plan.rfq_acceptance = Some(rfq_acceptance::intent(&plan, record, now)?);
        plan.phase = StockPlanPhase::SubmissionUnknown;
        plan.revision = plan.revision.checked_add(1).ok_or("计划版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        validate(&plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok((plan, true))
    }

    pub(super) fn change_rfq(
        &self,
        id: &str,
        apply: impl FnOnce(&mut StockRfq) -> Result<bool, String>,
    ) -> Result<Option<StockRfq>, String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let mut plan = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("股票计划不存在")?;
        let old = plan.rfq_acceptance.clone();
        let record = plan
            .rfq_acceptance
            .as_mut()
            .ok_or("原计划没有 RFQ 接受记录")?;
        if !apply(record)? {
            return Ok(None);
        }
        if !rfq_acceptance::transition(old.as_ref(), Some(record)) {
            return Err("RFQ 接受回执不允许改写身份、回退或丢失成交".into());
        }
        plan.updated_at_ms = plan.updated_at_ms.max(record.updated_at_ms);
        plan.revision = plan.revision.checked_add(1).ok_or("计划版本溢出")?;
        validate(&plan)?;
        self.persist(&mut inner, &plan, common::time::now_ms())?;
        let receipt = plan.rfq_acceptance.clone();
        inner.rows.insert(plan.request.request_id.clone(), plan);
        Ok(receipt)
    }

    pub(super) fn begin_order(
        &self,
        id: &str,
        fingerprint: &str,
        now: i64,
    ) -> Result<(StockExecutionPlan, bool), String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let mut plan = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("股票计划不存在")?;
        if plan.terms.account_fingerprint != fingerprint {
            return Err("股票计划属于其他账户凭证".into());
        }
        if plan.cex_order.is_some() {
            return Ok((plan, false));
        }
        if plan.phase_at(now) != StockPlanPhase::Reserved
            || plan.rfq_acceptance.is_some()
            || now >= plan.terms.market_valid_until_ms
            || now < plan.terms.created_at_ms
            || !matches!(
                plan.terms.cex_instruction,
                Some(StockCexInstruction::OrderBook { .. })
            )
        {
            return Err("股票计划不具备有效的订单簿提交条件".into());
        }
        plan.phase = StockPlanPhase::SubmissionUnknown;
        plan.cex_order = Some(StockCexOrder::intent(now));
        plan.revision = plan.revision.checked_add(1).ok_or("计划版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        validate(&plan)?;
        // Durable intent precedes the only POST; ambiguous writes never trigger a send.
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok((plan, true))
    }

    pub(super) fn begin_pair(
        &self, id: &str, fingerprint: &str, signed: &str, rfq: Option<StockRfq>, now: i64,
    ) -> Result<(StockExecutionPlan, bool), String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let mut plan = inner.rows.values().find(|p| p.plan_id == id).cloned().ok_or("股票计划不存在")?;
        if plan.terms.account_fingerprint != fingerprint { return Err("股票计划属于其他账户凭证".into()); }
        if plan.two_leg_started_at_ms.is_some() { return Ok((plan, false)); }
        if plan.phase_at(now) != StockPlanPhase::Reserved || now < plan.terms.created_at_ms
            || now >= plan.terms.market_valid_until_ms || plan.cex_order.is_some()
            || plan.rfq_acceptance.is_some() || plan.chain_submission.is_some()
            || plan.terms.cex_fee_budget.is_none() {
            return Err("两腿计划已过期、费用未绑定或已有单腿提交，不能再次启动".into());
        }
        plan.chain_submission = Some(chain::intent(&plan.terms.chain_cost, signed, now)?);
        match plan.terms.cex_instruction.as_ref() {
            Some(StockCexInstruction::OrderBook { .. }) if rfq.is_none() => {
                plan.cex_order = Some(StockCexOrder::intent(now));
            }
            Some(StockCexInstruction::AcceptRfq { .. }) => {
                plan.rfq_acceptance = Some(rfq_acceptance::intent(&plan, rfq.ok_or("原 RFQ 缺失")?, now)?);
            }
            _ => return Err("两腿计划缺少对应的交易所原始指令".into()),
        }
        plan.two_leg_started_at_ms = Some(now);
        plan.phase = StockPlanPhase::SubmissionUnknown;
        plan.revision = plan.revision.checked_add(1).ok_or("计划版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        validate(&plan)?;
        // One durable record claims both sends before either network request starts.
        self.persist(&mut inner, &plan, now)?;
        inner.rows.insert(plan.request.request_id.clone(), plan.clone());
        Ok((plan, true))
    }

    pub(super) fn change_order(
        &self,
        id: &str,
        now: i64,
        apply: impl FnOnce(&mut StockCexOrder, &StockCexInstruction) -> Result<bool, String>,
    ) -> Result<StockExecutionPlan, String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let mut plan = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("股票计划不存在")?;
        let old = plan.cex_order.clone();
        let row = plan.cex_order.as_mut().ok_or("原计划没有提交记录")?;
        let instruction = plan
            .terms
            .cex_instruction
            .as_ref()
            .ok_or("原计划没有交易指令")?;
        if !apply(row, instruction)? {
            return Ok(plan);
        }
        if !order_protocol::transition(old.as_ref(), plan.cex_order.as_ref()) {
            return Err("股票订单回执不允许回退或丢失已有成交".into());
        }
        plan.revision = plan.revision.checked_add(1).ok_or("计划版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        validate(&plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }

    pub(super) fn begin_chain(&self, id: &str, fingerprint: &str, signed: &str, now: i64) -> Result<(StockExecutionPlan, bool), String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let mut plan = inner.rows.values().find(|p| p.plan_id == id).cloned().ok_or("股票计划不存在")?;
        if plan.terms.account_fingerprint != fingerprint { return Err("股票计划属于其他账户凭证".into()); }
        if plan.chain_submission.is_some() { return Ok((plan, false)); }
        if plan.phase_at(now) != StockPlanPhase::Reserved || plan.cex_order.is_some() || plan.rfq_acceptance.is_some()
            || now < plan.terms.created_at_ms || now >= plan.terms.market_valid_until_ms {
            return Err("链上原计划已过期或已提交，不能重复发送".into());
        }
        plan.chain_submission = Some(chain::intent(&plan.terms.chain_cost, signed, now)?);
        plan.phase = StockPlanPhase::SubmissionUnknown;
        plan.revision = plan.revision.checked_add(1).ok_or("计划版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        validate(&plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner.rows.insert(plan.request.request_id.clone(), plan.clone());
        Ok((plan, true))
    }

    pub(super) fn change_chain(&self, id: &str, now: i64, apply: impl FnOnce(&mut StockChainSubmission) -> Result<(), String>) -> Result<StockExecutionPlan, String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let mut plan = inner.rows.values().find(|p| p.plan_id == id).cloned().ok_or("股票计划不存在")?;
        let old = plan.chain_submission.clone();
        apply(plan.chain_submission.as_mut().ok_or("原计划尚未提交链上交易")?)?;
        if !chain::transition(old.as_ref(), plan.chain_submission.as_ref()) { return Err("不能改写原交易身份或已核实回执".into()); }
        if old == plan.chain_submission { return Ok(plan); }
        plan.revision = plan.revision.checked_add(1).ok_or("计划版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        validate(&plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner.rows.insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }

    pub(super) fn previous(
        &self,
        request: &StockPlanRequest,
        fingerprint: &str,
    ) -> Result<Option<StockExecutionPlan>, String> {
        let inner = self.inner.lock();
        let Some(previous) = inner.rows.get(&request.request_id) else {
            return Ok(None);
        };
        if previous.request != *request || previous.terms.account_fingerprint != fingerprint {
            return Err("相同计划请求标识对应不同参数或凭证；原计划未改动".into());
        }
        Ok(Some(previous.clone()))
    }

    pub(super) fn previous_build(&self, request: &StockPlanBuildRequest, fingerprint: &str) -> Result<Option<StockExecutionPlan>, String> {
        let inner = self.inner.lock();
        self.healthy(&inner)?;
        let Some(previous) = inner.rows.get(&request.request_id) else { return Ok(None); };
        if previous.request.build.as_ref() != Some(request) || previous.terms.account_fingerprint != fingerprint {
            return Err("相同构建标识对应不同参数或凭证；原计划未改动".into());
        }
        Ok(Some(previous.clone()))
    }

    pub(super) fn reserve(
        &self,
        plan: StockExecutionPlan,
        now: i64,
    ) -> Result<StockExecutionPlan, String> {
        validate(&plan)?;
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        if let Some(old) = inner.rows.get(&plan.request.request_id) {
            if old.request != plan.request
                || old.terms.account_fingerprint != plan.terms.account_fingerprint
            {
                return Err("计划请求标识已用于其他参数".into());
            }
            return Ok(old.clone());
        }
        if plan.phase != StockPlanPhase::Reserved
            || plan.terms.cex_instruction.is_none()
            || now >= plan.terms.market_valid_until_ms
            || now < plan.terms.created_at_ms
        {
            return Err("计划市场证据已过期，未预留资金".into());
        }
        if let Some(StockCexInstruction::OrderBook { client_id, .. }) = &plan.terms.cex_instruction
        {
            if inner.rows.values().any(|p|matches!(&p.terms.cex_instruction,Some(StockCexInstruction::OrderBook {client_id:old,..}) if old==client_id)) {
                return Err("股票订单 clientId 已用于历史计划，请创建新计划".into());
            }
        }
        // The product has one configured Backpack stock account. Do not use an
        // API-key hash as an account ID: key rotation must not bypass reservations.
        if let Some(old) = inner.rows.values().find(|p| p.holds_funds(now)) {
            return Err(format!(
                "Backpack 股票交易通道已由 {} 预留；请先取消未提交计划或核对原订单",
                old.plan_id
            ));
        }
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }

    pub(super) fn cancel(&self, id: &str, now: i64) -> Result<StockExecutionPlan, String> {
        let mut inner = self.inner.lock();
        self.healthy(&inner)?;
        let mut plan = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("股票计划不存在")?;
        match plan.phase_at(now) {
            StockPlanPhase::SubmissionUnknown => {
                return Err("计划已有未明提交；只能核对原交易，不能取消预留或重发".into())
            }
            StockPlanPhase::Cancelled | StockPlanPhase::Expired | StockPlanPhase::Settled => return Ok(plan),
            StockPlanPhase::Reserved => {}
        }
        plan.phase = StockPlanPhase::Cancelled;
        plan.revision = plan.revision.checked_add(1).ok_or("计划版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }

    fn healthy(&self, inner: &Inner) -> Result<(), String> {
        if let Some(p) = &inner.problem {
            return Err(p.clone());
        }
        if self.path.is_none() || inner.lock.is_none() {
            return Err("股票计划持久化未配置，不能预留资金".into());
        }
        Ok(())
    }

    fn persist(
        &self,
        inner: &mut Inner,
        plan: &StockExecutionPlan,
        now: i64,
    ) -> Result<(), String> {
        let path = self.path.as_deref().ok_or("股票计划持久化未配置")?;
        let mut write_problem = None;
        let result = self.wallets.commit(
            Owner::new(Module::Stocks, &plan.plan_id),
            hold(plan)?,
            now,
            || {
                let write = (|| -> std::io::Result<()> {
                    let existed = path.exists();
                    let mut bytes = serde_json::to_vec(&Entry {
                        version: 1,
                        plan: plan.clone(),
                    })?;
                    bytes.push(b'\n');
                    if bytes.len() > 128 * 1024 {
                        return Err(std::io::Error::other("stock plan exceeds row budget"));
                    }
                    let mut file = options(true).open(path)?;
                    file.write_all(&bytes)?;
                    file.sync_data()?;
                    if !existed {
                        if let Some(parent) = path.parent() {
                            File::open(parent)?.sync_all()?;
                        }
                    }
                    Ok(())
                })();
                write.map_err(|_| {
                    let problem = "股票计划写入结果未核清；保留日志，禁止新预留".to_owned();
                    write_problem = Some(problem.clone());
                    problem
                })
            },
        );
        // Conflicts do not poison the journal; ambiguous writes do.
        if let Some(problem) = write_problem {
            inner.problem = Some(problem);
        }
        result
    }
}

pub(super) fn options(append: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .create(true)
        .write(true)
        .read(!append)
        .append(append)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

pub(super) fn plan_id(
    request: &StockPlanRequest,
    terms: &StockPlanTerms,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(request, terms)).map_err(|_| "股票计划无法编码")?;
    let hash = common::signing::hmac_sha256_hex(b"stock-plan-v1", &bytes);
    Ok(format!("stock-plan-{}", &hash[..32]))
}

fn hold(plan: &StockExecutionPlan) -> Result<Option<Hold>, String> {
    match plan.phase {
        StockPlanPhase::Reserved => Hold::wallet(
            "solana",
            &plan.request.wallet_address,
            Some(plan.terms.reserved_until_ms),
        )
        .and_then(|h|h.with_account("backpack_stocks","configured-account"))
        .map(Some),
        StockPlanPhase::SubmissionUnknown => {
            Hold::wallet("solana", &plan.request.wallet_address, None)
                .and_then(|h|h.with_account("backpack_stocks","configured-account")).map(Some)
        }
        _ => Ok(None),
    }
}

fn validate(plan: &StockExecutionPlan) -> Result<(), String> {
    use rust_decimal::Decimal;
    let id = &plan.request.request_id;
    let t = &plan.terms;
    let decimal = |s: &str| Decimal::from_str_exact(s).ok();
    if plan.request.build.as_ref().is_some_and(|b| b.request_id != *id
        || b.asset != plan.request.asset || b.direction != plan.request.direction
        || b.wallet_address != plan.request.wallet_address || b.input_raw != t.chain_cost.quote.input_raw) {
        return Err("原构建参数与持久化计划不一致".into());
    }
    if !(16..=128).contains(&id.len())
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        || plan.plan_id != plan_id(&plan.request, t)?
        || plan.revision == 0
        || plan.phase == StockPlanPhase::Expired
        || t.created_at_ms <= 0
        || plan.updated_at_ms < t.created_at_ms
        || plan.request.preflight_at_ms <= 0
        || plan.request.preflight_at_ms > t.created_at_ms
        || t.market_valid_until_ms <= t.created_at_ms
        || t.reserved_until_ms <= t.created_at_ms
        || t.market_valid_until_ms > t.reserved_until_ms
        || t.reserved_until_ms > t.created_at_ms.saturating_add(60_000)
        || t.security.asset != plan.request.asset
        || t.chain_cost.asset != plan.request.asset
        || t.chain_cost.wallet_address != plan.request.wallet_address
        || t.chain_cost.direction != plan.request.direction
        || !t.chain_cost.simulation_passed
        || !t.chain_cost.problems.is_empty()
        || t.chain_cost.native_usdc_budget(t.created_at_ms).is_none()
        || t.chain_cost.transaction_fingerprint.is_empty()
        || t.chain_cost
            .wallet_required_lamports
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .is_none()
        || [
            &t.cex_shares,
            &t.cex_notional_usdc,
            &t.after_known_costs_usdc,
        ]
        .iter()
        .any(|s| decimal(s).is_none_or(|n| n <= Decimal::ZERO))
        || t.account_fingerprint.is_empty()
        || !(3..=4).contains(&t.allocations.len())
        || t.allocations.iter().any(|a| {
            a.asset.is_empty()
                || a.location.is_empty()
                || decimal(&a.quantity)
                    .zip(decimal(&a.available_at_reservation))
                    .is_none_or(|(q, a)| q < Decimal::ZERO || a < q)
        })
    {
        return Err("股票计划身份、金额或有效期不完整".into());
    }
    if t.chain_cost
        .native_valuation
        .as_ref()
        .is_some_and(|v| v.replenishment.is_some())
    {
        let budget = t
            .chain_cost
            .complete_native_usdc_budget(t.created_at_ms)
            .as_deref()
            .and_then(decimal)
            .ok_or("股票计划补仓成本未闭合")?;
        let sol = t
            .chain_cost
            .total_native_required_lamports(t.created_at_ms)
            .map(Decimal::from)
            .and_then(|n| n.checked_div(Decimal::from(1_000_000_000)))
            .ok_or("股票计划补仓周转余额未知")?;
        let chain_buy = plan.request.direction == StockChainDirection::Buy;
        let expected_usdc = if chain_buy {
            decimal(&t.chain_cost.quote.input_raw)
                .and_then(|n| n.checked_div(Decimal::from(1_000_000)))
                .and_then(|n| n.checked_add(budget))
        } else {
            Some(budget)
        };
        let usdc = t
            .allocations
            .get(if chain_buy { 1 } else { 3 })
            .ok_or("股票计划缺少补仓 USDC 备款")?;
        let gas = &t.allocations[2];
        if t.allocations.len() != if chain_buy { 3 } else { 4 }
            || usdc.location != "Solana"
            || usdc.asset
                != if chain_buy {
                    "USDC"
                } else {
                    "USDC / SOL 补仓"
                }
            || decimal(&usdc.quantity) != expected_usdc
            || gas.location != "Solana"
            || gas.asset != "SOL / 保守周转余额"
            || decimal(&gas.quantity) != Some(sol)
        {
            return Err("股票计划补仓成本与资金预留不一致".into());
        }
    } else if t.allocations.len() != 3 {
        return Err("股票计划包含未核实的额外备款".into());
    }
    super::super::onchain_comparison::stock_inventory::validate_owner(
        &plan.request.wallet_address,
    )?;
    if let Some(compiled) = &t.cex_instruction {
        if compiled != &order_compile::compile(&plan.request, t)? {
            return Err("股票计划的交易指令与原始金额或身份不一致".into());
        }
    }
    if let Some(fee) = &t.cex_fee_budget {
        let side = if plan.request.direction == StockChainDirection::Buy { StockRfqSide::Ask } else { StockRfqSide::Bid };
        let valid_basis = match &fee.basis {
            StockCexFeeBasis::OrderBookQuote { observed_at_ms, .. } => t.route.kind == StockRouteKind::OrderBook
                && *observed_at_ms <= t.created_at_ms && t.created_at_ms - *observed_at_ms <= 300_000,
            StockCexFeeBasis::RfqIncluded { quote_id } => t.route.kind == StockRouteKind::Rfq
                && t.rfq.as_ref().is_some_and(|r| r.candidate.quote_id == *quote_id),
        };
        let cex = &t.allocations[0];
        if !valid_basis || StockCexFeeBudget::calculate(&t.cex_notional_usdc, side, fee.basis.clone()).as_ref() != Some(fee)
            || cex.location != "Backpack" || cex.asset != if side == StockRfqSide::Bid {"USDC"} else {&plan.request.asset}
            || fee.required(side, &t.cex_shares).as_deref() != Some(cex.quantity.as_str()) {
            return Err("股票计划费用或交易所资金预留与原始金额不一致".into());
        }
    }
    if t.preflight_evidence.is_some() {
        plan.validate_preflight_evidence()?;
    }
    if let Some(at) = plan.two_leg_started_at_ms {
        let cex_at = plan.cex_order.as_ref().map(|o| o.submitted_at_ms).or_else(|| plan.rfq_acceptance.as_ref()?.acceptance.as_ref().map(|a| a.submitted_at_ms));
        if at < t.created_at_ms || at >= t.market_valid_until_ms || cex_at != Some(at)
            || plan.chain_submission.as_ref().map(|c| c.submitted_at_ms) != Some(at) || t.cex_fee_budget.is_none() {
            return Err("两腿提交意图或费用凭据不完整".into());
        }
    } else if plan.chain_submission.is_some() && (plan.cex_order.is_some() || plan.rfq_acceptance.is_some()) {
        return Err("两腿提交缺少统一的持久化意图".into());
    }
    if let Some(order) = &plan.cex_order {
        if plan.rfq_acceptance.is_some() {
            return Err("同一计划不能同时提交订单簿与 RFQ".into());
        }
        if !matches!(plan.phase, StockPlanPhase::SubmissionUnknown | StockPlanPhase::Settled)
            || order.submitted_at_ms < t.created_at_ms
            || order.submitted_at_ms >= t.market_valid_until_ms
            || order.updated_at_ms > plan.updated_at_ms
        {
            return Err("股票订单未绑定有效计划或资金占用".into());
        }
        order_protocol::validate(order, t.cex_instruction.as_ref().ok_or("订单缺少原始指令")?)?;
    }
    if t.chain_cost.transaction.is_some() { chain::validate_artifact(&t.chain_cost)?; }
    if let Some(row) = &plan.chain_submission {
        if !matches!(plan.phase, StockPlanPhase::SubmissionUnknown | StockPlanPhase::Settled) || row.submitted_at_ms < t.created_at_ms
            || row.submitted_at_ms >= t.market_valid_until_ms || row.submitted_at_ms > plan.updated_at_ms {
            return Err("链上提交未绑定有效计划或资金占用".into());
        }
        chain::validate_record(&t.chain_cost, row)?;
    }
    rfq_acceptance::validate(plan)?;
    settlement::validate_tail(plan)?;
    Ok(())
}

fn read_rows(path: &Path, rows: &mut BTreeMap<String, StockExecutionPlan>) -> Result<(), String> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("股票计划日志不可读".into()),
    };
    if file.metadata().map_err(|_| "股票计划元数据不可读")?.len() > 64 * 1024 * 1024 {
        return Err("股票计划日志超过读取上限，请保留原文件核查".into());
    }
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = std::io::Read::by_ref(&mut reader)
            .take(128 * 1024 + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| "股票计划日志读取失败")?;
        if n == 0 {
            return Ok(());
        }
        if n > 128 * 1024 || line.last() != Some(&b'\n') {
            return Err("股票计划日志存在不完整记录，不能释放或新增预留".into());
        }
        let entry: Entry = serde_json::from_slice(&line).map_err(|_| "股票计划日志格式异常")?;
        if entry.version != 1 {
            return Err("不支持的股票计划日志版本".into());
        }
        validate(&entry.plan)?;
        if let Some(old) = rows.get(&entry.plan.request.request_id) {
            if old.request != entry.plan.request
                || old.terms != entry.plan.terms
                || entry.plan.revision != old.revision.saturating_add(1)
                || entry.plan.updated_at_ms < old.updated_at_ms
                || (old.two_leg_started_at_ms.is_some() && old.two_leg_started_at_ms != entry.plan.two_leg_started_at_ms)
                || (old.two_leg_started_at_ms.is_none() && entry.plan.two_leg_started_at_ms.is_some()
                    && (old.phase != StockPlanPhase::Reserved || old.cex_order.is_some() || old.rfq_acceptance.is_some() || old.chain_submission.is_some()))
                || !order_protocol::transition(
                    old.cex_order.as_ref(),
                    entry.plan.cex_order.as_ref(),
                )
                || !rfq_acceptance::transition(
                    old.rfq_acceptance.as_ref(),
                    entry.plan.rfq_acceptance.as_ref(),
                )
                || !chain::transition(old.chain_submission.as_ref(), entry.plan.chain_submission.as_ref())
                || !settlement::transition(old, &entry.plan)
                || !matches!(
                    (old.phase, entry.plan.phase),
                    (
                        StockPlanPhase::Reserved,
                        StockPlanPhase::Cancelled | StockPlanPhase::SubmissionUnknown
                    ) | (
                        StockPlanPhase::SubmissionUnknown,
                        StockPlanPhase::SubmissionUnknown | StockPlanPhase::Settled
                    )
                )
            {
                return Err("股票计划日志存在身份变化或非法状态回退".into());
            }
        } else if entry.plan.phase != StockPlanPhase::Reserved
            || entry.plan.revision != 1
            || entry.plan.cex_order.is_some()
            || entry.plan.rfq_acceptance.is_some()
            || entry.plan.chain_submission.is_some()
            || entry.plan.two_leg_started_at_ms.is_some()
            || !entry.plan.native_topups.is_empty()
            || !entry.plan.recoveries.is_empty()
            || entry.plan.settlement.is_some()
        {
            return Err("股票计划日志缺少初始预留".into());
        }
        rows.insert(entry.plan.request.request_id.clone(), entry.plan);
    }
}

#[cfg(test)]
mod tests;
