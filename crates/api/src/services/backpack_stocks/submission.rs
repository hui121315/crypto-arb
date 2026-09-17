use super::*;

impl BackpackStocks {
    pub(crate) async fn execute_plan(
        self: &Arc<Self>,
        request: StockPlanExecutionRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        check_live(&trading.risk_config())?;
        self.execute_owned(request, hub, move |service, request, hub| async move {
            check_live(&trading.risk_config())?;
            match request.action {
                StockExecutionAction::Pair => service.send_stock_pair(&request.plan_id, hub).await,
                StockExecutionAction::Recovery { index } => service.send_recovery(&request.plan_id,index,&hub).await,
                StockExecutionAction::NativeTopup { index } => {
                    service
                        .send_native_topup(&request.plan_id, index, &hub)
                        .await
                }
            }
        })
        .await
    }

    pub(super) async fn execute_owned<F, Fut>(
        self: &Arc<Self>,
        request: StockPlanExecutionRequest,
        hub: realtime::WsHub,
        submit: F,
    ) -> Result<StockMarketSnapshot, String>
    where
        F: FnOnce(Arc<Self>, StockPlanExecutionRequest, realtime::WsHub) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<StockExecutionPlan, String>> + Send + 'static,
    {
        if self.execution_previous(&request)? {
            return Ok(self.snapshot());
        }
        let guard = self
            .submission_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "股票提交正在处理，请等待原计划结果，不会排队另发")?;
        let service = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // The bounded owner survives a cancelled HTTP request; the journal owns recovery.
        tokio::spawn(async move {
            let result = async {
                if service.execution_previous(&request)? {
                    return Ok(service.snapshot());
                }
                tokio::time::timeout(
                    Duration::from_secs(25),
                    submit(service.clone(), request, hub.clone()),
                )
                .await
                .map_err(|_| "股票提交等待超时，请核对原计划；不会自动重发或释放占用")??;
                Ok(service.snapshot())
            }
            .await;
            service.publish_rfq(&hub);
            drop(service);
            drop(guard);
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| "股票提交任务中断，请核对原计划；不会自动重发".to_owned())?
    }

    fn execution_previous(&self, request: &StockPlanExecutionRequest) -> Result<bool, String> {
        if !request.confirm_live {
            return Err("请确认本次股票计划的实盘提交".into());
        }
        let plan = self.plan_store.get(&request.plan_id)?;
        let fingerprint = (self.credential_loader)()?.fingerprint();
        if fingerprint != plan.terms.account_fingerprint {
            return Err("股票计划属于其他账户，请恢复原凭证".into());
        }
        let now = common::time::now_ms();
        let (submitted, valid) = match request.action {
            StockExecutionAction::Pair => (
                plan.two_leg_started_at_ms.is_some(),
                plan.phase_at(now) == StockPlanPhase::Reserved
                    && now >= plan.terms.created_at_ms
                    && now < plan.terms.market_valid_until_ms
                    && plan.cex_order.is_none()
                    && plan.rfq_acceptance.is_none()
                    && plan.chain_submission.is_none(),
            ),
            StockExecutionAction::NativeTopup { index } => {
                let row = plan.native_topups.get(index).ok_or("SOL 补回计划不存在")?;
                (
                    row.submission.is_some(),
                    plan.phase == StockPlanPhase::SubmissionUnknown
                        && index + 1 == plan.native_topups.len()
                        && now >= row.prepared_at_ms
                        && row
                            .valuation
                            .replenishment
                            .as_ref()
                            .is_some_and(|p| now < p.valid_until_ms),
                )
            }
            StockExecutionAction::Recovery { index } => {
                let row=plan.recoveries.get(index).ok_or("补偿计划不存在")?;
                (row.submission.is_some(),plan.phase==StockPlanPhase::SubmissionUnknown
                    && index+1==plan.recoveries.len() && row.cancelled_at_ms.is_none()
                    && now>=row.prepared_at_ms && now<row.cost.valid_until_ms)
            }
        };
        if submitted {
            return Ok(true);
        }
        if plan.revision != request.revision || !valid {
            return Err("股票计划版本已变化或报价过期，请刷新后重新确认；没有提交".into());
        }
        Ok(false)
    }
}

pub(super) fn check_live(risk: &trading::RiskConfig) -> Result<(), String> {
    if !risk.live_trading_enabled {
        return Err("当前为模拟环境，请在设置切换实盘后确认股票计划".into());
    }
    if risk.kill_switch_active {
        return Err("全局急停已开启，股票资金提交已停止".into());
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn stock_execution_obeys_global_live_and_kill_switch() {
    let mut risk = trading::RiskConfig::default();
    assert!(check_live(&risk).unwrap_err().contains("模拟环境"));
    risk.live_trading_enabled = true;
    assert!(check_live(&risk).is_ok());
    risk.kill_switch_active = true;
    assert!(check_live(&risk).unwrap_err().contains("急停"));
}
