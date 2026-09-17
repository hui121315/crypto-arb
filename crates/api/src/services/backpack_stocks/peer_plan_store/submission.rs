use super::*;

impl Store {
    pub(in crate::services::backpack_stocks) fn get(
        &self,
        id: &str,
    ) -> Result<StockPeerPlan, String> {
        let i = self.inner.lock();
        if let Some(e) = &i.problem {
            return Err(e.clone());
        }
        i.rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or("股票双边计划不存在".into())
    }

    pub(in crate::services::backpack_stocks) fn pending(
        &self,
        fingerprint: &str,
    ) -> Vec<StockPeerPlan> {
        self.inner
            .lock()
            .rows
            .values()
            .filter(|p| {
                p.phase == StockPeerPlanPhase::SubmissionUnknown
                    && p.terms.account_fingerprint == fingerprint
            })
            .cloned()
            .collect()
    }
    pub(in crate::services::backpack_stocks) fn begin(
        &self,
        request: &StockPeerExecutionRequest,
        fingerprint: &str,
        chain: StockChainSubmission,
        now: i64,
    ) -> Result<(StockPeerPlan, bool), String> {
        let mut i = self.inner.lock();
        if let Some(e) = &i.problem {
            return Err(e.clone());
        }
        let old = i
            .rows
            .values()
            .find(|p| p.plan_id == request.plan_id)
            .ok_or("股票双边计划不存在")?;
        if !request.confirm_live || old.terms.account_fingerprint != fingerprint {
            return Err("本次提交未确认或原 Kraken 账户已改变".into());
        }
        if old.phase == StockPeerPlanPhase::SubmissionUnknown {
            return Ok((old.clone(), false));
        }
        if old.phase != StockPeerPlanPhase::Reserved
            || old.revision != request.revision
            || now < old.terms.created_at_ms
            || now >= old.terms.market_valid_until_ms
        {
            return Err("股票计划版本变化或原报价过期，没有提交".into());
        }
        let mut p = old.clone();
        let client = format!("sp{}", &uuid::Uuid::new_v4().simple().to_string()[..16]);
        if i.rows.values().any(|r| {
            r.cex_order
                .as_ref()
                .is_some_and(|c| c.client_order_id == client)
        }) {
            return Err("股票订单编号冲突，没有提交".into());
        }
        let cex = StockPeerOrderReceipt::pending(p.terms.draft.clone(), client)?;
        cex.kraken_submission("local-identity-check", 1, now)?;
        p.cex_order = Some(cex);
        p.chain_submission = Some(chain);
        p.phase = StockPeerPlanPhase::SubmissionUnknown;
        p.revision += 1;
        p.updated_at_ms = now;
        transition(old, &p)?;
        self.persist(&mut i, &p, now)?;
        i.rows.insert(p.request.request_id.clone(), p.clone());
        Ok((p, true))
    }

    pub(in crate::services::backpack_stocks) fn update(
        &self,
        id: &str,
        change: impl FnOnce(&mut StockPeerPlan) -> Result<(), String>,
    ) -> Result<StockPeerPlan, String> {
        let mut i = self.inner.lock();
        if let Some(e) = &i.problem {
            return Err(e.clone());
        }
        let old = i
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .ok_or("股票双边计划不存在")?;
        if old.phase != StockPeerPlanPhase::SubmissionUnknown {
            return Err("尚未提交双边计划".into());
        }
        let mut p = old.clone();
        change(&mut p)?;
        if p == *old {
            return Ok(p);
        }
        p.revision = old.revision.checked_add(1).ok_or("股票计划版本溢出")?;
        p.updated_at_ms = common::time::now_ms().max(old.updated_at_ms);
        transition(old, &p)?;
        self.persist(&mut i, &p, p.updated_at_ms)?;
        i.rows.insert(p.request.request_id.clone(), p.clone());
        Ok(p)
    }

    pub(in crate::services::backpack_stocks) fn receipt(
        &self,
        id: &str,
        incoming: &StockPeerOrderReceipt,
    ) -> Result<StockPeerPlan, String> {
        self.update(id, |p| {
            let row = p.cex_order.as_mut().ok_or("原 Kraken 订单丢失")?;
            if let Err(e) = row.merge_snapshot(incoming) {
                row.mark_conflict(e);
            }
            if row.receipt_complete() {
                p.cex_history.problem = None;
            }
            if row.receipt_complete()
                && p.chain_submission
                    .as_ref()
                    .is_some_and(|r| r.receipt.is_some())
            {
                p.execution_problem = None;
            }
            Ok(())
        })
    }
}
