use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;
use exchange::ExchangeAdapter;
use tokio::sync::broadcast;

#[async_trait::async_trait]
pub(super) trait ChainTransport: Send + Sync {
    async fn check(&self, cost: &StockChainCost) -> Result<(), String>;
    async fn wallet(&self, cost: &StockChainCost) -> Result<StockWalletEvidence, String>;
    fn sign(&self, cost: &StockChainCost) -> Result<String, String>;
    async fn send(&self, cost: &StockChainCost, signed: &str) -> Result<Option<String>, String>;
    async fn lookup(
        &self,
        cost: &StockChainCost,
        row: &StockChainSubmission,
    ) -> Result<chain::Lookup, String>;
}
pub(super) struct LiveChain;
#[async_trait::async_trait]
impl ChainTransport for LiveChain {
    async fn wallet(&self, c: &StockChainCost) -> Result<StockWalletEvidence, String> {
        crate::services::onchain_comparison::stock_inventory::read(&c.wallet_address, &c.mint).await
    }
    async fn check(&self, c: &StockChainCost) -> Result<(), String> {
        chain::recheck_original(c).await
    }
    fn sign(&self, c: &StockChainCost) -> Result<String, String> {
        chain_execution::sign(c)
    }
    async fn send(&self, c: &StockChainCost, signed: &str) -> Result<Option<String>, String> {
        chain::submit(c, signed).await
    }
    async fn lookup(
        &self,
        c: &StockChainCost,
        row: &StockChainSubmission,
    ) -> Result<chain::Lookup, String> {
        chain::lookup(c, row).await
    }
}

impl BackpackStocks {
    pub(crate) async fn execute_peer_plan(
        self: &Arc<Self>,
        request: StockPeerExecutionRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        self.execute_peer_owned(
            request,
            hub,
            Arc::new(LiveChain),
            Arc::new(move || submission::check_live(&trading.risk_config())),
        )
        .await
    }

