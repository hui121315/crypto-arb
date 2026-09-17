use fs2::FileExt;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use shared_types::stocks::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u8,
    record: StockRfq,
}
#[derive(Default)]
struct Inner {
    rows: BTreeMap<String, StockRfq>,
    problem: Option<String>,
    writer: Option<File>,
    recent: Vec<String>,
    pending: BTreeSet<String>,
}
pub(super) struct RfqStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl RfqStore {
    pub(super) fn load(path: Option<PathBuf>) -> Self {
        let (rows, problem) = match path.as_deref().map(read_rows).transpose() {
            Ok(rows) => (rows.unwrap_or_default(), None),
            Err(_) => (
                BTreeMap::new(),
                Some("股票 RFQ 日志读取失败；禁止新请求，请保留原文件核查".into()),
            ),
        };
        let mut inner = Inner {
            rows,
            problem,
            ..Default::default()
        };
        invalidate(&mut inner);
        reindex(&mut inner);
        Self {
            path,
            inner: Mutex::new(inner),
        }
    }
    pub(super) fn problem(&self) -> Option<String> {
        self.inner.lock().problem.clone()
    }
    pub(super) fn records(&self) -> Vec<StockRfq> {
        let inner = self.inner.lock();
        inner
            .pending
            .iter()
            .chain(
                inner
                    .recent
                    .iter()
                    .filter(|id| !inner.pending.contains(*id)),
            )
            .take(48)
            .filter_map(|id| inner.rows.get(id).cloned())
            .collect()
    }
    pub(super) fn get(&self, id: &str) -> Option<StockRfq> {
        self.inner.lock().rows.get(id).cloned()
    }

    pub(super) fn finish_unsent(&self, request: StockRfqRequest, now: i64) -> Result<StockRfq, String> {
        if !valid_id(&request.request_id) {
            return Err("RFQ 请求标识无效".into());
        }
        let mut inner = self.inner.lock();
        self.writer(&mut inner)?;
        if let Some(old) = inner.rows.get(&request.request_id) {
            return if old.request == request { Ok(old.clone()) } else { Err("RFQ 请求参数冲突".into()) };
        }
        // No exchange identity exists. This durable terminal row fences any delayed POST.
        let record = StockRfq {
            request, client_id: 0, account_fingerprint: String::new(), symbol: String::new(),
            rfq_id: None, phase: StockRfqPhase::NotSent, candidate: None,
            submission_time_ms: None, expiry_time_ms: None, source_at_us: None,
            fill_price: None, executed_quantity: None, executed_quote_quantity: None,
            fills: vec![], settlement: Default::default(), acceptance: None,
            needs_recheck: false, cancel_requested: false, created_at_ms: now, updated_at_ms: now,
            problem: Some("该编号已在本地结束，未向交易所发送；迟到请求不会再发送".into()),
        };
        self.append(&mut inner, &record)?;
        inner.recent.insert(0, record.request.request_id.clone());
        inner.recent.truncate(48);
        inner.rows.insert(record.request.request_id.clone(), record.clone());
        Ok(record)
    }

    #[cfg(test)]
    pub(super) fn claim(
        &self,
        request: StockRfqRequest,
        fingerprint: &str,
        symbol: String,
        now: i64,
    ) -> Result<(StockRfq, bool), String> {
        self.claim_with_receipts(request, fingerprint, symbol, now, &[])
    }

    pub(super) fn claim_with_receipts(
        &self,
        request: StockRfqRequest,
        fingerprint: &str,
        symbol: String,
        now: i64,
        receipts: &[StockRfq],
    ) -> Result<(StockRfq, bool), String> {
        if !valid_id(&request.request_id) {
            return Err("RFQ 请求标识无效".into());
        }
        let mut inner = self.inner.lock();
        self.writer(&mut inner)?;
        if let Some(previous) = inner.rows.get(&request.request_id) {
            if previous.request != request
                || previous.account_fingerprint != fingerprint
                || previous.symbol != symbol
            {
                return Err("相同 RFQ 请求标识对应不同参数或账户".into());
            }
            return Ok((previous.clone(), true));
        }
        let pending = inner
            .rows
            .values()
            .filter(|r| {
                !receipts
                    .iter()
                    .any(|a| a.request.request_id == r.request.request_id)
            })
            .chain(receipts.iter())
            .filter(|r| r.unresolved());
        if pending.clone().count() >= 16 {
            return Err("未结 RFQ 已达 16 条，请先核对或取消旧询价".into());
        }
        if pending.clone().any(|r| {
            r.account_fingerprint == fingerprint
                && r.request.asset == request.asset
                && r.request.side == request.side
        }) {
            return Err("该股票方向仍有未结询价；请核对或取消原请求后再询价".into());
        }
        let client_id = (0..16)
            .map(|_| uuid::Uuid::new_v4().as_u128() as u32)
            .find(|id| {
                *id != 0
                    && !inner
                        .rows
                        .values()
                        .chain(receipts.iter())
                        .any(|r| r.client_id == *id)
            })
            .ok_or("RFQ clientId 分配失败")?;
        let record = StockRfq {
            request,
            client_id,
            account_fingerprint: fingerprint.into(),
            symbol,
            rfq_id: None,
            phase: StockRfqPhase::SubmissionUnknown,
            candidate: None,
            submission_time_ms: None,
            expiry_time_ms: None,
            source_at_us: None,
            fill_price: None,
            executed_quantity: None,
            executed_quote_quantity: None,
            fills: vec![],
            settlement: Default::default(),
            acceptance: None,
            needs_recheck: true,
            cancel_requested: false,
            created_at_ms: now,
            updated_at_ms: now,
            problem: Some("询价提交中；未明确确认前不会重发".into()),
        };
        self.append(&mut inner, &record)?;
        inner.recent.insert(0, record.request.request_id.clone());
        inner.recent.truncate(48);
        inner.pending.insert(record.request.request_id.clone());
        inner
            .rows
            .insert(record.request.request_id.clone(), record.clone());
        Ok((record, false))
    }
    pub(super) fn change(
        &self,
        id: &str,
        durable: bool,
        change: impl FnOnce(&mut StockRfq) -> Result<bool, String>,
    ) -> Result<Option<StockRfq>, String> {
        let mut inner = self.inner.lock();
        if durable {
            self.writer(&mut inner)?;
        }
        let mut record = inner.rows.get(id).cloned().ok_or("RFQ 请求不存在")?;
        if !change(&mut record)? {
            return Ok(None);
        }
        if durable {
            self.append(&mut inner, &record)?;
        }
        if record.unresolved() {
            inner.pending.insert(id.into());
        } else {
            inner.pending.remove(id);
        }
        inner.rows.insert(id.into(), record.clone());
        Ok(Some(record))
    }
    pub(super) fn disconnect(&self) {
        invalidate(&mut self.inner.lock());
    }

