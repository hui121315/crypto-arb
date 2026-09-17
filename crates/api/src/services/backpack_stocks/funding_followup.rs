use super::*;
use std::sync::Weak;

fn delay_ms(attempt: u8) -> i64 {
    5_000_i64
        .saturating_mul(1_i64 << attempt.min(4))
        .min(60_000)
}

pub(super) fn validate(plan: &StockFundingPlan) -> Result<(), String> {
    let Some(f) = &plan.followup else {
        return Ok(());
    };
    let submitted = plan
        .withdrawal
        .as_ref()
        .map(|w| w.submitted_at_ms)
        .or_else(|| plan.transfer.as_ref()?.submitted_at_ms)
        .ok_or("自动核验缺少原提交")?;
    if !(1..=STOCK_FUNDING_FOLLOWUP_LIMIT).contains(&f.attempts)
        || f.last_at_ms < submitted.saturating_add(5_000)
        || f.last_at_ms > plan.updated_at_ms
        || f.paused != f.next_at_ms.is_none()
        || (f.attempts == STOCK_FUNDING_FOLLOWUP_LIMIT && !f.paused)
        || f.next_at_ms
            .is_some_and(|at| at != f.last_at_ms.saturating_add(delay_ms(f.attempts)))
        || f.problem
            .as_ref()
            .is_some_and(|p| p.is_empty() || p.len() > 1024)
    {
        return Err("补库自动核验次数或时序无效".into());
    }
    Ok(())
}

pub(super) fn transition(old: &StockFundingPlan, new: &StockFundingPlan) -> Result<(), String> {
    if old.withdrawal != new.withdrawal || old.transfer != new.transfer || old.phase != new.phase {
        return Err("自动核验调度不能修改原转账或到账凭据".into());
    }
    let f = new.followup.as_ref().ok_or("自动核验记录不能删除")?;
    let previous = old.followup.as_ref();
    if f.attempts == previous.map_or(1, |p| p.attempts.saturating_add(1)) {
        if old
            .funding_followup_at()
            .is_none_or(|at| at > new.updated_at_ms)
            || f.last_at_ms != new.updated_at_ms
            || f.paused != (f.attempts == STOCK_FUNDING_FOLLOWUP_LIMIT)
            || f.problem.is_some()
        {
            return Err("自动核验尚未到期、已暂停或超出预算".into());
        }
    } else {
        let p = previous.ok_or("自动核验缺少原尝试记录")?;
        if f.attempts != p.attempts
            || f.last_at_ms != p.last_at_ms
            || (p.paused && !f.paused)
            || (!f.paused && f.next_at_ms != p.next_at_ms)
            || f.problem.is_none()
        {
            return Err("自动核验预算与暂停状态不能被重置".into());
        }
    }
    Ok(())
}

impl BackpackStocks {
    pub(crate) fn resume_funding(self: &Arc<Self>, hub: realtime::WsHub) {
        let mut worker = self.funding_worker.lock();
        if self.funding_store.next_followup().is_some()
            && worker.as_ref().is_none_or(|h| h.is_finished())
        {
            *worker = Some(tokio::spawn(run(Arc::downgrade(self), hub)));
        }
    }

    async fn funding_followup_step(&self, id: &str, hub: &realtime::WsHub) -> Result<bool, String> {
        let Ok(_guard) = self.submission_lock.try_lock() else {
            return Ok(false);
        };
        let original = self.funding_store.get(id)?;
        let now = common::time::now_ms();
        if original.funding_followup_at().is_none_or(|at| at > now) {
            return Ok(false);
        }
        let attempts = original.followup.as_ref().map_or(1, |f| f.attempts + 1);
        let paused = attempts == STOCK_FUNDING_FOLLOWUP_LIMIT;
        let begun = self.funding_store.update_followup(
            &original,
            StockFundingFollowup {
                attempts,
                last_at_ms: now,
                next_at_ms: (!paused).then(|| now.saturating_add(delay_ms(attempts))),
                paused,
                problem: None,
            },
            now,
        )?;
        // Persist the attempt before credential loading or network I/O. Restart never resets it.
        let keys = match (self.credential_loader)() {
            Ok(keys) if keys.fingerprint() == begun.terms.account_fingerprint => keys,
            _ => {
                self.funding_followup_problem(
                    id,
                    "自动核验已暂停：请恢复原 Backpack 凭证后手动核对",
                    true,
                )?;
                self.publish_plan(hub);
                return Ok(true);
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(28), async {
            match begun.request.target {
                StockFundingTarget::Solana => self.recheck_funding_inner(id, &keys).await,
                StockFundingTarget::Backpack => self.recheck_funding_transfer(&begun, &keys).await,
            }
        })
        .await
        .map_err(|_| "原补库核验超时，已保留原交易与占用；未重新转账".to_owned())
        .and_then(|r| r);
        if let Err(problem) = result {
            self.funding_followup_problem(id, &problem, false)?;
        }
        self.publish_plan(hub);
        Ok(true)
    }

    fn funding_followup_problem(&self, id: &str, problem: &str, pause: bool) -> Result<(), String> {
        let plan = self.funding_store.get(id)?;
        let mut f = plan.followup.clone().ok_or("自动核验尚未记录")?;
        f.problem = Some(problem.into());
        if pause {
            f.paused = true;
            f.next_at_ms = None;
        }
        if plan.followup.as_ref() != Some(&f) {
            self.funding_store
                .update_followup(&plan, f, common::time::now_ms())?;
        }
        Ok(())
    }
}

async fn run(service: Weak<BackpackStocks>, hub: realtime::WsHub) {
    loop {
        let Some(s) = service.upgrade() else {
            return;
        };
        let next = {
            // Coordinate idle exit with submit/start so a newly saved intent is not stranded.
            let mut worker = s.funding_worker.lock();
            let next = s.funding_store.next_followup();
            if next.is_none() {
                worker.take();
                return;
            }
            next.unwrap()
        };
        let wait = next.1.saturating_sub(common::time::now_ms()).max(0) as u64;
        if wait > 0 {
            drop(s);
            tokio::time::sleep(Duration::from_millis(wait.min(30_000))).await;
            continue;
        }
        let result = s.funding_followup_step(&next.0, &hub).await;
        if result.is_err() {
            // A journal/transition error cannot be repaired by polling the exchange again.
            s.funding_worker.lock().take();
            s.publish_plan(&hub);
            return;
        }
        drop(s);
        if matches!(result, Ok(false)) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod tests;
