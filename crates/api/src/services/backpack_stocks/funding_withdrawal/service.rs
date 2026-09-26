use super::*;

impl BackpackStocks {
    pub(crate) async fn submit_funding(
        self: &Arc<Self>,
        request: StockFundingSubmitRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        if self.funding_store.get(&request.plan_id)?.request.target==StockFundingTarget::Backpack {
            return self.submit_funding_transfer(request,hub,trading).await;
        }
        self.submit_funding_with(request, hub, true, move || trading.risk_config(),
            |service, plan, hub| async move { service.refresh_funding_submission(&plan, &hub).await }).await
    }

    pub(super) async fn submit_funding_with<R, F, Fut>(
        self: &Arc<Self>,
        request: StockFundingSubmitRequest,
        hub: realtime::WsHub,
        auto_followup: bool,
        risk: R,
        refresh: F,
    ) -> Result<StockMarketSnapshot, String>
    where
        R: Fn() -> trading::RiskConfig + Send + Sync + 'static,
        F: FnOnce(Arc<Self>, StockFundingPlan, realtime::WsHub) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<StockFundingPlan, String>> + Send + 'static,
    {
        if !request.confirm_live {
            return Err("请确认本次补库实盘提现".into());
        }
        let initial = self.funding_store.get(&request.plan_id)?;
        let keys = (self.credential_loader)()?;
        if keys.fingerprint() != initial.terms.account_fingerprint {
            return Err("请恢复补库原账户凭证".into());
        }
        // A prior durable intent is authoritative even if the HTTP acknowledgement was lost.
        if initial.withdrawal.is_some() {
            if auto_followup { self.resume_funding(hub); }
            return Ok(self.snapshot());
        }
        submission::check_live(&risk())?;
        if initial.revision != request.revision
            || initial.phase_at(common::time::now_ms()) != StockFundingPlanPhase::Reserved
            || initial.request.target != StockFundingTarget::Solana
        {
            return Err("补库计划已到期、版本变化或该转账方向尚未接入".into());
        }
        let body = payload(&initial, request.two_factor_token.as_deref())?;
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
                let fresh = refresh(service.clone(), initial.clone(), hub.clone()).await?;
                check_frozen_terms(&initial, &fresh)?;
                service.ensure_generation(generation, &initial.request.security_asset)?;
                if (service.credential_loader)()?.fingerprint() != keys.fingerprint() { return Err("账户已变化，未提现".into()); }
                submission::check_live(&risk())?;
                let now = common::time::now_ms();
                let begun = service.funding_store.update(&initial, StockFundingWithdrawal {
                    client_id: client_id(&initial), submitted_at_ms: now, query_count: 0,
                    last_query_at_ms: None, remote: None, receipt: None,
                    problem: Some("提交结果尚未确认；只查询原提现，不能重复提交".into()),
                    evidence_conflict: None,
                }, now)?;
                service.publish_plan(&hub);
                let response = service.signed_rfq_request(&keys, reqwest::Method::POST, PATH, "withdraw", &body).await;
                let mut updated = begun.withdrawal.clone().unwrap();
                let mut observed = begun.clone();
                observed.updated_at_ms = common::time::now_ms();
                match response {
                    Ok(bytes) => match serde_json::from_slice(&bytes).map_err(|_| ReadProblem::Unavailable("提现响应不完整".into())).and_then(|v| parse(&observed, v)) {
                        Ok(remote) => { updated.remote = Some(remote); updated.problem = Some("已取得提现回执，目标钱包到账尚未核验".into()); }
                        Err(problem) => problem.record(&mut updated),
                    },
                    Err(_) => updated.problem = Some("提现回复未确认，请检查权限、2FA、白名单或网络，并只查询原提现；不会自动重发".into()),
                }
                service.funding_store.update(&begun, updated, common::time::now_ms())?;
                // Immediate acknowledgement check; the bounded worker later follows this same ID.
                service.recheck_funding_inner(&initial.plan_id, &keys).await?;
                Ok(service.snapshot())
            }).await.map_err(|_| "提现处理超时，请查询原记录；已提交资金不会自动释放或重发".to_owned()).and_then(|r| r);
            service.publish_plan(&hub);
            drop(guard);
            if auto_followup { service.resume_funding(hub); }
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| "提现处理任务中断，请查询原记录，不能重新提币".to_owned())?
    }

    pub(in crate::services::backpack_stocks) async fn refresh_funding_submission(
        self: Arc<Self>,
        plan: &StockFundingPlan,
        hub: &realtime::WsHub,
    ) -> Result<StockFundingPlan, String> {
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票账户读取进行中")?;
        let generation = self.generation.load(Ordering::SeqCst);
        let request = StockPreflightRequest {
            source_plan: None,
            asset: plan.request.security_asset.clone(),
            wallet_address: Some(plan.request.wallet_address.clone()),
        };
        let inputs = self
            .read_preflight_inputs(&request, generation, hub)
            .await?;
        let keys = (self.credential_loader)()?;
        if inputs.fingerprint.as_deref() != Some(&plan.terms.account_fingerprint)
            || keys.fingerprint() != plan.terms.account_fingerprint
        {
            return Err("原账户余额未重新核实，未提现".into());
        }
        // Re-read transfer metadata at the fund-action boundary, even within the display cache TTL.
        let (tokens, assets) =
            protocol::asset_context(&self.read("/api/v1/assets").await?, &request.asset)?;
        let (cap,address)=self.funding_endpoints(&plan.request,&keys).await?;
        self.ensure_generation(generation, &request.asset)?;
        let mut snapshot = self.snapshot();
        snapshot.tokens = tokens;
        snapshot.funding_assets = assets;
        snapshot.token_metadata_at_ms = Some(common::time::now_ms());
        snapshot.token_metadata_problem = None;
        let report = snapshot
            .preflight
            .as_ref()
            .filter(|p| p.checked_at_ms == plan.request.preflight_at_ms)
            .ok_or("原补库检查已变化")?;
        let account = self.account.read();
        let account = account
            .evidence
            .as_ref()
            .filter(|a| a.fingerprint == plan.terms.account_fingerprint)
            .ok_or("账户库存未核实")?;
        funding_plan::prepare(
            plan.request.clone(),
            &snapshot,
            account,
            &inputs.wallet.ok_or("钱包库存未核实")?,
            &report.directions,
            cap,
            address,
            common::time::now_ms(),
        )
    }

    pub(crate) async fn recheck_funding(
        self: &Arc<Self>,
        request: StockPlanCancelRequest,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .submission_lock
            .try_lock()
            .map_err(|_| "股票资金操作进行中，请稍后查询原提现")?;
        let before = self.funding_store.get(&request.plan_id)?;
        let keys = (self.credential_loader)()?;
        if before.request.target==StockFundingTarget::Backpack {
            let result=tokio::time::timeout(Duration::from_secs(28),self.recheck_funding_transfer(&before,&keys)).await
                .map_err(|_|"原链上补库查询超时，未重新广播".to_owned()).and_then(|r|r);
            if self.funding_store.get(&before.plan_id)?.revision!=before.revision {self.publish_plan(&hub);}
            return result.map(|_|self.snapshot());
        }
        let result = tokio::time::timeout(
            Duration::from_secs(28),
            self.recheck_funding_inner(&request.plan_id, &keys),
        )
        .await
        .map_err(|_| "原提现查询超时，占用已保留，没有重提".to_owned())
        .and_then(|r| r);
        if self
            .funding_store
            .get(&request.plan_id)
            .is_ok_and(|p| p.revision != before.revision)
        {
            self.publish_plan(&hub);
        }
        result.map(|_| self.snapshot())
    }

    pub(in crate::services::backpack_stocks) async fn recheck_funding_inner(
        &self,
        id: &str,
        keys: &credentials::Credentials,
    ) -> Result<(), String> {
        self.recheck_funding_with(id, keys, |plan, hash| async move {
            crate::services::onchain_comparison::read_stock_funding_receipt(&plan, &hash).await
        })
        .await
    }

    pub(super) async fn recheck_funding_with<F, Fut>(
        &self,
        id: &str,
        keys: &credentials::Credentials,
        read_credit: F,
    ) -> Result<(), String>
    where
        F: FnOnce(StockFundingPlan, String) -> Fut,
        Fut: std::future::Future<Output = Result<StockFundingReceipt, String>>,
    {
        let original = self.funding_store.get(id)?;
        if original.terms.account_fingerprint != keys.fingerprint() {
            return Err("请恢复原提现账户凭证".into());
        }
        let mut w = original
            .withdrawal
            .clone()
            .ok_or("尚未提交，无需查询提现")?;
        let now = common::time::now_ms();
        if w.last_query_at_ms
            .is_some_and(|at| now.saturating_sub(at) < QUERY_INTERVAL_MS)
        {
            return Err("原提现查询间隔 5 秒，请等待原结果".into());
        }
        w.query_count = w
            .query_count
            .checked_add(1)
            .ok_or("原提现查询次数超出范围")?;
        w.last_query_at_ms = Some(now);
        let queried = self.funding_store.update(&original, w.clone(), now)?;
        let result = async {
            let bytes = self
                .signed_rfq_request(
                    keys,
                    reqwest::Method::GET,
                    PATH,
                    "withdrawalQueryAll",
                    &json!({"clientId":w.client_id,"limit":2}),
                )
                .await
                .map_err(|_| ReadProblem::Unavailable("原提现历史读取失败，已保留占用，不重复提币".into()))?;
            let rows: Vec<Value> =
                serde_json::from_slice(&bytes).map_err(|_| ReadProblem::Unavailable("原提现历史响应不完整".into()))?;
            if rows.is_empty() {
                return Err(ReadProblem::Unavailable("尚未查到原提现；空历史不代表未扣款，不能重提".into()));
            }
            if rows.len() != 1 {
                return Err(ReadProblem::Conflict("同一提现编号出现多条记录，需要核对，不合并金额".into()));
            }
            let mut observed = queried.clone();
            observed.updated_at_ms = common::time::now_ms();
            let remote = parse(&observed, rows.into_iter().next().unwrap())?;
            w.remote = Some(remote);
            w.problem = w.receipt.as_ref().map(|r| credit_problem(&queried, r));
            let recorded =
                self.funding_store
                    .update(&queried, w.clone(), common::time::now_ms())
                    .map_err(ReadProblem::Unavailable)?;
            if w.receipt.is_some() {
                return Ok(());
            }
            self.verify_funding_credit(&recorded, read_credit).await.map_err(ReadProblem::Unavailable)
        }
        .await;
        if let Err(problem) = result {
            let current = self.funding_store.get(id)?;
            let mut saved = current.withdrawal.clone().ok_or("原提现记录缺失")?;
            problem.record(&mut saved);
            self.funding_store
                .update(&current, saved, common::time::now_ms())?;
        }
        Ok(())
    }

    async fn verify_funding_credit<F, Fut>(
        &self,
        plan: &StockFundingPlan,
        read_credit: F,
    ) -> Result<(), String>
    where
        F: FnOnce(StockFundingPlan, String) -> Fut,
        Fut: std::future::Future<Output = Result<StockFundingReceipt, String>>,
    {
        let mut w = plan.withdrawal.clone().ok_or("原提现记录缺失")?;
        let remote = w.remote.as_ref().ok_or("原提现回执未知")?;
        if remote.is_internal {
            return Err("交易所记为内部划转，未取得目标链上到账证据".into());
        }
        if remote.status != "confirmed" {
            return Err(format!("交易所提现状态 {}，继续查询原记录", remote.status));
        }
        let hash = remote
            .transaction_hash
            .as_deref()
            .ok_or("原提现尚无链上交易编号")?;
        let receipt = read_credit(plan.clone(), hash.to_owned()).await?;
        w.problem = Some(credit_problem(plan, &receipt));
        w.receipt = Some(receipt);
        self.funding_store.update(plan, w, common::time::now_ms())?;
        Ok(())
    }
}

