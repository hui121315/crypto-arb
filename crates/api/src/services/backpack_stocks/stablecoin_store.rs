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
const MAX_LINE: u64 = 128 * 1024;
const MAX_FILE: u64 = 16 * 1024 * 1024;
pub(super) mod native_topup;
mod submission;

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u8,
    plan: StockStablecoinPlan,
}

#[derive(Default)]
struct Inner {
    rows: BTreeMap<String, StockStablecoinPlan>,
    problem: Option<String>,
    _lock: Option<File>,
}

pub(super) struct StablecoinStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
    claims: Arc<WalletClaims>,
}

impl StablecoinStore {
    pub(super) fn load(path: Option<PathBuf>, claims: Arc<WalletClaims>) -> Self {
        let mut inner = Inner::default();
        if let Some(path) = path.as_deref() {
            inner.problem = (|| -> Result<(), String> {
                let parent = path.parent().ok_or("兑换日志目录无效")?;
                std::fs::create_dir_all(parent).map_err(|_| "兑换日志目录不可用")?;
                let file = plan_store::options(false)
                    .open(path.with_extension("lock"))
                    .map_err(|_| "兑换日志锁不可用")?;
                file.try_lock_exclusive()
                    .map_err(|_| "兑换日志已由其他实例占用")?;
                inner._lock = Some(file);
                read_rows(path, &mut inner.rows)
            })()
            .err();
            let holds = inner
                .rows
                .values()
                .filter(|p| claims_required(p))
                .map(|p| hold(p).map(|h| (Owner::new(Module::StockStablecoin, &p.plan_id), h)))
                .collect::<Result<Vec<_>, _>>();
            claims.restore(Module::StockStablecoin, holds, inner.problem.clone());
        }
        Self {
            path,
            inner: Mutex::new(inner),
            claims,
        }
    }

    pub(super) fn records(&self) -> Vec<StockStablecoinPlan> {
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
        request: &StockStablecoinPlanRequest,
    ) -> Result<Option<StockStablecoinPlan>, String> {
        let inner = self.inner.lock();
        if let Some(problem) = &inner.problem {
            return Err(problem.clone());
        }
        let old = inner.rows.get(&request.request_id);
        if old.is_some_and(|p| p.request != *request) {
            return Err("同一兑换请求不能更换钱包、投入、目标或原报价".into());
        }
        Ok(old.cloned())
    }

