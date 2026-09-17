use super::*;
use crate::services::onchain_comparison::stock_costs::execution;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner, WalletClaims};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};

const MAX_ROWS: usize = 256;
const MAX_LINE: u64 = 256 * 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;
pub(super) mod conversion;
pub(super) mod inventory;
mod native_topup;
pub(super) mod recovery;
mod submission;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    version: u8,
    plan: StockPeerPlan,
}
#[derive(Default)]
struct Inner {
    rows: BTreeMap<String, StockPeerPlan>,
    problem: Option<String>,
    _lock: Option<File>,
}
pub(super) struct Store {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
    claims: Arc<WalletClaims>,
}

impl Store {
    pub(super) fn load(path: Option<PathBuf>, claims: Arc<WalletClaims>) -> Self {
        let mut inner = Inner::default();
        if let Some(path) = path.as_deref() {
            inner.problem = (|| -> Result<(), String> {
                std::fs::create_dir_all(path.parent().ok_or("股票计划目录无效")?)
                    .map_err(|_| "股票计划目录不可用")?;
                let lock = plan_store::options(false)
                    .open(path.with_extension("lock"))
                    .map_err(|_| "股票计划锁不可用")?;
                lock.try_lock_exclusive()
                    .map_err(|_| "股票计划已由其他实例占用")?;
                inner._lock = Some(lock);
                read(path, &mut inner.rows)
            })()
            .err();
            let holds = inner
                .rows
                .values()
                .filter(|p| p.phase != StockPeerPlanPhase::Cancelled)
                .map(|p| hold(p).map(|h| (Owner::new(Module::StockPeer, &p.plan_id), h)))
                .collect();
            claims.restore(Module::StockPeer, holds, inner.problem.clone());
        }
        Self {
            path,
            inner: Mutex::new(inner),
            claims,
        }
    }
    pub(super) fn records(&self) -> Vec<StockPeerPlan> {
        let now = common::time::now_ms();
        let mut rows = self.inner.lock().rows.values().cloned().collect::<Vec<_>>();
        rows.sort_by_key(|p| (!p.holds_funds(now), std::cmp::Reverse(p.updated_at_ms)));
        rows.truncate(48);
        rows
    }
    pub(super) fn problem(&self) -> Option<String> {
        self.inner.lock().problem.clone()
    }
    pub(super) fn previous(
        &self,
        request: &StockPeerPlanRequest,
    ) -> Result<Option<StockPeerPlan>, String> {
        let i = self.inner.lock();
        if let Some(e) = &i.problem {
            return Err(e.clone());
        }
        let row = i.rows.get(&request.request_id);
        if row.is_some_and(|p| p.request != *request) {
            return Err("请求编号已绑定原钱包、方向和数量，不能替换参数".into());
        }
        Ok(row.cloned())
    }
    pub(super) fn reserve(
        &self,
        request: StockPeerPlanRequest,
        terms: StockPeerPlanTerms,
        now: i64,
    ) -> Result<StockPeerPlan, String> {
        let mut i = self.inner.lock();
        if let Some(e) = &i.problem {
            return Err(e.clone());
        }
        if let Some(old) = i.rows.get(&request.request_id) {
            return if old.request == request {
                Ok(old.clone())
            } else {
                Err("请求编号已绑定其他参数".into())
            };
        }
        if now < terms.created_at_ms || now >= terms.market_valid_until_ms {
            return Err("股票计划报价已失效，未预留".into());
        }
        if i.rows.len() >= MAX_ROWS {
            return Err("股票计划历史达到上限，请保留原日志".into());
        }
        let p = StockPeerPlan {
            plan_id: format!("stock-peer-{}", request.request_id),
            request,
            terms,
            phase: StockPeerPlanPhase::Reserved,
            revision: 1,
            updated_at_ms: now,
            cex_order: None,
            chain_submission: None,
            execution_problem: None,
            cex_history: StockPeerHistoryCheck::default(),
            recoveries: vec![],
            conversions: vec![],
            native_topups: vec![],
            inventory_orders: vec![],
        };
        self.persist(&mut i, &p, now)?;
        i.rows.insert(p.request.request_id.clone(), p.clone());
        Ok(p)
    }
    pub(super) fn cancel(
        &self,
        request: &StockPlanRevisionRequest,
        now: i64,
    ) -> Result<StockPeerPlan, String> {
        let mut i = self.inner.lock();
        if let Some(e) = &i.problem {
            return Err(e.clone());
        }
        let old = i
            .rows
            .values()
            .find(|p| p.plan_id == request.plan_id)
            .ok_or("股票双边计划不存在")?;
        if old.phase == StockPeerPlanPhase::Cancelled {
            return Ok(old.clone());
        }
        if old.phase != StockPeerPlanPhase::Reserved {
            return Err("双边提交已经开始，不能取消预留或释放未核对资金".into());
        }
        if old.revision != request.revision {
            return Err("计划版本已变化，请刷新后取消".into());
        }
        let mut p = old.clone();
        p.phase = StockPeerPlanPhase::Cancelled;
        p.revision += 1;
        p.updated_at_ms = now.max(p.updated_at_ms);
        transition(old, &p)?;
        self.persist(&mut i, &p, now)?;
        i.rows.insert(p.request.request_id.clone(), p.clone());
        Ok(p)
    }
    fn persist(&self, inner: &mut Inner, p: &StockPeerPlan, now: i64) -> Result<(), String> {
        let path = self
            .path
            .as_deref()
            .ok_or("股票双边计划持久化未配置，未预留资金")?;
        validate(p)?;
        let claim = (p.phase != StockPeerPlanPhase::Cancelled)
            .then(|| hold(p))
            .transpose()?;
        let mut write_problem = None;
        let result = self.claims.commit(
            Owner::new(Module::StockPeer, &p.plan_id),
            claim,
            now,
            || {
                let result = append(path, p);
                write_problem = result.as_ref().err().cloned();
                result
            },
        );
        if write_problem.is_some() {
            inner.problem = write_problem;
        }
        result
    }
}

