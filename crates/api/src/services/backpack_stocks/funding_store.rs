use super::*;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner, WalletClaims};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u8,
    plan: StockFundingPlan,
}
#[derive(Default)]
struct Inner {
    rows: BTreeMap<String, StockFundingPlan>,
    problem: Option<String>,
    _lock: Option<File>,
}
pub(super) struct FundingStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
    claims: Arc<WalletClaims>,
}

impl FundingStore {
    pub(super) fn load(path: Option<PathBuf>, claims: Arc<WalletClaims>) -> Self {
        let mut inner = Inner::default();
        if let Some(path) = path.as_deref() {
            inner.problem = (|| -> Result<(), String> {
                let parent = path.parent().ok_or("补库日志目录无效")?;
                std::fs::create_dir_all(parent).map_err(|_| "补库日志目录不可用")?;
                let lock = plan_store::options(false)
                    .open(path.with_extension("lock"))
                    .map_err(|_| "补库日志锁不可用")?;
                lock.try_lock_exclusive()
                    .map_err(|_| "补库日志已被其他实例占用")?;
                inner._lock = Some(lock);
                read_rows(path, &mut inner.rows)
            })()
            .err();
            let holds = inner
                .rows
                .values()
                .filter(|p| p.phase.holds_funds())
                .map(|p| hold(p).map(|h| (Owner::new(Module::StockFunding, &p.plan_id), h)))
                .collect::<Result<Vec<_>, _>>();
            claims.restore(Module::StockFunding, holds, inner.problem.clone());
        }
        Self {
            path,
            inner: Mutex::new(inner),
            claims,
        }
    }
    pub(super) fn records(&self) -> Vec<StockFundingPlan> {
        let now = common::time::now_ms();
        let mut rows = self.inner.lock().rows.values().cloned().collect::<Vec<_>>();
        rows.sort_by_key(|p| {
            (
                !p.phase_at(now).holds_funds(),
                std::cmp::Reverse(p.terms.created_at_ms),
            )
        });
        rows.truncate(48);
        rows
    }
    pub(super) fn problem(&self) -> Option<String> {
        self.inner.lock().problem.clone()
    }
    pub(super) fn next_followup(&self) -> Option<(String, i64)> {
        let inner = self.inner.lock();
        if inner.problem.is_some() { return None; }
        inner.rows.values().filter_map(|p| Some((p.plan_id.clone(), p.funding_followup_at()?)))
            .min_by_key(|(_, at)| *at)
    }
    pub(super) fn update_followup(
        &self,
        expected: &StockFundingPlan,
        followup: StockFundingFollowup,
        now: i64,
    ) -> Result<StockFundingPlan, String> {
        let mut inner = self.inner.lock();
        let old = inner.rows.get(&expected.request.request_id).ok_or("补库计划不存在")?;
        if old != expected { return Err("补库记录已变化，未重复查询".into()); }
        let mut plan = old.clone();
        plan.followup = Some(followup);
        plan.revision = plan.revision.checked_add(1).ok_or("补库版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        funding_withdrawal::transition(old, &plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner.rows.insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }
    pub(super) fn get(&self, id: &str) -> Result<StockFundingPlan, String> {
        let inner = self.inner.lock();
        if let Some(problem) = &inner.problem {
            return Err(problem.clone());
        }
        inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("补库计划不存在".into())
    }
    pub(super) fn update(
        &self,
        expected: &StockFundingPlan,
        withdrawal: StockFundingWithdrawal,
        now: i64,
    ) -> Result<StockFundingPlan, String> {
        let mut inner = self.inner.lock();
        let old = inner
            .rows
            .get(&expected.request.request_id)
            .ok_or("补库计划不存在")?;
        if old != expected {
            return Err("补库计划已变化，请核对原记录".into());
        }
        if old.withdrawal.as_ref() == Some(&withdrawal) {
            return Ok(old.clone());
        }
        let mut plan = old.clone();
        plan.phase = if withdrawal.receipt.is_some() {
            StockFundingPlanPhase::Received
        } else {
            StockFundingPlanPhase::Withdrawing
        };
        plan.withdrawal = Some(withdrawal);
        plan.revision = plan.revision.checked_add(1).ok_or("补库版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        funding_withdrawal::transition(old, &plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }
    pub(super) fn previous(
        &self,
        request: &StockFundingPlanRequest,
        fingerprint: &str,
    ) -> Result<Option<StockFundingPlan>, String> {
        let inner = self.inner.lock();
        let old = inner.rows.get(&request.request_id);
        if old.is_some_and(|p| p.request != *request || p.terms.account_fingerprint != fingerprint)
        {
            return Err("该请求编号已绑定其他补库参数或账户，不能重用".into());
        }
        Ok(old.cloned())
    }
    pub(super) fn update_transfer(
        &self,
        expected: &StockFundingPlan,
        transfer: StockFundingTransfer,
        now: i64,
    ) -> Result<StockFundingPlan, String> {
        let mut inner=self.inner.lock();
        let old=inner.rows.get(&expected.request.request_id).ok_or("补库计划不存在")?;
        if old!=expected {return Err("补库计划已变化，请核对原记录".into());}
        if old.transfer.as_ref()==Some(&transfer) {return Ok(old.clone());}
        let mut plan=old.clone();
        plan.phase=funding_transfer::phase(&plan,&transfer);
        plan.transfer=Some(transfer);
        plan.revision=plan.revision.checked_add(1).ok_or("补库版本溢出")?;
        plan.updated_at_ms=now.max(plan.updated_at_ms);
        funding_withdrawal::transition(old,&plan)?;
        self.persist(&mut inner,&plan,now)?;
        inner.rows.insert(plan.request.request_id.clone(),plan.clone());
        Ok(plan)
    }
    pub(super) fn insert(
        &self,
        plan: StockFundingPlan,
        now: i64,
    ) -> Result<StockFundingPlan, String> {
        funding_plan::validate(&plan)?;
        let mut inner = self.inner.lock();
        if let Some(old) = inner.rows.get(&plan.request.request_id) {
            if old.request != plan.request
                || old.terms.account_fingerprint != plan.terms.account_fingerprint
            {
                return Err("同一请求不能重新规划补库金额或账户".into());
            }
            return Ok(old.clone());
        }
        if plan.phase != StockFundingPlanPhase::Reserved
            || plan.revision != 1
            || plan.transfer.is_some()
            || plan.followup.is_some()
            || now >= plan.terms.valid_until_ms
            || now < plan.terms.created_at_ms
        {
            return Err("补库计划已过期或初始状态无效".into());
        }
        if inner.rows.len() >= 256 {
            return Err("补库计划历史达到保存上限，请保留并整理原日志".into());
        }
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }
    pub(super) fn cancel(
        &self,
        request: &StockPlanRevisionRequest,
        now: i64,
    ) -> Result<StockFundingPlan, String> {
        let mut inner = self.inner.lock();
        let mut plan = inner
            .rows
            .values()
            .find(|p| p.plan_id == request.plan_id)
            .cloned()
            .ok_or("补库计划不存在")?;
        if plan.phase == StockFundingPlanPhase::Cancelled {
            return Ok(plan);
        }
        if plan.withdrawal.is_some() || plan.transfer.as_ref().is_some_and(|t|t.submitted_at_ms.is_some()) {
            return Err("补库已记录提交，不能取消、过期释放或重新转账；请查询原交易".into());
        }
        if request.revision != plan.revision {
            return Err("补库计划已变化，请刷新后取消".into());
        }
        plan.phase = StockFundingPlanPhase::Cancelled;
        plan.revision = plan.revision.checked_add(1).ok_or("版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }
    fn persist(&self, inner: &mut Inner, plan: &StockFundingPlan, now: i64) -> Result<(), String> {
        if let Some(problem) = &inner.problem {
            return Err(problem.clone());
        }
        let path = self
            .path
            .as_deref()
            .ok_or("补库持久化未配置，不能预留资金")?;
        funding_plan::validate(plan)?;
        let claim = if plan.phase.holds_funds() {
            Some(hold(plan)?)
        } else {
            None
        };
        let mut write_problem = None;
        let result = self.claims.commit(
            Owner::new(Module::StockFunding, &plan.plan_id),
            claim,
            now,
            || {
                let result = append(path, plan);
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

fn hold(plan: &StockFundingPlan) -> Result<Hold, String> {
    Hold::wallet(
        "solana",
        &plan.request.wallet_address,
        if plan.withdrawal.is_some() || plan.transfer.as_ref().is_some_and(|t|t.submitted_at_ms.is_some()) {
            None
        } else {
            Some(plan.terms.valid_until_ms)
        },
    )?
    .with_account("backpack_stocks", "configured-account")
}
fn append(path: &Path, plan: &StockFundingPlan) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&Entry {
        version: 1,
        plan: plan.clone(),
    })
    .map_err(|_| "补库记录无法编码")?;
    if bytes.len() > 128 * 1024 {
        return Err("补库记录超出上限".into());
    }
    bytes.push(b'\n');
    let mut file = plan_store::options(true)
        .open(path)
        .map_err(|_| "补库日志无法打开，未预留")?;
    if file
        .metadata()
        .map_err(|_| "补库日志大小未知")?
        .len()
        .saturating_add(bytes.len() as u64)
        > 16 * 1024 * 1024
    {
        return Err("补库日志超过上限，请保留原文件".into());
    }
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "补库日志写入结果不明，停止新预留并保留文件")?;
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|_| "补库目录同步失败")?;
    }
    Ok(())
}
fn read_rows(path: &Path, rows: &mut BTreeMap<String, StockFundingPlan>) -> Result<(), String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("补库日志无法读取".into()),
    };
    if file.metadata().map_err(|_| "补库日志大小未知")?.len() > 16 * 1024 * 1024 {
        return Err("补库日志超过读取上限".into());
    }
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = Read::by_ref(&mut reader)
            .take(128 * 1024 + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| "补库日志读取失败")?;
        if n == 0 {
            break;
        }
        if n > 128 * 1024 || line.last() != Some(&b'\n') {
            return Err("补库日志存在超大行或未完成的尾部，请保留原文件".into());
        }
        let entry: Entry = serde_json::from_slice(&line).map_err(|_| "补库日志格式损坏")?;
        if entry.version != 1 {
            return Err("补库日志版本未知".into());
        }
        let p = entry.plan;
        funding_plan::validate(&p)?;
        if let Some(old) = rows.get(&p.request.request_id) {
            if old == &p {
                continue;
            }
            funding_withdrawal::transition(old, &p)?;
        } else if p.revision != 1 || p.phase != StockFundingPlanPhase::Reserved || p.transfer.is_some() || p.followup.is_some() {
            return Err("补库历史缺少初始计划".into());
        }
        rows.insert(p.request.request_id.clone(), p);
        if rows.len() > 256 {
            return Err("补库历史数量超过上限".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
