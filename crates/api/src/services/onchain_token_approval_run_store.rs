use common::config::AppConfig;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use shared_types::{
    OnchainChainSettlementStatus as ReceiptStatus, OnchainExecutionToken,
    OnchainTokenApprovalBuildResponse as Plan, OnchainTokenApprovalRunStatus as Status,
    OnchainTokenApprovalSubmitResponse as Response, OnchainWalletReceipt,
    OnchainWalletReceiptBasis,
};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

const MAX_CHECKS: u8 = 12;
mod wallet_claims;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ApprovalRecord {
    version: u8,
    pub(crate) plan: Plan,
    pub(crate) response: Response,
    confirmed: Vec<String>,
    checks: BTreeMap<String, u8>,
    last_check_ms: i64,
}

#[derive(Debug)]
pub(crate) struct OnchainTokenApprovalRunStore {
    path: Option<PathBuf>,
    rows: Mutex<BTreeMap<String, ApprovalRecord>>,
    problem: Mutex<Option<String>>,
    wallet_claims: std::sync::Arc<super::onchain_wallet_claims::WalletClaims>,
}

impl OnchainTokenApprovalRunStore {
    pub(crate) fn load(config: &AppConfig) -> Self {
        // Keep the approval journal beside the configured cross-chain runtime journal.
        let path = config
            .storage
            .onchain_cross_chain_ledger_path
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map(|p| {
                config
                    .storage
                    .resolve_runtime_path(p)
                    .with_file_name("onchain_token_approval_runs.jsonl")
            });
        Self::load_path(path)
    }

