use super::*;
use shared_types::stocks::comparison::positive;

impl BackpackStocks {
    pub(super) fn visible_rfqs(&self) -> Vec<StockRfq> {
        let mut records = self.rfq_records();
        records.sort_by_key(|r| (!r.unresolved(), std::cmp::Reverse(r.created_at_ms)));
        records.truncate(48);
        records
    }

    pub(super) fn stock_rfq(&self, id: &str) -> Option<StockRfq> {
        self.plan_store
            .accepted_rfq(id)
            .or_else(|| self.rfq_store.get(id))
    }

    pub(super) fn rfq_records(&self) -> Vec<StockRfq> {
        let mut rows: std::collections::BTreeMap<_, _> = self
            .rfq_store
            .records()
            .into_iter()
            .map(|r| (r.request.request_id.clone(), r))
            .collect();
        // An accepted RFQ lives in the same durable journal as its wallet hold.
        // The earlier inquiry log is never allowed to overwrite that receipt.
        for r in self.plan_store.accepted_rfqs() {
            rows.insert(r.request.request_id.clone(), r);
        }
        rows.into_values().collect()
    }

    pub(super) fn change_rfq(
        &self,
        id: &str,
        durable: bool,
        apply: impl FnOnce(&mut StockRfq) -> Result<bool, String>,
    ) -> Result<Option<StockRfq>, String> {
        let _state = self.rfq_state_lock.lock();
        if let Some(record) = self.plan_store.accepted_rfq(id) {
            let plan_id = &record
                .acceptance
                .as_ref()
                .ok_or("接受记录缺少计划编号")?
                .plan_id;
            self.plan_store.change_rfq(plan_id, apply)
        } else {
            if let Some(problem) = self.plan_store.problem() {
                return Err(problem);
            }
            self.rfq_store.change(id, durable, apply)
        }
    }

    pub(super) fn record_rfq_receipt(
        &self,
        id: &str,
        durable: bool,
        apply: impl FnOnce(&mut StockRfq) -> Result<bool, String>,
    ) -> Result<Option<StockRfq>, String> {
        let result = self.change_rfq(id, durable, |r| {
            let before = r.clone();
            if !apply(r)? {
                return Ok(false);
            }
            if before
                .acceptance
                .as_ref()
                .is_some_and(|a| a.evidence_conflict)
            {
                // Additional evidence can be retained, but cannot silently resolve a conflict.
                r.settlement.paused = true;
                r.settlement.next_at_ms = None;
                r.problem = before.problem.clone();
                let updated_at = r.updated_at_ms;
                r.updated_at_ms = before.updated_at_ms;
                if *r == before {
                    return Ok(false);
                }
                r.updated_at_ms = updated_at;
            }
            Ok(true)
        });
        if let Err(problem) = &result {
            if self.plan_store.accepted_rfq(id).is_some() {
                self.change_rfq(id, true, |r| {
                    let a = r.acceptance.as_mut().ok_or("RFQ 接受记录缺失")?;
                    if a.evidence_conflict && r.problem.as_ref() == Some(problem) {
                        return Ok(false);
                    }
                    a.evidence_conflict = true;
                    r.settlement.paused = true;
                    r.settlement.next_at_ms = None;
                    r.problem = Some(problem.clone());
                    r.updated_at_ms = common::time::now_ms().max(r.updated_at_ms);
                    Ok(true)
                })?;
            }
        }
        result
    }

    // Internal transport only. The complete two-leg coordinator is not yet
    // exposed, so no public route can accept this financial commitment alone.
    #[allow(dead_code)]
    pub(super) async fn send_rfq_leg(
        self: &Arc<Self>,
        id: &str,
        hub: realtime::WsHub,
    ) -> Result<StockExecutionPlan, String> {
        let _order = self.order_lock.try_lock().map_err(|_| "股票计划正在处理")?;
        let _rfq = self.rfq_lock.try_lock().map_err(|_| "RFQ 操作进行中")?;
        let keys = (self.credential_loader)()?;
        let fingerprint = keys.fingerprint();
        let old = self.plan_store.get(id)?;
        if old.terms.account_fingerprint != fingerprint {
            return Err("请恢复原计划的账户凭证".into());
        }
        if old.rfq_acceptance.is_some() {
            return Ok(old);
        }
        if !matches!(
            old.terms.cex_instruction,
            Some(StockCexInstruction::AcceptRfq { .. })
        ) {
            return Err("原计划不是股票 RFQ 接受指令".into());
        }
        self.ensure_rfq_started(hub.clone());
        self.wait_rfq_subscription(&fingerprint).await?;
        let (plan, send) = self.begin_stock_rfq(id, &fingerprint)?;
        if !send {
            return Ok(plan);
        }
        self.submit_prepared_rfq(&keys, &plan, &hub).await
    }

