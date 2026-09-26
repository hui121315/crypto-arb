use super::*;

const PAGE_SIZE: u32 = 100;
const REQUESTS_PER_CHECK: usize = 4;

fn start(transfer: &StockFundingTransfer, now: i64) -> Result<StockFundingDepositScan, String> {
    Ok(StockFundingDepositScan {
        from_ms: transfer
            .submitted_at_ms
            .ok_or("原提交时间未知")?
            .saturating_sub(5000),
        to_ms: now,
        scanned_rows: 0,
        checkpoint: None,
        matched: false,
        completed_at_ms: None,
    })
}

pub(super) fn validate(plan: &StockFundingPlan) -> Result<(), String> {
    let t = plan.transfer.as_ref().ok_or("原转账记录缺失")?;
    let Some(s) = &t.deposit_scan else {
        return Ok(());
    };
    let at = t.submitted_at_ms.ok_or("分页缺少原提交")?;
    let digest_valid = s
        .checkpoint
        .as_ref()
        .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    let pending = s.completed_at_ms.is_none();
    if !t
        .receipt
        .as_ref()
        .is_some_and(|r| r.within_plan && r.succeeded)
        || t.query_count == 0
        || t.last_query_at_ms.is_none()
        || s.from_ms != at.saturating_sub(5000)
        || s.to_ms < at
        || s.to_ms > plan.updated_at_ms
        || s.completed_at_ms
            .is_some_and(|at| at < s.to_ms || at > plan.updated_at_ms)
        || pending && s.scanned_rows % PAGE_SIZE != 0
        || pending && s.scanned_rows == 0 && (s.checkpoint.is_some() || s.matched)
        || (s.scanned_rows > 0 || !pending) && !digest_valid
        || s.matched && (s.scanned_rows == 0 || t.deposit.is_none())
    {
        return Err("原入账分页范围、进度或终态不一致，保留占用".into());
    }
    Ok(())
}

pub(super) fn transition(old: &StockFundingPlan, new: &StockFundingPlan) -> Result<(), String> {
    let a = old.transfer.as_ref().and_then(|t| t.deposit_scan.as_ref());
    let b = new.transfer.as_ref().and_then(|t| t.deposit_scan.as_ref());
    if a == b {
        return Ok(());
    }
    if !old.phase.holds_funds() {
        return Err("已结束的原转账不能重新启动入账查询或占用".into());
    }
    let b = b.ok_or("原入账分页记录不能删除")?;
    let beginning =
        b.scanned_rows == 0 && b.checkpoint.is_none() && !b.matched && b.completed_at_ms.is_none();
    if beginning {
        return if b.to_ms == new.updated_at_ms && a.is_none_or(|a| b.to_ms >= a.to_ms) {
            Ok(())
        } else {
            Err("入账续查不能改写已冻结的历史范围".into())
        };
    }
    let a = a.ok_or("入账续查必须先持久化查询范围")?;
    let rows = b
        .scanned_rows
        .checked_sub(a.scanned_rows)
        .ok_or("入账分页不能跳回或跳过历史")?;
    if a.completed_at_ms.is_some()
        || a.from_ms != b.from_ms
        || a.to_ms != b.to_ms
        || a.matched && !b.matched
        || match b.completed_at_ms {
            None => rows != PAGE_SIZE,
            Some(at) => rows >= PAGE_SIZE || at != new.updated_at_ms,
        }
    {
        return Err("入账分页只能逐页推进或记录已查完，不能跳页放行".into());
    }
    Ok(())
}

fn fingerprint(rows: &Value) -> Result<String, ReadProblem> {
    if rows.as_array().is_none_or(|r| r.len() > PAGE_SIZE as usize) {
        return Err(ReadProblem::Unavailable(
            "Backpack 入账历史响应无效或过大".into(),
        ));
    }
    let bytes = serde_json::to_vec(rows)
        .map_err(|_| ReadProblem::Unavailable("Backpack 入账历史无法编码".into()))?;
    Ok(common::signing::hmac_sha256_hex(b"stock-deposit-page-v1", &bytes))
}