    pub(super) fn reserve(
        &self,
        request: StockStablecoinPlanRequest,
        preview: StockStablecoinPreview,
        now: i64,
    ) -> Result<StockStablecoinPlan, String> {
        let mut inner = self.inner.lock();
        if let Some(old) = inner.rows.get(&request.request_id) {
            if old.request != request {
                return Err("兑换请求编号已绑定其他参数".into());
            }
            if let Some(problem) = &inner.problem {
                return Err(problem.clone());
            }
            return Ok(old.clone());
        }
        if !preview.can_reserve(now) {
            return Err("兑换证据过期或有未解决缺口，请重新试算".into());
        }
        if inner.rows.len() >= MAX_ROWS {
            return Err("兑换历史达到保存上限，请保留原日志".into());
        }
        let plan = StockStablecoinPlan {
            plan_id: id(&request, &preview)?,
            request,
            preview,
            phase: StockStablecoinPlanPhase::Reserved,
            revision: 1,
            updated_at_ms: now,
            submission: None,
            native_topups: vec![],
        };
        validate(&plan)?;
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
    ) -> Result<StockStablecoinPlan, String> {
        let mut inner = self.inner.lock();
        if let Some(problem) = &inner.problem {
            return Err(problem.clone());
        }
        let old = inner
            .rows
            .values()
            .find(|p| p.plan_id == request.plan_id)
            .ok_or("兑换计划不存在")?;
        if old.phase == StockStablecoinPlanPhase::Cancelled {
            return Ok(old.clone());
        }
        if old.submission.is_some() {
            return Err("兑换已进入提交，不能取消或释放；请核对原交易".into());
        }
        if old.revision != request.revision {
            return Err("兑换计划版本已变化，请刷新后取消".into());
        }
        let mut plan = old.clone();
        plan.phase = StockStablecoinPlanPhase::Cancelled;
        plan.revision = plan.revision.checked_add(1).ok_or("兑换版本溢出")?;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        transition(old, &plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }

    fn persist(
        &self,
        inner: &mut Inner,
        plan: &StockStablecoinPlan,
        now: i64,
    ) -> Result<(), String> {
        if let Some(problem) = &inner.problem {
            return Err(problem.clone());
        }
        let path = self
            .path
            .as_deref()
            .ok_or("兑换计划持久化未配置，未预留资金")?;
        validate(plan)?;
        let claim = claims_required(plan).then(|| hold(plan)).transpose()?;
        let mut write_problem = None;
        let result = self.claims.commit(
            Owner::new(Module::StockStablecoin, &plan.plan_id),
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

fn hold(p: &StockStablecoinPlan) -> Result<Hold, String> {
    Hold::wallet(
        "solana",
        &p.request.conversion.wallet_address,
        if p.phase == StockStablecoinPlanPhase::Reserved {
            Some(p.preview.valid_until_ms)
        } else if p.phase == StockStablecoinPlanPhase::Completed && p.native_accounting().is_ok() {
            p.native_topups
                .last()
                .filter(|r| r.cancelled_at_ms.is_none() && r.terms.submission.is_none())
                .and_then(|r| {
                    r.terms
                        .valuation
                        .replenishment
                        .as_ref()
                        .map(|p| p.valid_until_ms)
                })
        } else {
            None
        },
    )
}

fn claims_required(p: &StockStablecoinPlan) -> bool {
    matches!(
        p.phase,
        StockStablecoinPlanPhase::Reserved
            | StockStablecoinPlanPhase::SubmissionUnknown
            | StockStablecoinPlanPhase::NeedsReview
    ) || p.native_topups.last().is_some_and(|r|r.cancelled_at_ms.is_none() && r.terms.submission.is_none())
        || (!p.native_topups.is_empty() && p.native_accounting().is_err())
}

fn id(r: &StockStablecoinPlanRequest, p: &StockStablecoinPreview) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(r, p)).map_err(|_| "兑换计划无法编码")?;
    let hash = common::signing::hmac_sha256_hex(b"stock-stablecoin-plan-v1", &bytes);
    Ok(format!("stock-stablecoin-{}", &hash[..32]))
}

pub(super) fn validate_request(r: &StockStablecoinPlanRequest) -> Result<(), String> {
    if !(16..=128).contains(&r.request_id.len())
        || !r
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("兑换请求编号无效".into());
    }
    r.conversion.amounts_raw()?;
    crate::services::onchain_comparison::stock_inventory::validate_owner(
        &r.conversion.wallet_address,
    )
}

fn validate(p: &StockStablecoinPlan) -> Result<(), String> {
    validate_request(&p.request)?;
    let q = &p.preview;
    let cost = q.cost.as_ref().ok_or("兑换缺少原交易")?;
    let rebuilt = stablecoin_preview(
        q.request.clone(),
        q.wallet.clone(),
        q.quote.clone(),
        q.cost.clone(),
        vec![],
        q.checked_at_ms,
    )?;
    if &rebuilt != q
        || !q.can_reserve(q.checked_at_ms)
        || p.request.conversion != q.request
        || p.request.preview_at_ms != q.checked_at_ms
        || p.request.transaction_fingerprint != cost.transaction_fingerprint
        || p.plan_id != id(&p.request, q)?
        || p.updated_at_ms < q.checked_at_ms
        || p.phase == StockStablecoinPlanPhase::Reserved && p.updated_at_ms >= q.valid_until_ms
    {
        return Err("兑换计划身份、金额、原报价或状态无效".into());
    }
    execution::validate_artifact(cost)?;
    native_topup::validate_history(p)?;
    match &p.submission {
        None if matches!(
            (p.phase, p.revision),
            (StockStablecoinPlanPhase::Reserved, 1) | (StockStablecoinPlanPhase::Cancelled, 2)
        ) && p.native_topups.is_empty() =>
        {
            Ok(())
        }
        Some(s)
            if p.revision >= 2
                && s.submitted_at_ms >= q.checked_at_ms
                && s.submitted_at_ms < q.valid_until_ms
                && p.updated_at_ms >= s.submitted_at_ms
                && p.phase == p.receipt_phase() =>
        {
            execution::validate_record(cost, s)
        }
        _ => Err("兑换提交时间、回执或状态不一致".into()),
    }
}

fn transition(old: &StockStablecoinPlan, next: &StockStablecoinPlan) -> Result<(), String> {
    validate(next)?;
    if old.plan_id != next.plan_id
        || old.request != next.request
        || old.preview != next.preview
        || !matches!(
            old.phase,
            StockStablecoinPlanPhase::Reserved
                | StockStablecoinPlanPhase::SubmissionUnknown
                | StockStablecoinPlanPhase::Completed
        )
        || (old.phase == StockStablecoinPlanPhase::Reserved
            && !matches!(
                next.phase,
                StockStablecoinPlanPhase::Cancelled | StockStablecoinPlanPhase::SubmissionUnknown
            ))
        || !execution::transition(old.submission.as_ref(), next.submission.as_ref())
        || !native_topup::transition(old, next)
        || next.revision != old.revision + 1
        || next.updated_at_ms < old.updated_at_ms
    {
        return Err("兑换历史不能改写金额、复活预留或回退版本".into());
    }
    Ok(())
}

fn append(path: &Path, p: &StockStablecoinPlan) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&Entry {
        version: 1,
        plan: p.clone(),
    })
    .map_err(|_| "兑换日志无法编码")?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_LINE {
        return Err("兑换记录超过行上限".into());
    }
    let mut file = plan_store::options(true)
        .open(path)
        .map_err(|_| "兑换日志无法打开")?;
    if file
        .metadata()
        .map_err(|_| "兑换日志大小未知")?
        .len()
        .saturating_add(bytes.len() as u64)
        > MAX_FILE
    {
        return Err("兑换日志超过上限，请保留原文件".into());
    }
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "兑换写入结果不明，停止新预留并保留原文件")?;
    File::open(path.parent().ok_or("兑换日志目录无效")?)
        .and_then(|f| f.sync_all())
        .map_err(|_| "兑换目录同步失败".to_owned())
}

