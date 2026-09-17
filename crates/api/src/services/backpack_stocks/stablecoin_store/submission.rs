use super::*;

impl StablecoinStore {
    pub(in crate::services::backpack_stocks) fn get(
        &self,
        id: &str,
    ) -> Result<StockStablecoinPlan, String> {
        let inner = self.inner.lock();
        if let Some(e) = &inner.problem {
            return Err(e.clone());
        }
        inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .cloned()
            .ok_or_else(|| "兑换计划不存在".into())
    }

    pub(in crate::services::backpack_stocks) fn begin(
        &self,
        request: &StockStablecoinSubmitRequest,
        signed: &str,
        now: i64,
    ) -> Result<(StockStablecoinPlan, bool), String> {
        let mut inner = self.inner.lock();
        if let Some(e) = &inner.problem {
            return Err(e.clone());
        }
        if !request.confirm_live {
            return Err("请确认本次稳定币实盘兑换".into());
        }
        let old = inner
            .rows
            .values()
            .find(|p| p.plan_id == request.plan_id)
            .ok_or("兑换计划不存在")?;
        if old.submission.is_some() {
            return Ok((old.clone(), false));
        }
        if old.phase_at(now) != StockStablecoinPlanPhase::Reserved
            || old.revision != request.revision
            || !old.preview.can_reserve(now)
        {
            return Err("兑换计划版本变化、已取消或报价过期，未提交".into());
        }
        let mut plan = old.clone();
        plan.submission = Some(execution::intent(
            plan.preview.cost.as_ref().ok_or("兑换缺少原交易")?,
            signed,
            now,
        )?);
        plan.phase = StockStablecoinPlanPhase::SubmissionUnknown;
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        plan.revision = plan.revision.checked_add(1).ok_or("兑换版本溢出")?;
        transition(old, &plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok((plan, true))
    }

    pub(in crate::services::backpack_stocks) fn change_submission<F>(
        &self,
        id: &str,
        now: i64,
        change: F,
    ) -> Result<StockStablecoinPlan, String>
    where
        F: FnOnce(&mut StockChainSubmission) -> Result<(), String>,
    {
        let mut inner = self.inner.lock();
        if let Some(e) = &inner.problem {
            return Err(e.clone());
        }
        let old = inner
            .rows
            .values()
            .find(|p| p.plan_id == id)
            .ok_or("兑换计划不存在")?;
        let mut plan = old.clone();
        change(
            plan.submission
                .as_mut()
                .ok_or("兑换尚未提交，没有原交易可查询")?,
        )?;
        if &plan == old {
            return Ok(old.clone());
        }
        plan.phase = plan.receipt_phase();
        plan.updated_at_ms = now.max(plan.updated_at_ms);
        plan.revision = plan.revision.checked_add(1).ok_or("兑换版本溢出")?;
        transition(old, &plan)?;
        self.persist(&mut inner, &plan, now)?;
        inner
            .rows
            .insert(plan.request.request_id.clone(), plan.clone());
        Ok(plan)
    }
}