    async fn execute_peer_owned(
        self: &Arc<Self>,
        request: StockPeerExecutionRequest,
        hub: realtime::WsHub,
        io: Arc<dyn ChainTransport>,
        check_live: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) -> Result<StockMarketSnapshot, String> {
        check_live()?;
        let original = self.peer_plan_store.get(&request.plan_id)?;
        let adapter = self.peer_execution_adapter(&original)?;
        self.peer_execution_previous(&request, &original)?;
        if original.phase == StockPeerPlanPhase::SubmissionUnknown {
            return Ok(self.snapshot());
        }
        let guard = self
            .submission_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "股票提交正在处理，不会另行排队")?;
        let service = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        // Keep the owner alive through caller cancellation; only the journal may
        // authorize the first send, and every subsequent request observes it.
        tokio::spawn(async move {
            let id = request.plan_id.clone();
            let result = tokio::time::timeout(
                Duration::from_secs(25),
                service.dispatch_peer_pair(request, &hub, adapter, io, check_live),
            )
            .await;
            let result = match result {
                Ok(r) => r,
                Err(_) => {
                    let _ = service.peer_plan_store.update(&id, |p| {
                        p.execution_problem =
                            Some("双边等待超时，只核对原交易，不重发或释放资金".into());
                        Ok(())
                    });
                    Err("双边等待超时，请核对原计划，没有自动重发".into())
                }
            };
            service.publish_plan(&hub);
            drop(guard);
            let _ = sender.send(result.map(|_| service.snapshot()));
        });
        receiver
            .await
            .map_err(|_| "股票任务中断，请核对原计划".to_owned())?
    }

    pub(super) fn peer_execution_adapter(
        &self,
        p: &StockPeerPlan,
    ) -> Result<Arc<dyn ExchangeAdapter>, String> {
        let a = self
            .peer_feed
            .as_ref()
            .and_then(|(agg, _)| agg.get(&p.request.selection.venue))
            .ok_or("原 Kraken 场所未接入")?;
        if a.stock_account_fingerprint().as_deref() != Some(&p.terms.account_fingerprint) {
            return Err("请恢复原计划的 Kraken 现货账户，不使用 Backpack 凭证".into());
        }
        Ok(a)
    }
    fn peer_execution_previous(
        &self,
        r: &StockPeerExecutionRequest,
        p: &StockPeerPlan,
    ) -> Result<(), String> {
        if !r.confirm_live {
            return Err("请确认本次双边实盘提交".into());
        }
        if p.phase == StockPeerPlanPhase::SubmissionUnknown {
            return Ok(());
        }
        let now = common::time::now_ms();
        if p.phase != StockPeerPlanPhase::Reserved
            || p.revision != r.revision
            || now < p.terms.created_at_ms
            || now >= p.terms.market_valid_until_ms
        {
            return Err("计划版本变化或原报价过期，没有提交".into());
        }
        Ok(())
    }

    async fn dispatch_peer_pair(
        self: &Arc<Self>,
        request: StockPeerExecutionRequest,
        hub: &realtime::WsHub,
        adapter: Arc<dyn ExchangeAdapter>,
        io: Arc<dyn ChainTransport>,
        check_live: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) -> Result<(), String> {
        let p = self.peer_plan_store.get(&request.plan_id)?;
        self.peer_execution_previous(&request, &p)?;
        if p.phase == StockPeerPlanPhase::SubmissionUnknown {
            return Ok(());
        }
        let rx = adapter
            .subscribe_stock_receipts()
            .map_err(|_| "Kraken 原订单回执通道不可用")?;
        let (ready, account, checked_chain) = tokio::join!(
            adapter.prepare_stock_submission(),
            adapter.stock_cash_account(&p.request.selection.native_symbol),
            io.check(&p.terms.basis.chain_cost)
        );
        ready.map_err(|_| "Kraken 实盘写入或私有 WS 未就绪，没有签名和提交")?;
        let account = account.map_err(|_| "Kraken 原账户余额或费率复核失败，没有提交")?;
        checked_chain?;
        if account.stock_taker_pct != p.terms.basis.account.stock_taker_pct
            || account.fx_taker_pct != p.terms.basis.account.fx_taker_pct
        {
            return Err("Kraken 账户费用已改变，请重新构建计划".into());
        }
        check_live()?;
        if !Arc::ptr_eq(&adapter, &self.peer_execution_adapter(&p)?) {
            return Err("提交准备期间 Kraken 账户发生变化".into());
        }
        self.peer_execution_previous(&request, &self.peer_plan_store.get(&p.plan_id)?)?;
        if !self.peer_feed_enabled(&p.request.selection) {
            return Err("Kraken 行情订阅已关闭，没有提交".into());
        }
        let signed = {
            let current = self.snapshot.read();
            if current.comparison.as_ref().is_none_or(|c| {
                c.asset != p.request.asset
                    || c.mint != p.terms.basis.comparison.mint
                    || c.keyed != p.request.keyed
            }) || current.security.as_ref() != Some(&p.terms.basis.security)
            {
                return Err("股票、合约或 Provider 模式已改变，请重新构建".into());
            }
            // The monitor may already show another quote or input. Execute only
            // this explicitly confirmed plan's saved message, never that new quote.
            let mut basis = p.terms.basis.clone();
            basis.account = account;
            basis.peer = current.peer.clone().ok_or("股票双边行情丢失")?;
            let checked = prepare_peer_plan_terms(
                &p.request,
                basis,
                p.terms.account_fingerprint.clone(),
                common::time::now_ms(),
            )?;
            let price = stock_exact_decimal(&checked.draft.limit_price)?;
            let approved = stock_exact_decimal(&p.terms.draft.limit_price)?;
            if checked.draft.quantity != p.terms.draft.quantity
                || if p.request.direction == StockChainDirection::Buy {
                    price < approved
                } else {
                    price > approved
                }
            {
                return Err("当前股票数量或价格不再满足原计划，没有提交".into());
            }
            io.sign(&p.terms.basis.chain_cost)?
        };
        check_live()?;
        if !Arc::ptr_eq(&adapter, &self.peer_execution_adapter(&p)?) {
            return Err("签名后账户发生变化，未提交".into());
        }
        let now = common::time::now_ms();
        let intent = chain::intent(&p.terms.basis.chain_cost, &signed, now)?;
        let (p, send) =
            self.peer_plan_store
                .begin(&request, &p.terms.account_fingerprint, intent, now)?;
        if !send {
            return Ok(());
        }
        self.start_peer_receipts(
            adapter.clone(),
            p.terms.account_fingerprint.clone(),
            rx,
            hub.clone(),
        );
        self.publish_plan(hub);
        let id = &p.plan_id;
        let original = p.cex_order.as_ref().ok_or("原订单未保存")?;
        let cex = async {
            let result = adapter
                .submit_stock_order(original.draft.clone(), original.client_order_id.clone())
                .await;
            let saved = match result {
                Ok(r) => self.peer_plan_store.receipt(id, &r),
                Err(_) => self.peer_plan_store.update(id, |p| {
                    p.execution_problem =
                        Some("Kraken 提交结果未明，等待原订单回执；不会重发".into());
                    Ok(())
                }),
            };
            self.publish_plan(hub);
            saved
        };
        let onchain = async {
            let result = io.send(&p.terms.basis.chain_cost, &signed).await;
            let saved = self.peer_plan_store.update(id, |p| {
                let r = p.chain_submission.as_mut().ok_or("链上原交易丢失")?;
                match result {
                    Ok(hint) => {
                        r.provider_acknowledged = true;
                        r.provider_transaction_id = hint;
                        r.problem = Some("Provider 已回复，等待链上最终回执".into());
                    }
                    Err(_) => r.problem = Some("链上提交结果未明，只核对原交易，不重发".into()),
                }
                Ok(())
            });
            self.publish_plan(hub);
            saved
        };
        let (cex, onchain) = tokio::join!(cex, onchain);
        cex?;
        onchain?;
        Ok(())
    }

    pub(crate) async fn recheck_peer_plan(
        self: &Arc<Self>,
        request: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.recheck_peer_with(&request.plan_id, hub, &LiveChain)
            .await
    }
    async fn recheck_peer_with(
        self: &Arc<Self>,
        id: &str,
        hub: &realtime::WsHub,
        io: &dyn ChainTransport,
    ) -> Result<StockMarketSnapshot, String> {
        let _owner = self
            .submission_lock
            .try_lock()
            .map_err(|_| "原双边提交正在处理，请稍后核对")?;
        let p = self.peer_plan_store.get(id)?;
        if p.phase == StockPeerPlanPhase::Settled {
            return Ok(self.snapshot());
        }
        let original = p.cex_order.as_ref().ok_or("股票计划尚未提交")?;
        // Chain receipts remain independently recoverable if venue credentials are unavailable.
        match self.peer_execution_adapter(&p) {
            Ok(adapter) => {
                let result = adapter
                    .track_stock_order(original.clone())
                    .and_then(|_| adapter.subscribe_stock_receipts());
                match result {
                    Ok(rx) => {
                        if let Some(row) = adapter.stock_order_receipt(&original.client_order_id) {
                            self.peer_plan_store.receipt(id, &row)?;
                        }
                        self.start_peer_receipts(
                            adapter.clone(),
                            p.terms.account_fingerprint.clone(),
                            rx,
                            hub.clone(),
                        );
                    }
                    Err(_) => {
                        self.peer_plan_store.update(id, |p| {
                            p.execution_problem =
                                Some("Kraken 回执订阅失败，保留原订单等待核对".into());
                            Ok(())
                        })?;
                    }
                }
                self.recover_peer_history(id, &adapter).await?;
            }
            Err(e) => {
                self.peer_plan_store.update(id, |p| {
                    p.execution_problem = Some(e);
                    Ok(())
                })?;
            }
        }
        let p = self.peer_plan_store.update(id, |p| {
            let r = p.chain_submission.as_mut().ok_or("链上原交易丢失")?;
            if r.receipt.is_some() {
                return Ok(());
            }
            let now = common::time::now_ms();
            if now < r.next_recheck_at_ms {
                return Err("原交易核对冷却中".into());
            }
            r.recheck_attempts = r.recheck_attempts.checked_add(1).ok_or("核对次数溢出")?;
            r.next_recheck_at_ms = now.saturating_add(5000);
            Ok(())
        })?;
        let row = p.chain_submission.as_ref().ok_or("链上原交易丢失")?;
        if row.receipt.is_none() {
            let result = io.lookup(&p.terms.basis.chain_cost, row).await;
            self.peer_plan_store.update(id, |p| {
                let r = p.chain_submission.as_mut().ok_or("链上原交易丢失")?;
                match result {
                    Ok(found) => {
                        r.search_before = found.before;
                        if let Some(receipt) = found.receipt {
                            r.transaction_id = Some(receipt.transaction_id.clone());
                            r.receipt = Some(receipt);
                            r.problem = None;
                            if p.cex_order.as_ref().is_some_and(|c| c.receipt_complete()) {
                                p.execution_problem = None;
                            }
                        } else {
                            r.problem = Some("原交易最终回执尚未找到，保留资金占用，不重发".into());
                        }
                    }
                    Err(_) => r.problem = Some("原链上交易查询失败，保留占用，稍后核对".into()),
                }
                Ok(())
            })?;
        }
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
}

mod history;
mod receipts;
#[cfg(test)]
pub(super) mod tests;