    pub(crate) fn load_path(path: Option<PathBuf>) -> Self {
        let mut rows = BTreeMap::new();
        let result = (|| -> Result<(), String> {
            let Some(path) = &path else {
                return Ok(());
            };
            let file = match File::open(path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e.to_string()),
            };
            for line in BufReader::new(file).lines() {
                let line = line.map_err(|e| e.to_string())?;
                let row: ApprovalRecord = serde_json::from_str(&line)
                    .map_err(|e| format!("授权日志损坏，禁止新增提交：{e}"))?;
                validate(&row)?;
                if let Some(old) = rows.get(&row.response.run_id) {
                    validate_transition(old, &row)?;
                }
                if rows.values().any(|old: &ApprovalRecord| {
                    old.response.approval_id == row.response.approval_id
                        && old.response.run_id != row.response.run_id
                }) {
                    return Err("同一授权计划存在多个运行记录".into());
                }
                rows.insert(row.response.run_id.clone(), row);
            }
            Ok(())
        })();
        for row in rows.values_mut() {
            if row.response.status == Status::AwaitingFinality {
                row.response.status = Status::FinalityUnresolved;
                row.response.message =
                    "重启后仅核验已记录的授权交易，未广播的后续步骤不会自动提交".into();
            }
        }
        let store=Self {
            path,
            rows: Mutex::new(rows),
            problem: Mutex::new(result.err()),
            wallet_claims:Default::default(),
        };
        store.restore_wallet_claims();
        store
    }

    pub(crate) fn readiness(&self) -> Result<(), String> {
        if let Some(problem) = self.problem.lock().clone() {
            return Err(problem);
        }
        if self.path.is_none() {
            return Err("实盘授权需要配置持久化运行日志".into());
        }
        Ok(())
    }

    pub(crate) fn record(&self, id: &str) -> Option<ApprovalRecord> {
        self.rows.lock().get(id).cloned()
    }
    pub(crate) fn by_approval(&self, id: &str) -> Option<Response> {
        self.rows
            .lock()
            .values()
            .find(|r| r.response.approval_id == id)
            .map(|r| r.response.clone())
    }
    pub(crate) fn recent(&self, limit: usize) -> Vec<Response> {
        let mut rows = self
            .rows
            .lock()
            .values()
            .map(|r| r.response.clone())
            .collect::<Vec<_>>();
        rows.sort_by_key(|r| std::cmp::Reverse(r.updated_at_ms));
        rows.truncate(limit.clamp(1, 64));
        if let Some(problem) = self.problem.lock().clone() {
            for row in &mut rows {
                row.problem = Some(problem.clone());
                row.message = "授权运行日志异常，已停止新增提交和后台核验".into();
                row.fee_checks_exhausted = true;
                if row.status == Status::AwaitingFinality {
                    row.status = Status::FinalityUnresolved;
                }
            }
        }
        rows
    }

    pub(crate) fn create(&self, plan: Plan, response: Response) -> Result<Response, String> {
        self.readiness()?;
        let mut rows = self.rows.lock();
        if rows
            .get(&response.run_id)
            .is_some_and(|r| r.plan.approval_id != plan.approval_id)
        {
            return Err("授权运行编号冲突".into());
        }
        if let Some(old) = rows
            .values()
            .find(|r| r.response.approval_id == plan.approval_id)
        {
            return Ok(old.response.clone());
        }
        let row = ApprovalRecord {
            version: 1,
            plan,
            response,
            confirmed: vec![],
            checks: BTreeMap::new(),
            last_check_ms: 0,
        };
        validate(&row)?;
        self.append(&row)?;
        let response = row.response.clone();
        rows.insert(response.run_id.clone(), row);
        Ok(response)
    }

    fn update(
        &self,
        id: &str,
        f: impl FnOnce(&mut ApprovalRecord) -> Result<(), String>,
    ) -> Result<Response, String> {
        self.readiness()?;
        let mut rows = self.rows.lock();
        let old = rows.get(id).ok_or("授权运行记录不存在")?;
        let mut row = old.clone();
        f(&mut row)?;
        validate(&row)?;
        validate_transition(old, &row)?;
        if row == *old {
            return Ok(row.response);
        }
        self.append(&row)?;
        let response = row.response.clone();
        rows.insert(id.into(), row);
        Ok(response)
    }

    pub(crate) fn intent(
        &self,
        id: &str,
        position: usize,
        hash: &str,
        now: i64,
    ) -> Result<Response, String> {
        self.update(id, |r| {
            if r.response.transaction_ids.get(position).is_some() {
                return Err("授权交易已登记，禁止再次广播".into());
            }
            if r.response.transaction_ids.len() != position
                || position >= r.plan.transactions.len()
                || r.confirmed.len() != position
                || r.response.status != Status::AwaitingFinality
            {
                return Err("授权步骤已提交或前一步未确认，禁止重复广播".into());
            }
            r.response.transaction_ids.push(hash.into());
            r.response.updated_at_ms = now;
            Ok(())
        })
    }

    pub(crate) fn confirm(&self, id: &str, hash: &str) -> Result<(), String> {
        self.update(id, |r| {
            if !r.response.transaction_ids.iter().any(|v| v == hash) {
                return Err("未知授权交易终态".into());
            }
            if !r.confirmed.iter().any(|v| v == hash) {
                r.confirmed.push(hash.into());
            }
            Ok(())
        })
        .map(|_| ())
    }

    pub(crate) fn response(&self, response: Response) -> Result<Response, String> {
        self.update(&response.run_id, |r| {
            if response.transaction_ids != r.response.transaction_ids {
                return Err("授权反馈交易编号与广播前日志不一致".into());
            }
            if matches!(r.response.status, Status::Completed | Status::Failed)
                && matches!(
                    response.status,
                    Status::AwaitingFinality | Status::FinalityUnresolved
                )
            {
                return Ok(());
            }
            let receipts = r.response.fee_receipts.clone();
            let exhausted = r.response.fee_checks_exhausted;
            r.response = response.clone();
            r.response.fee_receipts = receipts;
            r.response.fee_checks_exhausted = exhausted;
            Ok(())
        })
    }

    pub(crate) fn due(&self, now: i64) -> Vec<(ApprovalRecord, String)> {
        let mut rows = self.rows.lock().values().cloned().collect::<Vec<_>>();
        rows.sort_by_key(|r| r.last_check_ms);
        rows.into_iter()
            .filter(|r| now.saturating_sub(r.last_check_ms) >= 5_000)
            .filter_map(|r| {
                let hash = r
                    .response
                    .transaction_ids
                    .iter()
                    .find(|hash| {
                        r.checks.get(*hash).copied().unwrap_or(0) < MAX_CHECKS
                            && !r
                                .response
                                .fee_receipts
                                .iter()
                                .any(|v| &v.basis.transaction_id == *hash && complete_cost(v))
                    })
                    .cloned()?;
                Some((r, hash))
            })
            .take(2)
            .collect()
    }

    pub(crate) fn receipt(
        &self,
        id: &str,
        receipt: OnchainWalletReceipt,
        now: i64,
    ) -> Result<Response, String> {
        self.update(id, |r| {
            if basis(r, &receipt.basis.transaction_id)? != receipt.basis
                || receipt.asset_changes_raw.len() != 1
            {
                return Err("授权费用回执与原交易身份不符".into());
            }
            if r.response.fee_receipts.iter().any(|old| {
                old.basis.transaction_id == receipt.basis.transaction_id && complete_cost(old)
            }) {
                return Ok(());
            }
            let count = r
                .checks
                .entry(receipt.basis.transaction_id.clone())
                .or_default();
            *count = count.saturating_add(1);
            r.last_check_ms = now;
            if let Some(old) = r
                .response
                .fee_receipts
                .iter_mut()
                .find(|v| v.basis.transaction_id == receipt.basis.transaction_id)
            {
                if receipt.block_ref.is_some() || old.block_ref.is_none() {
                    *old = receipt.clone();
                }
            } else {
                r.response.fee_receipts.push(receipt.clone());
            }
            r.response.fee_checks_exhausted = r.response.transaction_ids.iter().any(|hash| {
                r.checks.get(hash).copied().unwrap_or(0) >= MAX_CHECKS
                    && !r
                        .response
                        .fee_receipts
                        .iter()
                        .any(|v| v.basis.transaction_id == *hash && complete_cost(v))
            });
            if receipt.status == ReceiptStatus::ReviewRequired && complete_cost(&receipt) {
                r.response.status = Status::Failed;
                r.response.message =
                    "授权未被核验为成功，已保留实际链费与交易记录；未继续后续步骤".into();
                r.response.problem = receipt.problem.clone();
            }
            if r.response.fee_checks_exhausted && r.response.status == Status::AwaitingFinality {
                r.response.status = Status::FinalityUnresolved;
                r.response.message =
                    "本轮授权回执核验已暂停，原交易仍保留；可手动重新核验，不会重复广播".into();
            }
            if receipt.status == ReceiptStatus::Complete
                && !r.confirmed.contains(&receipt.basis.transaction_id)
            {
                r.confirmed.push(receipt.basis.transaction_id.clone());
            }
            if r.response.status == Status::FinalityUnresolved
                && r.response.transaction_ids.len() == r.plan.transactions.len()
                && r.response.transaction_ids.iter().all(|id| {
                    r.response.fee_receipts.iter().any(|v| {
                        v.basis.transaction_id == *id && v.status == ReceiptStatus::Complete
                    })
                })
            {
                r.response.status = Status::Completed;
                r.response.message = "原授权交易已核验，未重复广播".into();
                r.response.problem = None;
            }
            r.response.updated_at_ms = now;
            Ok(())
        })
    }

    pub(crate) fn resume_checks(&self, id: &str) -> Result<Response, String> {
        self.update(id, |r| {
            if !r.response.fee_checks_exhausted {
                return Ok(());
            }
            r.checks.clear();
            r.last_check_ms = 0;
            r.response.fee_checks_exhausted = false;
            r.response.message = "仅重新读取原授权交易回执，不重新广播".into();
            r.response.updated_at_ms = common::time::now_ms();
            Ok(())
        })
    }

    fn append(&self, row: &ApprovalRecord) -> Result<(), String> {
        use super::onchain_wallet_claims::{Module, Owner};

        let hold = wallet_claims::hold(row)?;
        self.wallet_claims.commit(
            Owner::new(Module::Approval, &row.response.run_id),
            hold,
            row.response.updated_at_ms,
            || {
                let result = (|| -> std::io::Result<()> {
                    let path = self
                        .path
                        .as_deref()
                        .ok_or_else(|| std::io::Error::other("授权日志未配置"))?;
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut bytes = serde_json::to_vec(row).map_err(std::io::Error::other)?;
                    bytes.push(b'\n');
                    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                    file.write_all(&bytes)?;
                    file.flush()?;
                    file.sync_data()?;
                    if let Some(parent) = path.parent() {
                        File::open(parent)?.sync_all()?;
                    }
                    Ok(())
                })();
                result.map_err(|e| {
                    let problem = format!("授权记录未持久化，禁止继续广播：{e}");
                    *self.problem.lock() = Some(problem.clone());
                    problem
                })
            },
        )
    }
}

