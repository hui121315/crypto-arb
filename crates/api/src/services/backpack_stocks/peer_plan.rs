use super::*;
use crate::services::onchain_comparison::{stock_costs, stock_inventory};
use std::future::Future;

impl BackpackStocks {
    pub(crate) fn with_peer_plan_store(mut self, path: std::path::PathBuf) -> Self {
        self.peer_plan_store = peer_plan_store::Store::load(Some(path), self.wallet_claims.clone());
        self
    }

    pub(crate) async fn build_peer_plan(
        self: &Arc<Self>,
        request: StockPeerPlanRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.build_peer_plan_with(
            request,
            hub,
            |service, request, generation, adapter| async move {
                service
                    .refresh_build_mint(generation, &request.asset)
                    .await?;
                let c = bind(&request, &service.snapshot())?;
                let account = tokio::time::timeout(
                    Duration::from_secs(12),
                    adapter.stock_cash_account(&request.selection.native_symbol),
                )
                .await
                .map_err(|_| "Kraken 股票账户读取超时")?
                .map_err(|_| "Kraken 股票账户读取失败，请检查现货 API 读取权限")?;
                let wallet = tokio::time::timeout(
                    Duration::from_secs(8),
                    stock_inventory::read(&request.wallet_address, &c.mint),
                )
                .await
                .map_err(|_| "Solana 钱包读取超时")??;
                Ok((account, wallet))
            },
            |request, comparison| async move { stock_costs::read(&request, &comparison).await },
        )
        .await
    }

    pub(super) async fn build_peer_plan_with<I, IF, C, CF>(
        self: &Arc<Self>,
        mut request: StockPeerPlanRequest,
        hub: &realtime::WsHub,
        inputs: I,
        cost: C,
    ) -> Result<StockMarketSnapshot, String>
    where
        I: FnOnce(Arc<Self>, StockPeerPlanRequest, u64, Arc<dyn exchange::ExchangeAdapter>) -> IF,
        IF: Future<Output = Result<(StockPeerAccount, StockWalletEvidence), String>>,
        C: FnOnce(StockChainCostRequest, StockComparison) -> CF,
        CF: Future<Output = Result<StockChainCost, String>>,
    {
        request.wallet_address = request.wallet_address.trim().into();
        request.validate()?;
        stock_inventory::validate_owner(&request.wallet_address)?;
        let _preflight = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票预检或构建正在进行")?;
        let _quote = self
            .quote_lock
            .try_lock()
            .map_err(|_| "链上报价正在更新，请稍后构建")?;
        // Read the durable original before any venue/RPC request, including after
        // cancellation, expiry, credential replacement or restart.
        if self.peer_plan_store.previous(&request)?.is_some() {
            self.publish_plan(hub);
            return Ok(self.snapshot());
        }
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        bind(&request, &self.snapshot())?;
        if !self.peer_feed_enabled(&request.selection) {
            return Err("Kraken 行情订阅已关闭".into());
        }
        self.wallet_claims
            .check("solana", &request.wallet_address, common::time::now_ms())?;
        let (aggregator, _) = self.peer_feed.as_ref().ok_or("股票场所接入未配置")?;
        let adapter = aggregator
            .get(&request.selection.venue)
            .ok_or("Kraken 未接入")?;
        let fingerprint = adapter
            .stock_account_fingerprint()
            .ok_or("请配置 Kraken 现货账户，不使用 Backpack 凭证")?;
        let unchanged_account = || {
            if aggregator
                .get(&request.selection.venue)
                .is_none_or(|a| !Arc::ptr_eq(&adapter, &a))
                || adapter.stock_account_fingerprint().as_deref() != Some(&fingerprint)
            {
                Err("构建期间账户配置发生变化，未预留资金".to_string())
            } else {
                Ok(())
            }
        };
        let (account, wallet) =
            inputs(self.clone(), request.clone(), generation, adapter.clone()).await?;
        self.ensure_generation(generation, &request.asset)?;
        unchanged_account()?;
        let baseline = bind(&request, &self.snapshot())?;
        let c = cost(
            StockChainCostRequest {
                asset: request.asset.clone(),
                direction: request.direction,
                wallet_address: request.wallet_address.clone(),
            },
            baseline.clone(),
        )
        .await?;
        self.ensure_generation(generation, &request.asset)?;
        unchanged_account()?;
        self.refresh_peer(common::time::now_ms());
        let mut s = self.snapshot.write();
        if self.generation.load(Ordering::SeqCst) != generation
            || s.comparison.as_ref() != Some(&baseline)
        {
            return Err("股票或原投入已改变，旧构建结果已丢弃".into());
        }
        bind(&request, &s)?;
        let mut reviewed = s.clone();
        chain_cost::apply(&mut reviewed, c.clone())?;
        let now = common::time::now_ms();
        let basis = StockPeerPlanBasis {
            security: reviewed.security.clone().ok_or("股票身份丢失")?,
            tokens: reviewed.tokens.clone(),
            token_metadata_at_ms: reviewed.token_metadata_at_ms,
            comparison: reviewed.comparison.clone().ok_or("链上报价丢失")?,
            peer: reviewed.peer.clone().ok_or("股票市场丢失")?,
            account: account.clone(),
            wallet: wallet.clone(),
            chain_cost: c,
        };
        let terms = prepare_peer_plan_terms(&request, basis, fingerprint, now)?;
        let report = StockPeerPreflight {
            asset: request.asset.clone(),
            selection: request.selection.clone(),
            checked_at_ms: now,
            account: Some(account),
            wallet: Some(wallet),
            problems: vec![],
        };
        self.peer_plan_store
            .reserve(request, terms, common::time::now_ms())?;
        reviewed.peer_preflight = Some(report);
        *s = reviewed;
        drop(s);
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(crate) fn cancel_peer_plan(
        &self,
        request: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.peer_plan_store
            .cancel(&request, common::time::now_ms())?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }
}

fn bind(
    request: &StockPeerPlanRequest,
    s: &StockMarketSnapshot,
) -> Result<StockComparison, String> {
    let c = s.comparison.as_ref().ok_or("请先读取双向链上报价")?;
    if s.peer
        .as_ref()
        .is_none_or(|p| p.selection != request.selection)
        || c.asset != request.asset
        || c.keyed != request.keyed
        || request
            .direction
            .quote(c)
            .is_none_or(|q| q.input_raw != request.input_raw)
    {
        return Err("股票、对比市场、Provider 或原投入发生变化，请重新构建".into());
    }
    Ok(c.clone())
}

#[cfg(test)]
pub(super) mod tests;