    fn writer(&self, inner: &mut Inner) -> Result<(), String> {
        if let Some(problem) = &inner.problem {
            return Err(problem.clone());
        }
        if inner.writer.is_some() {
            return Ok(());
        }
        let path = self
            .path
            .as_deref()
            .ok_or("股票 RFQ 持久化未配置，禁止发送新询价")?;
        let result = (|| -> std::io::Result<File> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut options = OpenOptions::new();
            options.create(true).read(true).write(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options.open(path.with_extension("lock"))?;
            file.try_lock_exclusive()?;
            let rows = read_rows(path)?;
            inner.rows = rows;
            invalidate(inner);
            reindex(inner);
            Ok(file)
        })();
        match result {
            Ok(file) => {
                inner.writer = Some(file);
                Ok(())
            }
            Err(_) => {
                let problem = "股票 RFQ 日志被占用或不可写；未发送请求".to_owned();
                inner.problem = Some(problem.clone());
                Err(problem)
            }
        }
    }
    fn append(&self, inner: &mut Inner, record: &StockRfq) -> Result<(), String> {
        let path = self.path.as_deref().ok_or("RFQ 持久化未配置")?;
        let result = (|| -> std::io::Result<()> {
            let existed = path.exists();
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path)?;
            let mut row = serde_json::to_vec(&Entry {
                version: 1,
                record: record.clone(),
            })?;
            row.push(b'\n');
            file.write_all(&row)?;
            file.sync_data()?;
            if !existed {
                if let Some(parent) = path.parent() {
                    File::open(parent)?.sync_all()?;
                }
            }
            Ok(())
        })();
        result.map_err(|_| {
            let p = "RFQ 日志写入失败；保留原请求，禁止重复提交".to_owned();
            inner.problem = Some(p.clone());
            p
        })
    }
}

fn invalidate(inner: &mut Inner) {
    for record in inner.rows.values_mut().filter(|r| !r.phase.terminal()) {
        record.needs_recheck = true;
        record.candidate = None;
        record.problem = Some("私有 WS 尚未核对；旧报价不可接受，请核对原 RFQ".into());
    }
}

fn reindex(inner: &mut Inner) {
    let mut rows = inner.rows.values().collect::<Vec<_>>();
    rows.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
    inner.recent = rows
        .iter()
        .take(48)
        .map(|r| r.request.request_id.clone())
        .collect();
    inner.pending = rows
        .iter()
        .filter(|r| r.unresolved())
        .map(|r| r.request.request_id.clone())
        .collect();
}

fn read_rows(path: &Path) -> std::io::Result<BTreeMap<String, StockRfq>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e),
    };
    if file.metadata()?.len() > 64 * 1024 * 1024 {
        return Err(std::io::Error::other("RFQ journal exceeds read budget"));
    }
    let mut rows: BTreeMap<String, StockRfq> = BTreeMap::new();
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = std::io::Read::by_ref(&mut reader)
            .take(65_537)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if n > 65_536 || line.last() != Some(&b'\n') {
            return Err(std::io::Error::other("incomplete RFQ journal row"));
        }
        let entry: Entry = serde_json::from_slice(&line)?;
        if entry.version != 1
            || !valid_id(&entry.record.request.request_id)
            || entry.record.acceptance.is_some()
        {
            return Err(std::io::Error::other("invalid RFQ journal schema"));
        }
        if let Some(old) = rows.get(&entry.record.request.request_id) {
            if old.request != entry.record.request
                || old.client_id != entry.record.client_id
                || old.account_fingerprint != entry.record.account_fingerprint
                || old.symbol != entry.record.symbol
                || old
                    .rfq_id
                    .as_ref()
                    .is_some_and(|id| entry.record.rfq_id.as_ref() != Some(id))
            {
                return Err(std::io::Error::other("RFQ journal identity changed"));
            }
        }
        rows.insert(entry.record.request.request_id.clone(), entry.record);
    }
    Ok(rows)
}

fn valid_id(id: &str) -> bool {
    (16..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
