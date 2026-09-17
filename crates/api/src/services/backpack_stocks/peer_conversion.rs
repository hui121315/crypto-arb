use super::*;
use exchange::ExchangeAdapter;

impl BackpackStocks {
    async fn peer_conversion_market(
        &self,
        p: &StockPeerPlan,
        a: &Arc<dyn ExchangeAdapter>,
    ) -> Result<(shared_types::VenueInstrument, StockPeerQuote), String> {
        if !self.peer_feed_enabled(&p.request.selection) {
            return Err("Kraken 现货 WS 订阅已关闭".into());
        }
        let (registry, market) = self.peer_sources.as_ref().ok_or("共享行情未接入")?;
        let symbol = format!("USDC/{}", p.terms.draft.quote_asset);
        if let Ok(Ok(exchange::PublicWsSnapshot::Ready(rows))) = tokio::time::timeout(
            Duration::from_millis(300),
            a.public_ws_spot_snapshot(std::slice::from_ref(&symbol)),
        )
        .await
        {
            market.store_spot_ticks(&rows, crate::services::market_data::MarketSource::WsPush);
        }
        let spec = registry
            .public_native_instrument("kraken", &symbol, shared_types::FeeProduct::Spot)
            .ok_or("官方换汇规格尚未就绪")?;
        let quote = peers::spot_quote(market, "kraken", &symbol, common::time::now_ms())
            .ok_or("换汇 WS 盘口尚未就绪，未使用固定汇率")?;
        Ok((spec, quote))
    }
    pub(crate) async fn prepare_peer_conversion(
        &self,
        r: StockPeerConversionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _lock = self
            .submission_lock
            .try_lock()
            .map_err(|_| "原交易或资金处置正在处理")?;
        let p = self.peer_plan_store.get(&r.plan_id)?;
        if p.conversions.last().is_some_and(|c| c.request == r) {
            return Ok(self.snapshot());
        }
        if p.revision != r.revision || !p.peer_conversion_available(common::time::now_ms()) {
            return Err("换汇版本变化或已有原计划待处理".into());
        }
        p.peer_conversion_gap()?;
        let adapter = self.peer_execution_adapter(&p)?;
        let account = adapter
            .stock_cash_account(&p.request.selection.native_symbol)
            .await
            .map_err(|_| "原 Kraken 账户余额与换汇费率读取失败")?;
        let (market, quote) = self.peer_conversion_market(&p, &adapter).await?;
        if !Arc::ptr_eq(&adapter, &self.peer_execution_adapter(&p)?) {
            return Err("换汇准备期间原账户变化".into());
        }
        let c =
            StockPeerConversion::compile(&p, r, market, quote, account, common::time::now_ms())?;
        self.peer_plan_store.update(&p.plan_id, |current| {
            if current.revision != p.revision {
                return Err("原记录已变化".into());
            }
            current.conversions.push(c);
            Ok(())
        })?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
    pub(crate) fn cancel_peer_conversion(
        &self,
        r: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.peer_plan_store.update(&r.plan_id, |p| {
            let c = p.conversions.get_mut(r.index).ok_or("换汇不存在")?;
            if c.cancelled_at_ms.is_some() {
                return Ok(());
            }
            if p.revision != r.revision || c.order.is_some() {
                return Err("换汇已变化或提交，不能取消".into());
            }
            c.cancelled_at_ms = Some(common::time::now_ms());
            Ok(())
        })?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
    pub(crate) async fn submit_peer_conversion(
        self: &Arc<Self>,
        r: StockPeerRecoverySubmitRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        self.submit_peer_conversion_with(
            r,
            hub,
            Arc::new(move || submission::check_live(&trading.risk_config())),
        )
        .await
    }
    async fn submit_peer_conversion_with(
        self: &Arc<Self>,
        r: StockPeerRecoverySubmitRequest,
        hub: realtime::WsHub,
        check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) -> Result<StockMarketSnapshot, String> {
        check()?;
        if !r.confirm_live {
            return Err("请确认本次真实换汇".into());
        }
        let p = self.peer_plan_store.get(&r.plan_id)?;
        if p.conversions
            .get(r.index)
            .ok_or("换汇不存在")?
            .order
            .is_some()
        {
            return Ok(self.snapshot());
        }
        let adapter = self.peer_execution_adapter(&p)?;
        let owner = self
            .submission_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "资金处置正在处理，不另行排队")?;
        let service = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(20), async {
                let p = service.peer_plan_store.get(&r.plan_id)?;
                let c = checked(&p, &r)?.clone();
                let events = adapter
                    .subscribe_stock_receipts()
                    .map_err(|_| "换汇私有回执通道未就绪")?;
                adapter
                    .prepare_stock_submission()
                    .await
                    .map_err(|_| "换汇 WS 交易通道未就绪")?;
                let account = adapter
                    .stock_cash_account(&p.request.selection.native_symbol)
                    .await
                    .map_err(|_| "原账户复核失败")?;
                let (market, quote) = service.peer_conversion_market(&p, &adapter).await?;
                let mut prefix = p.clone();
                prefix.conversions.truncate(r.index);
                c.check_current(&prefix, &market, &quote, &account, common::time::now_ms())?;
                check()?;
                if !Arc::ptr_eq(&adapter, &service.peer_execution_adapter(&p)?) {
                    return Err("原账户已改变，未提交".into());
                }
                let client = peer_plan_store::conversion::client_id(&p, r.index);
                let original = StockPeerOrderReceipt::pending(c.draft.clone(), client)?;
                original.kraken_submission("local-intent-check", 1, common::time::now_ms())?;
                service.peer_plan_store.update(&r.plan_id, |p| {
                    checked(p, &r)?;
                    p.conversions[r.index].order = Some(original.clone());
                    Ok(())
                })?;
                service.start_peer_receipts(
                    adapter.clone(),
                    p.terms.account_fingerprint.clone(),
                    events,
                    hub.clone(),
                );
                service.publish_plan(&hub);
                // Only this journal transition authorizes a new send. Timeout/restart never sends again.
                match adapter
                    .submit_stock_order(original.draft, original.client_order_id)
                    .await
                {
                    Ok(row) => {
                        service
                            .peer_plan_store
                            .conversion_receipt(&r.plan_id, r.index, &row)?;
                    }
                    Err(_) => {
                        service.peer_plan_store.update(&r.plan_id, |p| {
                            let o = p.conversions[r.index].order.as_mut().ok_or("原换汇丢失")?;
                            if !o.receipt_complete() && !o.evidence_conflict {
                                o.problem = Some("换汇提交结果未明，只核对原订单，不重发".into());
                            }
                            Ok(())
                        })?;
                    }
                }
                Ok::<_, String>(())
            })
            .await
            .unwrap_or_else(|_| Err("换汇等待超时，请核对原记录，没有自动重发".into()));
            service.publish_plan(&hub);
            drop(owner);
            let _ = tx.send(result.map(|_| service.snapshot()));
        });
        rx.await
            .map_err(|_| "换汇任务中断，请核对原记录".to_owned())?
    }
    pub(crate) async fn recheck_peer_conversion(
        self: &Arc<Self>,
        r: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _owner = self
            .submission_lock
            .try_lock()
            .map_err(|_| "资金处置正在处理")?;
        let p = self.peer_plan_store.get(&r.plan_id)?;
        let adapter = self.peer_execution_adapter(&p)?;
        let original = p
            .conversions
            .get(r.index)
            .and_then(|c| c.order.as_ref())
            .ok_or("换汇尚未提交")?;
        adapter
            .track_stock_order(original.clone())
            .map_err(|_| "换汇原订单恢复失败")?;
        if let Some(o) = adapter.stock_order_receipt(&original.client_order_id) {
            self.peer_plan_store
                .conversion_receipt(&p.plan_id, r.index, &o)?;
        }
        if let Ok(rx) = adapter.subscribe_stock_receipts() {
            self.start_peer_receipts(
                adapter.clone(),
                p.terms.account_fingerprint,
                rx,
                hub.clone(),
            );
        }
        self.recover_peer_conversion(&r.plan_id, r.index, &adapter)
            .await?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
    pub(super) async fn recover_peer_conversion(
        &self,
        id: &str,
        index: usize,
        adapter: &Arc<dyn ExchangeAdapter>,
    ) -> Result<(), String> {
        let p = self.peer_plan_store.get(id)?;
        let c = p.conversions.get(index).ok_or("换汇不存在")?;
        let o = c.order.as_ref().ok_or("换汇尚未提交")?;
        let now = common::time::now_ms();
        if o.receipt_complete() || now < c.history.next_check_at_ms {
            return Ok(());
        }
        self.peer_plan_store.update(id, |p| {
            let h = &mut p.conversions[index].history;
            h.attempts = h.attempts.checked_add(1).ok_or("次数溢出")?;
            h.next_check_at_ms =
                now.saturating_add((15_000 * i64::from(h.attempts.min(4))).min(60_000));
            h.problem = Some("已登记原换汇历史查询，不重发订单".into());
            Ok(())
        })?;
        let found =
            tokio::time::timeout(Duration::from_secs(12), adapter.reconcile_stock_order(o)).await;
        if !Arc::ptr_eq(adapter, &self.peer_execution_adapter(&p)?) {
            return Err("换汇查询期间原账户变化".into());
        }
        if let Ok(Ok(Some(row))) = found {
            self.peer_plan_store.conversion_receipt(id, index, &row)?;
        }
        self.peer_plan_store.update(id, |p| {
            let c = &mut p.conversions[index];
            c.history.checked_at_ms = Some(common::time::now_ms());
            c.history.problem = if c.order.as_ref().is_some_and(|r| r.receipt_complete()) {
                None
            } else {
                Some("原换汇终态或费用未核齐，保留占用，只核对原订单".into())
            };
            Ok(())
        })?;
        Ok(())
    }
}
fn checked<'a>(
    p: &'a StockPeerPlan,
    r: &StockPeerRecoverySubmitRequest,
) -> Result<&'a StockPeerConversion, String> {
    let c = p.conversions.get(r.index).ok_or("换汇不存在")?;
    if !r.confirm_live
        || p.revision != r.revision
        || r.index + 1 != p.conversions.len()
        || c.cancelled_at_ms.is_some()
        || c.order.is_some()
        || common::time::now_ms() >= c.valid_until_ms
    {
        return Err("换汇未确认、版本变化、取消、过期或已经提交".into());
    }
    Ok(c)
}

#[cfg(test)]
mod tests;
