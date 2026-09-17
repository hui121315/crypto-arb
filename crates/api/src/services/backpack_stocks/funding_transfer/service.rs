use super::*;
use rust_decimal::{prelude::ToPrimitive, Decimal};

impl BackpackStocks {
    pub(crate) async fn prepare_funding_transfer(
        self: &Arc<Self>,
        request: StockPlanRevisionRequest,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.prepare_funding_transfer_with(request, hub, |service, plan, hub| async move {
            let fresh = service
                .clone()
                .refresh_funding_submission(&plan, &hub)
                .await?;
            funding_withdrawal::check_frozen_terms(&plan, &fresh)?;
            let retained = service.funding_sol_reserve(&plan)?;
            let (client, url) = chain::endpoint().await?;
            chain::prepare_with(&client, &url, &plan, retained).await
        })
        .await
    }

    pub(super) async fn prepare_funding_transfer_with<W, F>(
        self: &Arc<Self>,
        request: StockPlanRevisionRequest,
        hub: realtime::WsHub,
        work: W,
    ) -> Result<StockMarketSnapshot, String>
    where
        W: FnOnce(Arc<Self>, StockFundingPlan, realtime::WsHub) -> F,
        F: std::future::Future<Output = Result<StockFundingTransferPreparation, String>>,
    {
        let _guard = self
            .submission_lock
            .try_lock()
            .map_err(|_| "股票资金操作进行中")?;
        let initial = self.funding_store.get(&request.plan_id)?;
        if (self.credential_loader)()?.fingerprint() != initial.terms.account_fingerprint {
            return Err("请恢复补库原账户凭证".into());
        }
        if initial.transfer.is_some() {
            return Ok(self.snapshot());
        }
        if initial.revision != request.revision
            || initial.phase_at(common::time::now_ms()) != StockFundingPlanPhase::Reserved
            || initial.request.target != StockFundingTarget::Backpack
        {
            return Err("原补库计划已到期或发生变化".into());
        }
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &initial.request.security_asset)?;
        let p = work(self.clone(), initial.clone(), hub.clone()).await?;
        self.ensure_generation(generation, &initial.request.security_asset)?;
        if (self.credential_loader)()?.fingerprint() != initial.terms.account_fingerprint {
            return Err("账户已变化，未保存转账".into());
        }
        self.funding_store.update_transfer(
            &initial,
            StockFundingTransfer {
                preparation: p,
                submitted_at_ms: None,
                transaction_hash: None,
                acknowledged: false,
                query_count: 0,
                last_query_at_ms: None,
                receipt: None,
                deposit: None,
                problem: None,
            },
            common::time::now_ms(),
        )?;
        self.publish_plan(&hub);
        Ok(self.snapshot())
    }

    fn funding_sol_reserve(&self, plan: &StockFundingPlan) -> Result<u64, String> {
        let snapshot = self.snapshot.read();
        let report = snapshot
            .preflight
            .as_ref()
            .filter(|p| p.checked_at_ms == plan.request.preflight_at_ms)
            .ok_or("原补库预检已变化")?;
        let row = report
            .directions
            .iter()
            .find(|r| r.direction == plan.request.direction.label())
            .ok_or("原套利方向未知")?;
        let mut found = false;
        let reserve = row
            .inventory
            .iter()
            .filter(|r| {
                r.location == "Solana" && ["SOL", "SOL / 保守周转余额"].contains(&r.asset.as_str())
            })
            .try_fold(Decimal::ZERO, |n, r| -> Result<Decimal, String> {
                found = true;
                n.checked_add(decimal(
                    r.required.as_deref().ok_or("原套利 SOL 备款未知")?,
                )?)
                .ok_or("SOL 备款溢出".into())
            });
        if !found {
            return Err("原套利缺少 SOL 备款，不能占用后续交易所需资金".into());
        }
        reserve?
            .checked_mul(Decimal::from(1_000_000_000u64))
            .and_then(|n| n.ceil().to_u64())
            .ok_or("SOL 备款无法转为原始单位".into())
    }

    pub(in crate::services::backpack_stocks) async fn submit_funding_transfer(
        self: &Arc<Self>,
        request: StockFundingSubmitRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        self.submit_funding_transfer_with(
            request,
            hub,
            true,
            move || trading.risk_config(),
            |service, plan, hub| async move {
                let fresh = service
                    .clone()
                    .refresh_funding_submission(&plan, &hub)
                    .await?;
                funding_withdrawal::check_frozen_terms(&plan, &fresh)?;
                let p = &plan
                    .transfer
                    .as_ref()
                    .ok_or("请先核算原转账费用")?
                    .preparation;
                if service.funding_sol_reserve(&plan)? > p.retained_sol_lamports {
                    return Err("后续套利的 SOL 备款增加，请重建补库计划".into());
                }
                let (client, url) = chain::endpoint().await?;
                chain::check_with(&client, &url, &plan, p).await?;
                let signed = crate::services::onchain_signer::sign(
                    "solana",
                    &plan.request.wallet_address,
                    &chain::unsigned(&plan, p)?,
                    None,
                    None,
                )?;
                Ok((client, url, signed))
            },
        )
        .await
    }

    pub(super) async fn submit_funding_transfer_with<R, W, F>(
        self: &Arc<Self>,
        request: StockFundingSubmitRequest,
        hub: realtime::WsHub,
        auto_followup: bool,
        risk: R,
        work: W,
    ) -> Result<StockMarketSnapshot, String>
    where
        R: Fn() -> trading::RiskConfig + Send + Sync + 'static,
        W: FnOnce(Arc<Self>, StockFundingPlan, realtime::WsHub) -> F + Send + 'static,
        F: std::future::Future<Output = Result<(reqwest::Client, String, String), String>>
            + Send
            + 'static,
    {
        if !request.confirm_live || request.two_factor_token.is_some() {
            return Err("请确认原 Solana 转账；链上转入不使用 Backpack 2FA".into());
        }
        let initial = self.funding_store.get(&request.plan_id)?;
        let keys = (self.credential_loader)()?;
        if keys.fingerprint() != initial.terms.account_fingerprint {
            return Err("请恢复补库原账户凭证".into());
        }
        if initial
            .transfer
            .as_ref()
            .is_some_and(|t| t.submitted_at_ms.is_some())
        {
            if auto_followup { self.resume_funding(hub); }
            return Ok(self.snapshot());
        }
        submission::check_live(&risk())?;
        if initial.revision != request.revision
            || initial.phase_at(common::time::now_ms()) != StockFundingPlanPhase::Reserved
            || initial.request.target != StockFundingTarget::Backpack
            || initial.transfer.is_none()
        {
            return Err("请核算原转账费用，或重新检查已过期的补库计划".into());
        }
        let guard = self
            .submission_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "股票资金操作进行中，请核对原记录")?;
        let service = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(45), async {
                let generation = service.generation.load(Ordering::SeqCst);
                service.ensure_generation(generation, &initial.request.security_asset)?;
                let (client, url, signed) =
                    work(service.clone(), initial.clone(), hub.clone()).await?;
                service.ensure_generation(generation, &initial.request.security_asset)?;
                if (service.credential_loader)()?.fingerprint() != keys.fingerprint() {
                    return Err("账户已变化，未广播补库交易".into());
                }
                submission::check_live(&risk())?;
                let mut t = initial.transfer.clone().ok_or("原转账费用缺失")?;
                let hash = chain::signed_identity(&initial, &t.preparation, &signed)?;
                let now = common::time::now_ms();
                t.submitted_at_ms = Some(now);
                t.transaction_hash = Some(hash);
                t.problem = Some("已记录原交易；提交结果待核验，不会重新广播".into());
                let begun = service.funding_store.update_transfer(&initial, t, now)?;
                service.publish_plan(&hub);
                let mut t = begun.transfer.clone().unwrap();
                match chain::send_with(&client, &url, &begun, &t.preparation, &signed).await {
                    Ok(()) => {
                        t.acknowledged = true;
                        t.problem = Some("RPC 已接收，链上最终确认与 Backpack 入账待核验".into());
                    }
                    Err(_) => {
                        t.problem =
                            Some("广播回复未确认，只查询已保存的原签名，不会再次发送".into())
                    }
                }
                let sent =
                    service
                        .funding_store
                        .update_transfer(&begun, t, common::time::now_ms())?;
                service
                    .recheck_funding_transfer_with(&sent, &keys, move |plan| async move {
                        chain::receipt_with(&client, &url, &plan).await
                    })
                    .await?;
                Ok(service.snapshot())
            })
            .await
            .map_err(|_| "补库处理超时，请查询原签名；已提交资金不会到期释放或重发".to_owned())
            .and_then(|r| r);
            service.publish_plan(&hub);
            drop(guard);
            if auto_followup { service.resume_funding(hub); }
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| "补库处理任务中断，请核对原转账，不要重新发送".to_owned())?
    }

    pub(in crate::services::backpack_stocks) async fn recheck_funding_transfer(
        &self,
        plan: &StockFundingPlan,
        keys: &credentials::Credentials,
    ) -> Result<(), String> {
        self.recheck_funding_transfer_with(plan, keys, |plan| async move {
            let (client, url) = chain::endpoint().await?;
            chain::receipt_with(&client, &url, &plan).await
        })
        .await
    }

    pub(super) async fn recheck_funding_transfer_with<F, Fut>(
        &self,
        plan: &StockFundingPlan,
        keys: &credentials::Credentials,
        read_receipt: F,
    ) -> Result<(), String>
    where
        F: FnOnce(StockFundingPlan) -> Fut,
        Fut: std::future::Future<Output = Result<StockFundingTransferReceipt, String>>,
    {
        if keys.fingerprint() != plan.terms.account_fingerprint {
            return Err("原 Backpack 入账账户不一致".into());
        }
        let mut t = plan
            .transfer
            .clone()
            .filter(|t| t.submitted_at_ms.is_some())
            .ok_or("原补库尚未提交")?;
        if !plan.phase.holds_funds() {
            return Ok(());
        }
        let now = common::time::now_ms();
        if t.last_query_at_ms
            .is_some_and(|at| now < at.saturating_add(5_000))
        {
            return Ok(());
        }
        t.query_count = t.query_count.checked_add(1).ok_or("补库查询次数溢出")?;
        t.last_query_at_ms = Some(now);
        let mut current = self.funding_store.update_transfer(plan, t.clone(), now)?;
        if t.receipt.is_none() {
            match read_receipt(current.clone()).await {
                Ok(receipt) => {
                    t.receipt = Some(receipt);
                    t.problem = Some("原链上交易已核对，Backpack 入账待核验".into());
                    current = self.record_funding_observation(
                        &current,
                        t.clone(),
                        common::time::now_ms(),
                    )?;
                }
                Err(_) => {
                    t.problem = Some("原链上交易尚未完成终态与收支核验，保留占用；没有重发".into());
                    self.funding_store
                        .update_transfer(&current, t, common::time::now_ms())?;
                    return Ok(());
                }
            }
        }
        let receipt = t.receipt.as_ref().unwrap();
        if !receipt.within_plan || !receipt.succeeded {
            t.problem = Some(
                if !receipt.within_plan {
                    "实际收支超出原计划，已保存原始数量和网络费，继续保留占用"
                } else {
                    "原链上交易失败，实际网络费已记录；转账金额未扣除，原计划不重发"
                }
                .into(),
            );
            self.funding_store
                .update_transfer(&current, t, common::time::now_ms())?;
            return Ok(());
        }
        let result = self.find_funding_deposit(&current, keys).await;
        match result {
            Ok(Some(deposit)) => {
                let confirmed = deposit.status == "confirmed"
                    && decimal(&deposit.quantity)? == decimal(&current.terms.quantity)?;
                t.problem=Some(if confirmed{"Backpack 已确认原转账入账；实际转出、到账与 SOL 网络费已记录，下一笔交易仍须重新预检"}else{"已找到原交易入账记录，但状态或数量尚未满足计划，继续保留占用"}.into());
                t.deposit = Some(deposit);
            }
            Ok(None) => {
                t.problem = Some("链上已转出，Backpack 入账记录尚未出现；只查询原交易".into())
            }
            Err(problem) => t.problem = Some(problem),
        }
        self.record_funding_observation(&current, t, common::time::now_ms())?;
        Ok(())
    }

    fn record_funding_observation(
        &self,
        plan: &StockFundingPlan,
        t: StockFundingTransfer,
        now: i64,
    ) -> Result<StockFundingPlan, String> {
        if matches!(
            phase(plan, &t),
            StockFundingPlanPhase::Deposited | StockFundingPlanPhase::TransferFailed
        ) {
            // Invalidate old stock inventory before releasing the shared account claim.
            self.invalidate_funding_inventory(&plan.terms.account_fingerprint);
        }
        self.funding_store.update_transfer(plan, t, now)
    }

    async fn find_funding_deposit(
        &self,
        plan: &StockFundingPlan,
        keys: &credentials::Credentials,
    ) -> Result<Option<StockFundingDepositRecord>, String> {
        let from = plan
            .transfer
            .as_ref()
            .and_then(|t| t.submitted_at_ms)
            .ok_or("原提交时间未知")?
            .saturating_sub(5000);
        let to = common::time::now_ms();
        let mut observed = plan.clone();
        observed.updated_at_ms = to;
        let mut found = None;
        for page in 0..4 {
            let bytes=self.signed_rfq_request(keys,reqwest::Method::GET,"/wapi/v1/capital/deposits","depositQueryAll",
                &json!({"from":from,"to":to,"limit":100,"offset":page*100,"excludePlatform":true})).await
                .map_err(|_|"Backpack 原入账历史查询失败，保留占用；没有转账重试")?;
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "Backpack 入账历史无法解析")?;
            if let Some(deposit) = deposit_from_rows(&observed, &value)? {
                if found.is_some() {
                    return Err("同一原交易在多页历史中重复，继续保留占用".into());
                }
                found = Some(deposit);
            }
            if value.as_array().is_some_and(|r| r.len() < 100) {
                return Ok(found);
            }
        }
        Err("入账历史超过本次 400 条核验上限，未自动放行或重新转账".into())
    }
}
