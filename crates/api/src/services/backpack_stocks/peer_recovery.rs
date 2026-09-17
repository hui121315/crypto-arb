use super::peer_execution::{ChainTransport, LiveChain};
use super::*;
use crate::services::onchain_comparison::stock_costs::execution as chain;
use crate::services::onchain_comparison::{stock_costs, stock_inventory, stock_quotes};
use rust_decimal::Decimal;

impl BackpackStocks {
    pub(crate) async fn prepare_peer_recovery(
        self: &Arc<Self>,
        r: StockPeerRecoveryRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let service = self.clone();
        self.prepare_peer_recovery_with(r, hub, move |p, limit| async move {
            let security = service
                .catalog()
                .await?
                .rows
                .into_iter()
                .find(|s| s.asset == p.request.asset)
                .ok_or("原官方证券已不可用")?;
            let tokens =
                protocol::tokens(&service.read("/api/v1/assets").await?, &p.request.asset)?;
            let snapshot = StockMarketSnapshot {
                security: Some(security),
                tokens,
                ..Default::default()
            };
            let (address, _, decimals) = comparison::issuer(&snapshot)?;
            if address != p.terms.basis.chain_cost.mint.address
                || decimals != p.terms.basis.chain_cost.mint.decimals
            {
                return Err("官方股票合约映射变化，需核对后再补偿".into());
            }
            let mint =
                stock_quotes::mint_at_min_slot(&address, decimals, p.peer_minimum_slot()?).await?;
            if stock_exact_decimal(&mint.ui_multiplier)?
                != stock_exact_decimal(&p.terms.basis.chain_cost.mint.ui_multiplier)?
            {
                return Err("股票份额倍率已变化，请先核对公司行为".into());
            }
            let wallet = stock_inventory::read(&p.request.wallet_address, &mint).await?;
            let cost = quote_recovery(&p, &mint, &limit).await?;
            Ok((wallet, cost))
        })
        .await
    }

    async fn prepare_peer_recovery_with<F, Fut>(
        &self,
        r: StockPeerRecoveryRequest,
        hub: &realtime::WsHub,
        read: F,
    ) -> Result<StockMarketSnapshot, String>
    where
        F: FnOnce(StockPeerPlan, String) -> Fut,
        Fut: std::future::Future<Output = Result<(StockWalletEvidence, StockChainCost), String>>,
    {
        let _owner = self
            .submission_lock
            .try_lock()
            .map_err(|_| "股票原交易或补偿正在处理")?;
        let p = self.peer_plan_store.get(&r.plan_id)?;
        if p.recoveries
            .last()
            .is_some_and(|old| old.source_revision == r.revision && old.usdc_limit == r.usdc_limit)
        {
            return Ok(self.snapshot());
        }
        if p.revision != r.revision
            || stock_peer_recovery_limit(&r.usdc_limit).is_none()
            || !p.peer_recovery_available(common::time::now_ms())
        {
            return Err("计划版本变化、限额无效或已有补偿待处理".into());
        }
        let adapter = self.peer_execution_adapter(&p)?;
        let target = p.peer_recovery_target()?;
        let (wallet, cost) = read(p.clone(), r.usdc_limit.clone()).await?;
        if !Arc::ptr_eq(&adapter, &self.peer_execution_adapter(&p)?) {
            return Err("补偿准备期间原账户已改变".into());
        }
        let row = StockPeerRecovery {
            source_revision: p.revision,
            prepared_at_ms: common::time::now_ms(),
            usdc_limit: r.usdc_limit,
            target,
            cost,
            wallet,
            cancelled_at_ms: None,
            submission: None,
        };
        self.peer_plan_store.prepare_recovery(&p.plan_id, row)?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(crate) fn cancel_peer_recovery(
        &self,
        r: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.peer_plan_store.cancel_recovery(&r)?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(crate) async fn submit_peer_recovery(
        self: &Arc<Self>,
        r: StockPeerRecoverySubmitRequest,
        hub: realtime::WsHub,
        trading: Arc<crate::trading_service::TradingService>,
    ) -> Result<StockMarketSnapshot, String> {
        self.submit_peer_recovery_with(
            r,
            hub,
            Arc::new(LiveChain),
            Arc::new(move || submission::check_live(&trading.risk_config())),
        )
        .await
    }
    async fn submit_peer_recovery_with(
        self: &Arc<Self>,
        r: StockPeerRecoverySubmitRequest,
        hub: realtime::WsHub,
        io: Arc<dyn ChainTransport>,
        check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) -> Result<StockMarketSnapshot, String> {
        check()?;
        if !r.confirm_live {
            return Err("请确认本次真实补偿交易".into());
        }
        let p = self.peer_plan_store.get(&r.plan_id)?;
        let row = p.recoveries.get(r.index).ok_or("补偿不存在")?;
        if row.submission.is_some() {
            return Ok(self.snapshot());
        }
        let adapter = self.peer_execution_adapter(&p)?;
        let owner = self
            .submission_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| "原交易或补偿正在处理，不会另行排队")?;
        let service = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(25), async {
                let p = service.peer_plan_store.get(&r.plan_id)?;
                let row = checked_row(&p, &r)?;
                io.check(&row.cost).await?;
                let mut refreshed = row.clone();
                refreshed.wallet = io.wallet(&row.cost).await?;
                let mut prefix = p.clone();
                prefix.recoveries.truncate(r.index);
                refreshed.validate(&prefix, common::time::now_ms())?;
                check()?;
                if !Arc::ptr_eq(&adapter, &service.peer_execution_adapter(&p)?) {
                    return Err("签名前原账户已改变".into());
                }
                checked_row(&service.peer_plan_store.get(&r.plan_id)?, &r)?;
                let signed = io.sign(&row.cost)?;
                check()?;
                if !Arc::ptr_eq(&adapter, &service.peer_execution_adapter(&p)?) {
                    return Err("签名后原账户已改变，未发送".into());
                }
                // Durable ownership is established before the only transport call.
                service.peer_plan_store.begin_recovery(&r, &signed)?;
                service.publish_plan(&hub);
                let result = io.send(&row.cost, &signed).await;
                service
                    .peer_plan_store
                    .change_recovery(&r.plan_id, r.index, |s| {
                        match result {
                            Ok(hint) => {
                                s.provider_acknowledged = true;
                                s.provider_transaction_id = hint;
                                s.problem = Some("补偿已回复，等待原交易最终回执".into());
                            }
                            Err(_) => {
                                s.problem = Some("补偿提交结果未明，只核对原交易，不重发".into())
                            }
                        };
                        Ok(())
                    })?;
                Ok::<_, String>(())
            })
            .await
            .unwrap_or_else(|_| Err("补偿处理超时，请核对原补偿记录；没有自动重发".into()));
            service.publish_plan(&hub);
            drop(owner);
            let _ = tx.send(result.map(|_| service.snapshot()));
        });
        rx.await
            .map_err(|_| "补偿任务中断，请核对原记录".to_owned())?
    }

    pub(crate) async fn recheck_peer_recovery(
        &self,
        r: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.recheck_peer_recovery_with(r, hub, &LiveChain).await
    }
    async fn recheck_peer_recovery_with(
        &self,
        r: StockRecoveryActionRequest,
        hub: &realtime::WsHub,
        io: &dyn ChainTransport,
    ) -> Result<StockMarketSnapshot, String> {
        let _owner = self
            .submission_lock
            .try_lock()
            .map_err(|_| "原交易或补偿正在处理")?;
        let now = common::time::now_ms();
        let p = self
            .peer_plan_store
            .change_recovery(&r.plan_id, r.index, |s| {
                if s.receipt.is_some() {
                    return Ok(());
                }
                if now < s.next_recheck_at_ms {
                    return Err("原补偿交易核对冷却中".into());
                }
                s.recheck_attempts = s.recheck_attempts.checked_add(1).ok_or("核对次数溢出")?;
                s.next_recheck_at_ms = now.saturating_add(5000);
                Ok(())
            })?;
        let row = &p.recoveries[r.index];
        let submitted = row.submission.as_ref().ok_or("补偿尚未提交")?;
        if submitted.receipt.is_none() {
            let result = io.lookup(&row.cost, submitted).await;
            self.peer_plan_store
                .change_recovery(&r.plan_id, r.index, |s| {
                    match result {
                        Ok(found) => {
                            s.search_before = found.before;
                            if let Some(receipt) = found.receipt {
                                s.transaction_id = Some(receipt.transaction_id.clone());
                                s.receipt = Some(receipt);
                                s.problem = None;
                            } else {
                                s.problem =
                                    Some("原补偿最终回执尚未找到，保持占用，不重复发送".into());
                            }
                        }
                        Err(_) => s.problem = Some("原补偿查询失败，保留占用，稍后核对".into()),
                    };
                    Ok(())
                })?;
        }
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
}