fn hold(p: &StockPeerPlan) -> Result<Hold, String> {
    Hold::wallet(
        "solana",
        &p.request.wallet_address,
        (p.phase == StockPeerPlanPhase::Reserved).then_some(p.terms.reserved_until_ms),
    )?
    .with_account("kraken_stocks", "configured-account")
}

// Rebuild the evidence available to each independently approved child, not the
// final inventory after later trades. Late conflicts still block new actions.
fn historical_prefix(p: &StockPeerPlan, revision: u64) -> StockPeerPlan {
    let mut prefix = p.clone();
    prefix.revision = revision;
    prefix.recoveries.retain(|r| r.source_revision < revision);
    prefix.conversions.retain(|r| r.request.revision < revision);
    prefix
        .native_topups
        .retain(|r| r.terms.source_revision < revision);
    prefix
        .inventory_orders
        .retain(|r| r.request.revision < revision);
    for order in prefix
        .cex_order
        .iter_mut()
        .chain(
            prefix
                .conversions
                .iter_mut()
                .filter_map(|r| r.order.as_mut()),
        )
        .chain(
            prefix
                .inventory_orders
                .iter_mut()
                .filter_map(|r| r.order.as_mut()),
        )
    {
        order.evidence_conflict = false;
        order.problem = None;
    }
    prefix
}
fn validate(p: &StockPeerPlan) -> Result<(), String> {
    crate::services::onchain_comparison::stock_inventory::validate_owner(
        &p.request.wallet_address,
    )?;
    let rebuilt = prepare_peer_plan_terms(
        &p.request,
        p.terms.basis.clone(),
        p.terms.account_fingerprint.clone(),
        p.terms.created_at_ms,
    )?;
    if rebuilt != p.terms
        || p.plan_id != format!("stock-peer-{}", p.request.request_id)
        || p.updated_at_ms < p.terms.created_at_ms
        || !matches!(
            (p.phase, p.revision),
            (StockPeerPlanPhase::Reserved, 1)
                | (StockPeerPlanPhase::Cancelled, 2)
                | (StockPeerPlanPhase::SubmissionUnknown, 2..)
        )
        || p.phase == StockPeerPlanPhase::Reserved
            && p.updated_at_ms >= p.terms.market_valid_until_ms
    {
        return Err("股票计划原始参数、预算或版本不一致".into());
    }
    execution::validate_artifact(&p.terms.basis.chain_cost)?;
    if p.execution_problem.as_ref().is_some_and(|s| s.len() > 2048)
        || p.cex_history
            .problem
            .as_ref()
            .is_some_and(|s| s.len() > 2048)
    {
        return Err("股票提交诊断超过上限".into());
    }
    let h = &p.cex_history;
    if (h.attempts == 0 && *h != StockPeerHistoryCheck::default())
        || (h.attempts > 0
            && (p.phase != StockPeerPlanPhase::SubmissionUnknown
                || h.next_check_at_ms <= p.terms.created_at_ms))
        || h.checked_at_ms
            .is_some_and(|at| at < p.terms.created_at_ms || at > p.updated_at_ms)
    {
        return Err("股票历史核对记录无效".into());
    }
    match (&p.cex_order, &p.chain_submission, p.phase) {
        (Some(c), Some(chain), StockPeerPlanPhase::SubmissionUnknown) => {
            c.validate_stored()?;
            if c.draft != p.terms.draft
                || chain.submitted_at_ms < p.terms.created_at_ms
                || chain.submitted_at_ms >= p.terms.market_valid_until_ms
                || chain.submitted_at_ms > p.updated_at_ms
            {
                return Err("双边原交易未绑定预留时的参数或有效期".into());
            }
            execution::validate_record(&p.terms.basis.chain_cost, chain)
        }
        (None, None, StockPeerPlanPhase::Reserved | StockPeerPlanPhase::Cancelled)
            if p.execution_problem.is_none() =>
        {
            Ok(())
        }
        _ => Err("股票双边提交记录缺少一条腿".into()),
    }?;
    recovery::validate_history(p)?;
    conversion::validate_history(p)?;
    native_topup::validate_history(p)?;
    inventory::validate_history(p)
}
fn transition(old: &StockPeerPlan, p: &StockPeerPlan) -> Result<(), String> {
    if old.plan_id != p.plan_id
        || old.request != p.request
        || old.terms != p.terms
        || p.revision != old.revision + 1
        || p.updated_at_ms < old.updated_at_ms
        || p.cex_history.attempts < old.cex_history.attempts
        || p.cex_history.attempts > old.cex_history.attempts.saturating_add(1)
        || p.cex_history.next_check_at_ms < old.cex_history.next_check_at_ms
        || !recovery::transition(old, p)
        || !conversion::transition(old, p)
        || !native_topup::transition(old, p)
        || !inventory::transition(old, p)
        || old
            .cex_history
            .checked_at_ms
            .is_some_and(|at| p.cex_history.checked_at_ms.is_none_or(|next| next < at))
    {
        return Err("股票计划历史发生回退或原始参数变化".into());
    }
    match (old.phase, p.phase) {
        (StockPeerPlanPhase::Reserved, StockPeerPlanPhase::Cancelled) => Ok(()),
        (StockPeerPlanPhase::Reserved, StockPeerPlanPhase::SubmissionUnknown) => {
            let c = p.cex_order.as_ref().ok_or("缺少原订单")?;
            if *c != StockPeerOrderReceipt::pending(c.draft.clone(), c.client_order_id.clone())?
                || p.chain_submission
                    .as_ref()
                    .is_none_or(|r| r.receipt.is_some() || r.provider_acknowledged)
            {
                return Err("首次提交意图不能预填成交或 Provider 确认".into());
            }
            Ok(())
        }
        (StockPeerPlanPhase::SubmissionUnknown, StockPeerPlanPhase::SubmissionUnknown) => {
            let mut merged = old.cex_order.clone().ok_or("原订单丢失")?;
            let incoming = p.cex_order.as_ref().ok_or("原订单丢失")?;
            let _ = merged.merge_snapshot(incoming);
            if merged != *incoming
                || !execution::transition(
                    old.chain_submission.as_ref(),
                    p.chain_submission.as_ref(),
                )
            {
                return Err("双边回执发生回退或原交易变化".into());
            }
            Ok(())
        }
        _ => Err("股票双边计划不能退回未提交状态".into()),
    }
}
fn append(path: &Path, p: &StockPeerPlan) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&Entry {
        version: 1,
        plan: p.clone(),
    })
    .map_err(|_| "股票计划编码失败")?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_LINE {
        return Err("股票计划超过行上限".into());
    }
    let mut file = plan_store::options(true)
        .open(path)
        .map_err(|_| "股票计划日志不可写")?;
    if file
        .metadata()
        .map_err(|_| "股票计划大小未知")?
        .len()
        .saturating_add(bytes.len() as u64)
        > MAX_FILE
    {
        return Err("股票计划日志达到上限，请保留原文件".into());
    }
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "股票计划写入结果不明，停止预留并保留原日志")?;
    File::open(path.parent().ok_or("股票计划目录无效")?)
        .and_then(|f| f.sync_all())
        .map_err(|_| "股票计划目录同步失败".into())
}
fn read(path: &Path, rows: &mut BTreeMap<String, StockPeerPlan>) -> Result<(), String> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("股票计划不可读取".into()),
    };
    if file.metadata().map_err(|_| "股票计划大小未知")?.len() > MAX_FILE {
        return Err("股票计划超过读取上限".into());
    }
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = Read::by_ref(&mut reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| "股票计划读取失败")?;
        if n == 0 {
            break;
        }
        if n as u64 > MAX_LINE || line.last() != Some(&b'\n') {
            return Err("股票计划日志尾部不完整或行过长，请保留原文件".into());
        }
        let entry: Entry = serde_json::from_slice(&line).map_err(|_| "股票计划日志损坏")?;
        if entry.version != 1 {
            return Err("股票计划日志版本未知".into());
        }
        let p = entry.plan;
        validate(&p)?;
        if let Some(old) = rows.get(&p.request.request_id) {
            if old == &p {
                continue;
            }
            transition(old, &p)?;
        } else if p.phase != StockPeerPlanPhase::Reserved || p.revision != 1 {
            return Err("股票计划缺少原始预留记录".into());
        }
        if p.cex_order.as_ref().is_some_and(|c| {
            rows.values().any(|old| {
                old.plan_id != p.plan_id
                    && old
                        .cex_order
                        .as_ref()
                        .is_some_and(|other| other.client_order_id == c.client_order_id)
            })
        }) {
            return Err("股票订单编号被多个计划占用".into());
        }
        rows.insert(p.request.request_id.clone(), p);
        if rows.len() > MAX_ROWS {
            return Err("股票计划记录超过上限".into());
        }
    }
    Ok(())
}
