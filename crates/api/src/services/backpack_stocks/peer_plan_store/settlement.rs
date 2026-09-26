use super::*;

impl Store {
    pub(in crate::services::backpack_stocks) fn settle(
        &self,
        request: &StockPlanRevisionRequest,
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
        if old.phase == StockPeerPlanPhase::Settled {
            return Ok((old.clone(), false));
        }
        if old.revision != request.revision {
            return Err("回执版本已变化，请核对最新收支后结算".into());
        }
        let now = now.max(old.updated_at_ms);
        if let Some(problem) = old.peer_settlement_problem(now) {
            return Err(problem);
        }
        let mut p = old.clone();
        p.settlement = Some(StockPeerSettlement {
            source_revision: old.revision,
            settled_at_ms: now,
            accounting: old.accounting(),
        });
        p.phase = StockPeerPlanPhase::Settled;
        p.revision = old.revision.checked_add(1).ok_or("计划版本溢出")?;
        p.updated_at_ms = now;
        transition(old, &p)?;
        // WalletClaims commits the durable journal before releasing either hold.
        self.persist(&mut i, &p, now)?;
        i.rows.insert(p.request.request_id.clone(), p.clone());
        Ok((p, true))
    }
}