    pub(super) async fn submit_prepared_rfq(
        &self,
        keys: &credentials::Credentials,
        plan: &StockExecutionPlan,
        hub: &realtime::WsHub,
    ) -> Result<StockExecutionPlan, String> {
        let id = &plan.plan_id;
        self.publish_rfq(&hub);
        let instruction = plan.terms.cex_instruction.as_ref().ok_or("原始指令缺失")?;
        let request_id = &plan.terms.rfq.as_ref().ok_or("原 RFQ 缺失")?.request_id;
        let response = self
            .signed_rfq_request(
                keys,
                reqwest::Method::POST,
                instruction.path(),
                instruction.signing_instruction(),
                &instruction.request_body(),
            )
            .await;
        let now = common::time::now_ms();
        match response {
            Ok(bytes) => {
                let result = rfq_protocol::acknowledgement(&bytes).and_then(|native| {
                    self.record_rfq_receipt(request_id, true, |r| {
                        let before = r.clone();
                        rfq_protocol::apply_rest(r, native, now)?;
                        r.acceptance.as_mut().ok_or("接受记录缺失")?.acknowledged = true;
                        r.updated_at_ms = now.max(r.updated_at_ms);
                        Ok(before != *r)
                    })
                });
                if let Err(problem) = result {
                    self.rfq_problem_record(request_id, problem)?;
                }
            }
            Err(error) => {
                self.change_rfq(request_id, true, |r| {
                    let a = r.acceptance.as_mut().ok_or("接受记录缺失")?;
                    if error.rejected {
                        if a.acknowledged
                            || matches!(
                                r.phase,
                                StockRfqPhase::AcceptedBinding | StockRfqPhase::Filled
                            )
                        {
                            a.evidence_conflict = true;
                            r.settlement.paused = true;
                        } else {
                            a.rejected = true;
                        }
                    }
                    r.problem = Some(error.problem);
                    r.updated_at_ms = now.max(r.updated_at_ms);
                    Ok(true)
                })?;
            }
        }
        self.publish_rfq(&hub);
        self.plan_store.get(id)
    }
}

pub(super) fn intent(
    plan: &StockExecutionPlan,
    mut r: StockRfq,
    now: i64,
) -> Result<StockRfq, String> {
    let binding = plan.terms.rfq.as_ref().ok_or("原计划没有 RFQ")?;
    if r.request.request_id != binding.request_id
        || r.rfq_id.as_ref() != Some(&binding.rfq_id)
        || r.account_fingerprint != plan.terms.account_fingerprint
        || r.current_candidate(true, now) != Some(&binding.candidate)
        || r.expiry_time_ms != Some(binding.expiry_time_ms)
    {
        return Err("RFQ 账户、报价或时窗已变化，不能接受原报价".into());
    }
    r.acceptance = Some(StockRfqAcceptance {
        plan_id: plan.plan_id.clone(),
        quote_id: binding.candidate.quote_id.clone(),
        taker_price: binding.candidate.taker_price.clone(),
        submitted_at_ms: now,
        acknowledged: false,
        rejected: false,
        evidence_conflict: false,
    });
    r.candidate = None;
    r.phase = StockRfqPhase::AwaitingQuotes;
    r.needs_recheck = true;
    r.updated_at_ms = now;
    r.settlement = StockRfqSettlement {
        attempts: 0,
        next_at_ms: Some(now.saturating_add(5000)),
        paused: false,
    };
    r.problem = Some("接受请求已记录；等待交易所确认，不重复接受或取消".into());
    Ok(r)
}