fn credit_problem(plan: &StockFundingPlan, receipt: &StockFundingReceipt) -> String {
    let sufficient = receipt.credited_raw.parse::<u64>().ok()
        .zip(plan.terms.minimum_credit_raw.parse::<u64>().ok())
        .is_some_and(|(credited, minimum)| credited >= minimum);
    if sufficient {
        "链上已到账；交易所实际扣账与费用币种/含费口径尚未核清，保留占用"
    } else {
        "链上到账低于补库目标；实际费用与剩余缺口待核清，保留占用"
    }.into()
}

pub(in crate::services::backpack_stocks) fn check_frozen_terms(old: &StockFundingPlan, fresh: &StockFundingPlan) -> Result<(), String> {
    funding_plan::validate(fresh)?;
    let a = &old.terms;
    let b = &fresh.terms;
    if old.request != fresh.request
        || a.account_fingerprint != b.account_fingerprint
        || a.security != b.security
        || a.token != b.token
        || a.destination != b.destination
        || a.quantity != b.quantity
        || a.source_budget != b.source_budget
        || a.minimum_credit_raw != b.minimum_credit_raw
        || a.mint.address != b.mint.address
        || a.mint.decimals != b.mint.decimals
        || a.mint.ui_multiplier != b.mint.ui_multiplier
        || a.mint.next_change_at_ms != b.mint.next_change_at_ms
    {
        return Err("补库数量、费用、份额或收款身份已变化，请取消旧预留后重新检查；未提现".into());
    }
    Ok(())
}
