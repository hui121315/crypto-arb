use super::*;
use crate::services::onchain_wallet_claims::{Hold, Module, Owner, WalletClaims};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};

const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_LINE: u64 = 128 * 1024;
#[derive(Serialize, Deserialize)]
struct Entry {
    version: u8,
    plan: StockExchangeConversionPlan,
}
#[derive(Default)]
struct Inner {
    rows: BTreeMap<String, StockExchangeConversionPlan>,
    problem: Option<String>,
    _lock: Option<File>,
}
pub(in crate::services::backpack_stocks) struct Store {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
    claims: Arc<WalletClaims>,
}
impl Store {
    pub(in crate::services::backpack_stocks) fn load(
        path: Option<PathBuf>,
        claims: Arc<WalletClaims>,
    ) -> Self {
        let mut inner = Inner::default();
        if let Some(path) = path.as_deref() {
            inner.problem = (|| -> Result<(), String> {
                std::fs::create_dir_all(path.parent().ok_or("账户兑换目录无效")?)
                    .map_err(|_| "账户兑换目录不可用")?;
                let lock = super::super::plan_store::options(false)
                    .open(path.with_extension("lock"))
                    .map_err(|_| "账户兑换日志锁不可用")?;
                lock.try_lock_exclusive()
                    .map_err(|_| "账户兑换日志已被其他实例占用")?;
                inner._lock = Some(lock);
                read(path, &mut inner.rows)
            })()
            .err();
            let holds = inner
                .rows
                .values()
                .filter_map(|p| {
                    hold(p).transpose().map(|h| {
                        h.map(|h| (Owner::new(Module::StockExchangeConversion, &p.plan_id), h))
                    })
                })
                .collect();
            claims.restore(
                Module::StockExchangeConversion,
                holds,
                inner.problem.clone(),
            );
        }
        Self {
            path,
            inner: Mutex::new(inner),
            claims,
        }
    }
    pub(in crate::services::backpack_stocks) fn rows(&self) -> Vec<StockExchangeConversionPlan> {
        let mut rows = self.inner.lock().rows.values().cloned().collect::<Vec<_>>();
        rows.sort_by_key(|p| {
            (
                !p.holds_funds(common::time::now_ms()),
                std::cmp::Reverse(p.updated_at_ms),
            )
        });
        rows
    }
    pub(in crate::services::backpack_stocks) fn problem(&self) -> Option<String> {
        self.inner.lock().problem.clone()
    }
    // Keep source receipts stable until the dependent plan's journal append has completed.
    pub(in crate::services::backpack_stocks) fn with_costs<T>(
        &self,
        ids: &[String],
        fingerprint: &str,
        apply: impl FnOnce(Vec<StockExchangeConversionPlan>) -> Result<T, String>,
    ) -> Result<T, String> {
        if ids.is_empty() {
            return apply(vec![]);
        }
        if ids.len() > STOCK_CONVERSION_COST_LIMIT
            || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err("兑换费用最多 8 笔，且不能重复选择".into());
        }
        let inner = self.inner.lock();
        if let Some(problem) = &inner.problem {
            return Err(problem.clone());
        }
        let sources = ids
            .iter()
            .map(|id| {
                let p = inner
                    .rows
                    .values()
                    .find(|p| &p.plan_id == id)
                    .ok_or("原兑换费用记录不存在")?;
                if p.terms.account_fingerprint != fingerprint {
                    return Err("兑换费用属于其他账户凭证".into());
                }
                validate(p)?;
                p.confirmed_fee_usdc()?;
                Ok(p.clone())
            })
            .collect::<Result<Vec<_>, String>>()?;
        apply(sources)
    }
    pub(super) fn get(&self, id: &str) -> Result<StockExchangeConversionPlan, String> {
        self.inner
            .lock()
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("账户兑换计划不存在".into())
    }
    pub(super) fn previous(
        &self,
        r: &StockExchangeConversionRequest,
        fp: &str,
    ) -> Result<Option<StockExchangeConversionPlan>, String> {
        let inner = self.inner.lock();
        if let Some(e) = &inner.problem {
            return Err(e.clone());
        }
        let old = inner.rows.get(&r.request_id);
        if old.is_some_and(|p| p.request != *r || p.terms.account_fingerprint != fp) {
            return Err("兑换请求已绑定其他金额或账户".into());
        }
        Ok(old.cloned())
    }
    pub(super) fn insert(
        &self,
        p: StockExchangeConversionPlan,
        now: i64,
    ) -> Result<StockExchangeConversionPlan, String> {
        let mut inner = self.inner.lock();
        if let Some(old) = inner.rows.get(&p.request.request_id) {
            return if old.request == p.request
                && old.terms.account_fingerprint == p.terms.account_fingerprint
            {
                Ok(old.clone())
            } else {
                Err("兑换请求编号冲突".into())
            };
        }
        if inner.rows.len() >= 256 || !p.can_submit(now) || p.revision != 1 {
            return Err("兑换计划过期、状态无效或历史达到上限".into());
        }
        let client = client_id(&p)?;
        if inner
            .rows
            .values()
            .any(|old| client_id(old).ok() == Some(client))
        {
            return Err("兑换 clientId 与已有记录冲突，请重新生成请求".into());
        }
        validate(&p)?;
        self.persist(&mut inner, &p, now)?;
        inner.rows.insert(p.request.request_id.clone(), p.clone());
        Ok(p)
    }
    pub(super) fn change(
        &self,
        id: &str,
        now: i64,
        apply: impl FnOnce(&mut StockExchangeConversionPlan) -> Result<bool, String>,
    ) -> Result<StockExchangeConversionPlan, String> {
        let mut inner = self.inner.lock();
        if let Some(e) = &inner.problem {
            return Err(e.clone());
        }
        let old = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("账户兑换计划不存在")?;
        let mut next = old.clone();
        if !apply(&mut next)? {
            return Ok(old);
        }
        next.revision = old.revision.checked_add(1).ok_or("兑换版本溢出")?;
        next.updated_at_ms = now.max(old.updated_at_ms);
        transition(&old, &next)?;
        self.persist(&mut inner, &next, now)?;
        inner
            .rows
            .insert(next.request.request_id.clone(), next.clone());
        Ok(next)
    }
    fn persist(
        &self,
        inner: &mut Inner,
        p: &StockExchangeConversionPlan,
        now: i64,
    ) -> Result<(), String> {
        if let Some(e) = &inner.problem {
            return Err(e.clone());
        }
        let path = self
            .path
            .as_deref()
            .ok_or("账户兑换持久化未配置，不能预留资金")?;
        let mut write_error = None;
        let persist = || {
            let result = append(path, p);
            write_error = result.as_ref().err().cloned();
            result
        };
        let result = if p.order.as_ref().is_some_and(|o| o.evidence_conflict) {
            // A late conflict must be recorded even after this account has been reused by a stock plan.
            self.claims.persist_unresolved(
                Module::StockExchangeConversion,
                "兑换原回执存在冲突".into(),
                persist,
            )
        } else {
            self.claims.commit(
                Owner::new(Module::StockExchangeConversion, &p.plan_id),
                hold(p)?,
                now,
                persist,
            )
        };
        if write_error.is_some() {
            inner.problem = write_error;
        }
        result
    }
}
fn hold(p: &StockExchangeConversionPlan) -> Result<Option<Hold>, String> {
    if p.cancelled_at_ms.is_some() || p.order.is_some() && p.accounting().is_ok() {
        return Ok(None);
    }
    Hold {
        wallets: BTreeSet::new(),
        expires_at_ms: p.order.is_none().then_some(p.terms.valid_until_ms),
    }
    .with_account("backpack_stocks", "configured-account")
    .map(Some)
}
pub(super) fn client_id(p: &StockExchangeConversionPlan) -> Result<u32, String> {
    match p.terms.instruction {
        StockCexInstruction::OrderBook { client_id, .. } => Ok(client_id),
        _ => Err("兑换不能使用股票 RFQ".into()),
    }
}
pub(super) fn id(
    r: &StockExchangeConversionRequest,
    t: &StockExchangeConversionTerms,
) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(r, t)).map_err(|_| "兑换计划无法编码")?;
    Ok(format!(
        "stock-cex-convert-{}",
        &common::signing::hmac_sha256_hex(b"stock-cex-convert-v1", &bytes)[..32]
    ))
}
pub(in crate::services::backpack_stocks) fn validate(
    p: &StockExchangeConversionPlan,
) -> Result<(), String> {
    let t = &p.terms;
    let (input, _) = p.request.amounts()?;
    let (fee, net) = exchange_conversion_amounts(&p.request, &t.market, &t.book, &t.taker_fee_bps)?;
    let expected = StockCexInstruction::OrderBook {
        client_id: client_id(p)?,
        symbol: STOCK_CONVERSION_SYMBOL.into(),
        side: StockRfqSide::Ask,
        quantity: input.normalize().to_string(),
        limit_price: t.book.bid.clone().ok_or("兑换买价缺失")?,
    };
    let available = order_protocol::decimal(&t.available_usdt)?;
    let deadline = valid_until(&t.market, &t.book, t.balance_at_ms, t.fees_at_ms);
    if p.plan_id != id(&p.request, t)?
        || t.instruction != expected
        || client_id(p)? == 0
        || t.account_fingerprint.is_empty()
        || t.fee_budget_usdc != fee.normalize().to_string()
        || t.minimum_net_usdc != net.normalize().to_string()
        || available < input
        || t.valid_until_ms != deadline
        || t.created_at_ms <= 0
        || t.created_at_ms >= deadline
        || t.book.source_at_ms > t.created_at_ms + 2000
        || t.book.received_at_ms > t.created_at_ms
        || t.balance_at_ms > t.created_at_ms
        || t.fees_at_ms > t.created_at_ms
        || t.market.checked_at_ms > t.created_at_ms
        || p.updated_at_ms < t.created_at_ms
    {
        return Err("账户兑换身份、价格、金额、余额或时效不一致".into());
    }
    match &p.order {
        Some(o)
            if p.cancelled_at_ms.is_none()
                && p.revision >= 2
                && o.submitted_at_ms >= t.created_at_ms
                && o.submitted_at_ms < t.valid_until_ms
                && o.updated_at_ms <= p.updated_at_ms =>
        {
            order_protocol::validate(o, &t.instruction)
        }
        None if (p.revision == 1 && p.cancelled_at_ms.is_none())
            || (p.revision == 2
                && p.cancelled_at_ms
                    .is_some_and(|at| at >= t.created_at_ms && at <= p.updated_at_ms)) =>
        {
            Ok(())
        }
        _ => Err("账户兑换提交或取消状态不一致".into()),
    }
}
pub(super) fn valid_until(
    m: &StockConversionMarket,
    b: &StockBookQuote,
    balance: i64,
    fees: i64,
) -> i64 {
    [
        m.checked_at_ms.saturating_add(300_000),
        b.source_at_ms.saturating_add(10_000),
        b.received_at_ms.saturating_add(10_000),
        balance.saturating_add(30_000),
        fees.saturating_add(300_000),
    ]
    .into_iter()
    .min()
    .unwrap_or(0)
}
fn transition(
    a: &StockExchangeConversionPlan,
    b: &StockExchangeConversionPlan,
) -> Result<(), String> {
    validate(b)?;
    if a.plan_id != b.plan_id
        || a.request != b.request
        || a.terms != b.terms
        || a.cancelled_at_ms.is_some()
        || b.revision != a.revision + 1
        || b.updated_at_ms < a.updated_at_ms
        || !order_protocol::transition(a.order.as_ref(), b.order.as_ref())
    {
        return Err("账户兑换历史不允许改写计划、丢失成交或倒退".into());
    }
    Ok(())
}
fn append(path: &Path, p: &StockExchangeConversionPlan) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&Entry {
        version: 1,
        plan: p.clone(),
    })
    .map_err(|_| "账户兑换日志编码失败")?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_LINE {
        return Err("账户兑换记录超过行上限".into());
    }
    let mut f = super::super::plan_store::options(true)
        .open(path)
        .map_err(|_| "账户兑换日志无法打开")?;
    if f.metadata()
        .map_err(|_| "日志大小未知")?
        .len()
        .saturating_add(bytes.len() as u64)
        > MAX_FILE
    {
        return Err("账户兑换日志超过上限，请保留原日志".into());
    }
    f.write_all(&bytes)
        .and_then(|_| f.sync_all())
        .map_err(|_| "账户兑换写入结果不明，保留原日志")?;
    File::open(path.parent().ok_or("账户兑换目录无效")?)
        .and_then(|f| f.sync_all())
        .map_err(|_| "账户兑换目录同步失败".into())
}
fn read(
    path: &Path,
    rows: &mut BTreeMap<String, StockExchangeConversionPlan>,
) -> Result<(), String> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("账户兑换日志无法读取".into()),
    };
    if file.metadata().map_err(|_| "日志大小未知")?.len() > MAX_FILE {
        return Err("账户兑换日志超限".into());
    }
    let mut reader = BufReader::new(file);
    let mut bytes = vec![];
    loop {
        bytes.clear();
        let n = Read::by_ref(&mut reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|_| "账户兑换日志读取失败")?;
        if n == 0 {
            break;
        }
        if n as u64 > MAX_LINE || bytes.last() != Some(&b'\n') {
            return Err("账户兑换日志尾部不完整，请保留原文件".into());
        }
        let e: Entry = serde_json::from_slice(&bytes).map_err(|_| "账户兑换日志损坏")?;
        if e.version != 1 {
            return Err("账户兑换日志版本不支持".into());
        }
        validate(&e.plan)?;
        if let Some(old) = rows.get(&e.plan.request.request_id) {
            if old == &e.plan {
                continue;
            }
            transition(old, &e.plan)?;
        } else if e.plan.revision != 1 || e.plan.order.is_some() || e.plan.cancelled_at_ms.is_some()
        {
            return Err("账户兑换缺少初始记录".into());
        }
        rows.insert(e.plan.request.request_id.clone(), e.plan);
        if rows.len() > 256 {
            return Err("账户兑换历史超限".into());
        }
    }
    Ok(())
}