pub(crate) fn basis(row: &ApprovalRecord, hash: &str) -> Result<OnchainWalletReceiptBasis, String> {
    if !row.response.transaction_ids.iter().any(|v| v == hash) {
        return Err("授权交易未登记".into());
    }
    Ok(OnchainWalletReceiptBasis {
        chain: row.plan.chain.clone(),
        wallet: row.plan.wallet_address.clone(),
        transaction_id: hash.into(),
        require_sender: true,
        assets: vec![OnchainExecutionToken {
            symbol: row.plan.token_symbol.clone(),
            address: row.plan.token_address.clone(),
            decimals: row.plan.token_decimals,
        }],
    })
}

pub(crate) fn complete_cost(receipt: &OnchainWalletReceipt) -> bool {
    receipt.status != ReceiptStatus::Pending
        && receipt.block_ref.is_some()
        && receipt.network_cost.as_ref().is_some_and(|c| {
            let amount = |v: &Option<String>| {
                v.as_ref()
                    .and_then(|v| v.parse::<rust_decimal::Decimal>().ok())
                    .filter(|v| *v >= rust_decimal::Decimal::ZERO)
            };
            c.chain == receipt.basis.chain
                && c.transaction_id
                    .eq_ignore_ascii_case(&receipt.basis.transaction_id)
                && c.payer.eq_ignore_ascii_case(&receipt.basis.wallet)
                && Some(c.block_ref.as_str()) == receipt.block_ref.as_deref()
                && shared_types::onchain_chain_preset(&c.chain)
                    .is_some_and(|p| p.base_token.eq_ignore_ascii_case(&c.asset))
                && c.problem.is_none()
                && amount(&c.total_fee_exact).is_some()
                && amount(&c.execution_fee_exact)
                    .zip(amount(&c.additional_fee_exact))
                    .and_then(|(a, b)| a.checked_add(b))
                    == amount(&c.total_fee_exact)
        })
        && receipt.additional_native_change_raw.is_some()
        && receipt.asset_changes_raw.iter().all(Option::is_some)
}