fn read_rows(path: &Path, rows: &mut BTreeMap<String, StockStablecoinPlan>) -> Result<(), String> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("兑换日志无法读取".into()),
    };
    if file.metadata().map_err(|_| "兑换日志大小未知")?.len() > MAX_FILE {
        return Err("兑换日志超过读取上限".into());
    }
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = Read::by_ref(&mut reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| "兑换日志读取失败")?;
        if n == 0 {
            break;
        }
        if n as u64 > MAX_LINE || line.last() != Some(&b'\n') {
            return Err("兑换日志有超大行或未完成尾部，请保留原文件".into());
        }
        let entry: Entry = serde_json::from_slice(&line).map_err(|_| "兑换日志格式损坏")?;
        if entry.version != 1 {
            return Err("兑换日志版本未知".into());
        }
        let p = entry.plan;
        validate(&p)?;
        if let Some(old) = rows.get(&p.request.request_id) {
            if old == &p {
                continue;
            }
            transition(old, &p)?;
        } else if p.phase != StockStablecoinPlanPhase::Reserved || p.revision != 1 {
            return Err("兑换历史缺少初始计划".into());
        }
        rows.insert(p.request.request_id.clone(), p);
        if rows.len() > MAX_ROWS {
            return Err("兑换历史数量超过上限".into());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests;