fn checked_row<'a>(
    p: &'a StockPeerPlan,
    r: &StockPeerRecoverySubmitRequest,
) -> Result<&'a StockPeerRecovery, String> {
    let row = p.recoveries.get(r.index).ok_or("补偿不存在")?;
    if !r.confirm_live
        || p.revision != r.revision
        || r.index + 1 != p.recoveries.len()
        || row.cancelled_at_ms.is_some()
        || row.submission.is_some()
    {
        return Err("本次补偿未确认、已变化、已取消或已提交".into());
    }
    let mut prefix = p.clone();
    prefix.recoveries.truncate(r.index);
    row.validate(&prefix, common::time::now_ms())?;
    chain::validate_artifact(&row.cost)?;
    Ok(row)
}

async fn quote_recovery(
    p: &StockPeerPlan,
    mint: &StockMintEvidence,
    limit: &str,
) -> Result<StockChainCost, String> {
    let target = p.peer_recovery_target()?;
    let raw = target
        .stock_raw
        .parse::<u64>()
        .map_err(|_| "补偿数量无效")?;
    let cap = stock_peer_recovery_limit(limit).ok_or("USDC 限额无效")?;
    let buying = target.direction == StockChainDirection::Buy;
    let seed_input = if buying {
        (cap.min(Decimal::ONE) * Decimal::from(1_000_000))
            .normalize()
            .to_string()
    } else {
        target.stock_raw.clone()
    };
    let mut seed = stock_quotes::jupiter(
        p.request.keyed,
        if buying {
            shared_types::stocks::comparison::SOLANA_USDC
        } else {
            &mint.address
        },
        if buying {
            &mint.address
        } else {
            shared_types::stocks::comparison::SOLANA_USDC
        },
        &seed_input,
    )
    .await?;
    for _ in 0..3 {
        if buying {
            seed.input_raw =
                recovery::next_buy_input(&seed.input_raw, &seed.minimum_output_raw, raw, cap)?
                    .to_string();
        }
        let c = stock_costs::read_token_swap(
            &StockChainCostRequest {
                asset: p.request.asset.clone(),
                wallet_address: p.request.wallet_address.clone(),
                direction: target.direction,
            },
            p.request.keyed,
            mint,
            &seed,
        )
        .await?;
        if !buying
            || c.quote
                .minimum_output_raw
                .parse::<u64>()
                .is_ok_and(|n| n >= raw)
        {
            return Ok(c);
        }
        seed = c.quote;
    }
    Err("三次新报价仍不能补足股票，未保存或发送".into())
}

#[cfg(test)]
pub(super) mod tests;