fn apply_page(
    plan: &StockFundingPlan,
    rows: &Value,
    now: i64,
) -> Result<StockFundingTransfer, ReadProblem> {
    let digest = fingerprint(rows)?;
    let mut t = plan.transfer.clone().unwrap();
    let mut scan = t.deposit_scan.clone().unwrap();
    let deposit = deposit_from_rows(plan, rows)?;
    if let Some(deposit) = deposit {
        let at = chrono::DateTime::parse_from_rfc3339(&deposit.created_at)
            .map_err(|_| ReadProblem::Unavailable("入账时间无法核对".into()))?
            .timestamp_millis();
        if at < scan.from_ms || at > scan.to_ms {
            return Err(ReadProblem::Unavailable(
                "入账返回超出本轮查询时间范围，保留进度与占用".into(),
            ));
        }
        if scan.matched {
            // The same id across offset pages can be page movement, not a second deposit.
            // Different ids/identities were already rejected by merge_deposit.
            t.deposit = Some(deposit);
            t.deposit_scan = Some(start(&t, now).map_err(ReadProblem::Unavailable)?);
            t.problem = Some("入账历史跨页重现同一记录，已保留原凭据并重新核验；未释放占用".into());
            return Ok(t);
        }
        if deposit.status == "confirmed"
            && decimal(&deposit.quantity).map_err(ReadProblem::Unavailable)?
                != decimal(&plan.terms.quantity).map_err(ReadProblem::Unavailable)?
        {
            ReadProblem::Conflict(
                "Backpack 已确认数量与原计划不一致，实际入账已保存，保留占用".into(),
            )
            .record(&mut t);
        }
        t.deposit = Some(deposit);
        scan.matched = true;
    }
    let count = rows.as_array().unwrap().len() as u32;
    scan.scanned_rows = scan
        .scanned_rows
        .checked_add(count)
        .ok_or_else(|| ReadProblem::Unavailable("入账历史进度溢出，保留占用".into()))?;
    scan.checkpoint = Some(digest);
    if count < PAGE_SIZE {
        scan.completed_at_ms = Some(now);
    }
    t.problem = Some(if t.evidence_conflict.is_some() {
        "已保留原交易查询结果；此前回执冲突仍需人工核对，未释放占用".into()
    } else if scan.completed_at_ms.is_none() {
        format!(
            "已核验 {} 条入账历史，进度已保存；继续核对原交易，未释放占用",
            scan.scanned_rows
        )
    } else if !scan.matched {
        "本轮入账历史已查完，未找到原交易；后续扩大到最新时间继续查，不会重新转账".into()
    } else if t.deposit.as_ref().is_some_and(|d| d.status == "confirmed") {
        "Backpack 已确认原转账入账；实际转出、到账与 SOL 网络费已记录，下一笔交易仍须重新预检"
            .into()
    } else {
        "已找到原交易入账记录，交易所尚未确认；继续保留占用".into()
    });
    t.deposit_scan = Some(scan);
    Ok(t)
}

impl BackpackStocks {
    async fn funding_deposit_page(
        &self,
        keys: &credentials::Credentials,
        scan: &StockFundingDepositScan,
        offset: u32,
    ) -> Result<Value, ReadProblem> {
        let bytes = self.signed_rfq_request(keys, reqwest::Method::GET,
            "/wapi/v1/capital/deposits", "depositQueryAll",
            &json!({"from":scan.from_ms,"to":scan.to_ms,"limit":PAGE_SIZE,"offset":offset,"excludePlatform":true}),
        ).await.map_err(|_| ReadProblem::Unavailable("Backpack 原入账历史查询失败，进度和占用已保留；没有转账重试".into()))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| ReadProblem::Unavailable("Backpack 入账历史无法解析".into()))
    }

    fn save_deposit_read_problem(
        &self,
        plan: &StockFundingPlan,
        problem: ReadProblem,
    ) -> Result<(), String> {
        let mut t = plan.transfer.clone().unwrap();
        problem.record(&mut t);
        self.funding_store
            .update_transfer(plan, t, common::time::now_ms())?;
        Ok(())
    }

    pub(super) async fn scan_funding_deposit(
        &self,
        plan: &StockFundingPlan,
        keys: &credentials::Credentials,
    ) -> Result<(), String> {
        let mut current = plan.clone();
        let mut t = current.transfer.clone().unwrap();
        if t.deposit_scan
            .as_ref()
            .is_none_or(|s| s.completed_at_ms.is_some())
        {
            let now = common::time::now_ms();
            t.deposit_scan = Some(start(&t, now)?);
            current = self.funding_store.update_transfer(&current, t, now)?;
        }
        let scan = current
            .transfer
            .as_ref()
            .unwrap()
            .deposit_scan
            .clone()
            .unwrap();
        let mut budget = REQUESTS_PER_CHECK;
        if scan.scanned_rows > 0 {
            budget -= 1;
            let checked = self
                .funding_deposit_page(keys, &scan, scan.scanned_rows - PAGE_SIZE)
                .await
                .and_then(|rows| {
                    deposit_from_rows(&current, &rows)?;
                    fingerprint(&rows)
                });
            match checked {
                Ok(digest) if Some(&digest) == scan.checkpoint.as_ref() => {}
                Ok(_) => {
                    let now = common::time::now_ms();
                    let mut t = current.transfer.clone().unwrap();
                    t.deposit_scan = Some(start(&t, now)?);
                    t.problem =
                        Some("入账历史上一页已变化，原凭据保留并从头重新核验；未释放占用".into());
                    self.funding_store.update_transfer(&current, t, now)?;
                    return Ok(());
                }
                Err(problem) => return self.save_deposit_read_problem(&current, problem),
            }
        }
        for _ in 0..budget {
            let scan = current
                .transfer
                .as_ref()
                .unwrap()
                .deposit_scan
                .as_ref()
                .unwrap();
            let rows = match self
                .funding_deposit_page(keys, scan, scan.scanned_rows)
                .await
            {
                Ok(rows) => rows,
                Err(problem) => return self.save_deposit_read_problem(&current, problem),
            };
            let now = common::time::now_ms();
            let mut observed = current.clone();
            observed.updated_at_ms = now;
            let t = match apply_page(&observed, &rows, now) {
                Ok(t) => t,
                Err(problem) => return self.save_deposit_read_problem(&current, problem),
            };
            let s = t.deposit_scan.as_ref().unwrap();
            let stop =
                t.evidence_conflict.is_some() || s.completed_at_ms.is_some() || s.scanned_rows == 0;
            current = self.record_funding_observation(&current, t, now)?;
            if stop {
                break;
            }
        }
        Ok(())
    }
}