fn validate(row: &ApprovalRecord) -> Result<(), String> {
    if row.version != 1
        || row.response.run_id.is_empty()
        || row.response.approval_id != row.plan.approval_id
        || row.plan.transactions.is_empty()
        || row.plan.transactions.len() > 2
        || row.response.transaction_ids.len() > row.plan.transactions.len()
        || row
            .response
            .transaction_ids
            .iter()
            .enumerate()
            .any(|(i, id)| {
                id.len() != 66
                    || !id.starts_with("0x")
                    || !id[2..].bytes().all(|b| b.is_ascii_hexdigit())
                    || row.response.transaction_ids[..i].contains(id)
            })
        || row
            .confirmed
            .iter()
            .any(|id| !row.response.transaction_ids.contains(id))
    {
        return Err("授权日志的计划、步骤或交易编号无效".into());
    }
    for index in 0..row.plan.transactions.len() {
        approval_words(&row.plan, index)?;
    }
    if row.response.status == Status::Completed
        && (row.response.transaction_ids.len() != row.plan.transactions.len()
            || row.confirmed.len() != row.plan.transactions.len())
    {
        return Err("授权未逐笔确认，不能写入完成状态".into());
    }
    for receipt in &row.response.fee_receipts {
        if basis(row, &receipt.basis.transaction_id)? != receipt.basis {
            return Err("授权回执身份不一致".into());
        }
    }
    Ok(())
}

pub(crate) fn approval_words(
    plan: &Plan,
    position: usize,
) -> Result<(String, String, String), String> {
    let Some(shared_types::OnchainUnsignedTransaction::EvmCall {
        chain_id,
        from,
        to,
        data,
        value,
        ..
    }) = plan.transactions.get(position)
    else {
        return Err("只支持已编译的 ERC-20 授权交易".into());
    };
    let address = |a: &str| {
        a.len() == 42 && a.starts_with("0x") && a[2..].bytes().all(|v| v.is_ascii_hexdigit())
    };
    if !address(&plan.wallet_address)
        || !address(&plan.spender)
        || !address(&plan.token_address)
        || !from.eq_ignore_ascii_case(&plan.wallet_address)
        || !to.eq_ignore_ascii_case(&plan.token_address)
        || shared_types::onchain_chain_preset(&plan.chain).and_then(|p| p.chain_id)
            != Some(*chain_id)
        || !matches!(value.as_str(), "0" | "0x0" | "0x00")
        || data.len() != 138
        || !data.starts_with("0x095ea7b3")
        || !data[2..].bytes().all(|v| v.is_ascii_hexdigit())
    {
        return Err("授权交易的链、付款方、代币或调用内容与计划不一致".into());
    }
    let owner = format!("0x{:0>64}", &plan.wallet_address[2..]);
    let spender = format!("0x{:0>64}", &plan.spender[2..]);
    if !data[10..74].eq_ignore_ascii_case(&spender[2..]) {
        return Err("授权 spender 与计划不一致".into());
    }
    Ok((owner, spender, format!("0x{}", &data[74..])))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn test_plan() -> Plan {
    tests::plan()
}

fn validate_transition(old: &ApprovalRecord, row: &ApprovalRecord) -> Result<(), String> {
    if old.plan != row.plan
        || old.response.run_id != row.response.run_id
        || !row
            .response
            .transaction_ids
            .starts_with(&old.response.transaction_ids)
        || old.confirmed.iter().any(|id| !row.confirmed.contains(id))
    {
        return Err("授权持久化身份或已提交记录不能被覆盖".into());
    }
    Ok(())
}