pub(super) fn validate(plan: &StockExecutionPlan) -> Result<(), String> {
    let Some(r) = &plan.rfq_acceptance else {
        return Ok(());
    };
    let a = r.acceptance.as_ref().ok_or("RFQ 接受记录缺失")?;
    let binding = plan.terms.rfq.as_ref().ok_or("计划未绑定原 RFQ")?;
    let Some(StockCexInstruction::AcceptRfq {
        rfq_id,
        quote_id,
        symbol,
        side,
        quantity,
        taker_price,
    }) = &plan.terms.cex_instruction
    else {
        return Err("RFQ 接受与计划指令不匹配".into());
    };
    let q = positive(quantity).ok_or("RFQ 原计划股数无效")?;
    let decimal = |s: &str| rust_decimal::Decimal::from_str_exact(s).ok();
    if plan.cex_order.is_some()
        || !matches!(
            plan.phase,
            StockPlanPhase::SubmissionUnknown | StockPlanPhase::Settled
        )
        || a.plan_id != plan.plan_id
        || a.quote_id != *quote_id
        || a.taker_price != *taker_price
        || a.submitted_at_ms < plan.terms.created_at_ms
        || a.submitted_at_ms >= plan.terms.market_valid_until_ms
        || a.submitted_at_ms < r.created_at_ms
        || r.updated_at_ms < a.submitted_at_ms
        || r.updated_at_ms > plan.updated_at_ms
        || r.request.request_id != binding.request_id
        || r.rfq_id.as_ref() != Some(rfq_id)
        || r.request.asset != plan.request.asset
        || r.symbol != *symbol
        || r.request.side != *side
        || positive(&r.request.quantity) != Some(q)
        || r.client_id == 0
        || r.account_fingerprint != plan.terms.account_fingerprint
        || r.expiry_time_ms != Some(binding.expiry_time_ms)
        || r.candidate.is_some()
        || r.cancel_requested
        || r.submission_time_ms
            .is_none_or(|t| t > a.submitted_at_ms || t < r.created_at_ms)
        || r.settlement.attempts > 6
        || (a.evidence_conflict && !r.settlement.paused)
        || (a.rejected
            && (a.acknowledged
                || matches!(
                    r.phase,
                    StockRfqPhase::AcceptedBinding | StockRfqPhase::Filled
                )))
        || !matches!(
            r.phase,
            StockRfqPhase::AwaitingQuotes
                | StockRfqPhase::AcceptedBinding
                | StockRfqPhase::Filled
                | StockRfqPhase::Cancelled
                | StockRfqPhase::Expired
        )
        || r.executed_quantity
            .as_deref()
            .is_some_and(|v| decimal(v).is_none_or(|v| v.is_sign_negative() || v > q))
        || r.executed_quote_quantity
            .as_deref()
            .is_some_and(|v| decimal(v).is_none_or(|v| v.is_sign_negative()))
        || r.fills.iter().any(|f| {
            f.quote_id != *quote_id
                || positive(&f.price).is_none()
                || positive(&f.quantity).is_none()
                || positive(&f.quote_quantity).is_none()
        })
        || r.fills.len() > 1
        || (!r.fills.is_empty() && r.phase != StockRfqPhase::Filled)
    {
        return Err("RFQ 回执身份、数量、时窗或资金占用不一致".into());
    }
    if let Some(fill) = r.fills.first() {
        if positive(&fill.quantity) != r.executed_quantity.as_deref().and_then(positive)
            || positive(&fill.quote_quantity)
                != r.executed_quote_quantity.as_deref().and_then(positive)
        {
            return Err("RFQ 成交明细与累计收支不一致".into());
        }
    }
    Ok(())
}

pub(super) fn transition(old: Option<&StockRfq>, new: Option<&StockRfq>) -> bool {
    let Some(old) = old else {
        return true;
    };
    let Some(new) = new else {
        return false;
    };
    let (Some(a), Some(b)) = (&old.acceptance, &new.acceptance) else {
        return false;
    };
    let unchanged = old.request == new.request
        && old.client_id == new.client_id
        && old.account_fingerprint == new.account_fingerprint
        && old.symbol == new.symbol
        && old.rfq_id == new.rfq_id
        && old.created_at_ms == new.created_at_ms
        && old.submission_time_ms == new.submission_time_ms
        && old.expiry_time_ms == new.expiry_time_ms
        && a.plan_id == b.plan_id
        && a.quote_id == b.quote_id
        && a.taker_price == b.taker_price
        && a.submitted_at_ms == b.submitted_at_ms
        && (!a.acknowledged || b.acknowledged)
        && (!a.rejected || b.rejected)
        && (!a.evidence_conflict || b.evidence_conflict)
        && new.updated_at_ms >= old.updated_at_ms
        && new.source_at_us >= old.source_at_us
        && new.settlement.attempts >= old.settlement.attempts
        && old.fills.iter().all(|f| new.fills.contains(f));
    let amounts = [&old.executed_quantity, &old.executed_quote_quantity]
        .into_iter()
        .zip([&new.executed_quantity, &new.executed_quote_quantity])
        .all(|(old, new)| {
            old.as_deref().is_none_or(|old| {
                new.as_deref().is_some_and(|new| {
                    rust_decimal::Decimal::from_str_exact(old)
                        .ok()
                        .zip(rust_decimal::Decimal::from_str_exact(new).ok())
                        .is_some_and(|(a, b)| b >= a)
                })
            })
        });
    unchanged
        && amounts
        && (!old.phase.terminal() || new.phase == old.phase)
        && (old.phase != StockRfqPhase::AcceptedBinding
            || matches!(
                new.phase,
                StockRfqPhase::AcceptedBinding
                    | StockRfqPhase::Filled
                    | StockRfqPhase::Cancelled
                    | StockRfqPhase::Expired
            ))
}

#[cfg(test)]
mod tests;
