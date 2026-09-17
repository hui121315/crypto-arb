use super::*;
use crate::services::onchain_comparison::{stock_costs, stock_inventory, stock_quotes};
mod execution;
mod native_topup;

impl BackpackStocks {
    pub(crate) fn with_stablecoin_store(mut self, path: std::path::PathBuf) -> Self {
        self.stablecoin_store =
            stablecoin_store::StablecoinStore::load(Some(path), self.wallet_claims.clone());
        self
    }

    pub(crate) fn build_stablecoin_plan(
        &self,
        request: StockStablecoinPlanRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        stablecoin_store::validate_request(&request)?;
        if self.stablecoin_store.previous(&request)?.is_some() {
            return Ok(self.snapshot());
        }
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "另一项股票预检进行中")?;
        let (generation, preview) = self
            .stablecoin_preview
            .read()
            .clone()
            .ok_or("请先试算兑换，再保存原报价")?;
        self.ensure_generation(generation, &request.conversion.asset)?;
        if preview.request != request.conversion
            || preview.checked_at_ms != request.preview_at_ms
            || preview
                .cost
                .as_ref()
                .is_none_or(|c| c.transaction_fingerprint != request.transaction_fingerprint)
        {
            return Err("兑换输入或报价已变化，请重新试算；未预留资金".into());
        }
        {
            let snapshot = self.snapshot.read();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err("股票已切换，未保存旧兑换计划".into());
            }
            comparison::issuer(&snapshot)?;
            self.stablecoin_store
                .reserve(request, preview, common::time::now_ms())?;
        }
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(crate) fn cancel_stablecoin_plan(
        &self,
        request: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.stablecoin_store
            .cancel(&request, common::time::now_ms())?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(crate) async fn preview_stablecoin(
        &self,
        request: StockStablecoinRequest,
    ) -> Result<StockStablecoinPreview, String> {
        self.preview_stablecoin_with(request, |r, raw| {
            Box::pin(async move {
                let mint = stock_quotes::mint(STOCK_SOLANA_USDT, 6).await?;
                validate_mint(&mint)?;
                let wallet = stock_inventory::read(&r.wallet_address, &mint).await?;
                let quote = stock_quotes::jupiter(
                    r.keyed,
                    STOCK_SOLANA_USDT,
                    shared_types::stocks::comparison::SOLANA_USDC,
                    &raw.to_string(),
                )
                .await?;
                let request = StockChainCostRequest {
                    asset: "USDT".into(),
                    direction: StockChainDirection::Sell,
                    wallet_address: r.wallet_address.clone(),
                };
                let (cost, problems) =
                    match stock_costs::read_token_swap(&request, r.keyed, &mint, &quote).await {
                        Ok(cost) => (Some(cost), vec![]),
                        Err(e) => (None, vec![e]),
                    };
                // A wallet-specific build can change the quote; amounts and costs travel together.
                let quote = cost.as_ref().map_or(quote, |c| c.quote.clone());
                Ok((wallet, quote, cost, problems))
            })
        })
        .await
    }

    async fn preview_stablecoin_with<F>(
        &self,
        mut request: StockStablecoinRequest,
        read: F,
    ) -> Result<StockStablecoinPreview, String>
    where
        F: for<'a> FnOnce(
            &'a StockStablecoinRequest,
            u64,
        ) -> futures::future::BoxFuture<
            'a,
            Result<
                (
                    StockWalletEvidence,
                    StockDexQuote,
                    Option<StockChainCost>,
                    Vec<String>,
                ),
                String,
            >,
        >,
    {
        request.wallet_address = request.wallet_address.trim().into();
        request.input_usdt = request.input_usdt.trim().into();
        request.target_usdc = request.target_usdc.trim().into();
        let (raw, _) = request.amounts_raw()?;
        stock_inventory::validate_owner(&request.wallet_address)?;
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "另一项股票预检进行中")?;
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        let before = self.snapshot.read().security.clone();
        comparison::issuer(&self.snapshot())?;
        let (wallet, quote, cost, mut problems) = read(&request, raw).await?;
        self.ensure_generation(generation, &request.asset)?;
        if self.snapshot.read().security != before {
            return Err("股票身份已变化，旧兑换试算已丢弃".into());
        }
        let now = common::time::now_ms();
        if let Err(problem) = self
            .wallet_claims
            .check("solana", &request.wallet_address, now)
        {
            problems.push(format!("钱包已有其他资金计划占用：{problem}"));
        }
        let preview = stablecoin_preview(request, wallet, quote, cost, problems, now)?;
        *self.stablecoin_preview.write() = Some((generation, preview.clone()));
        Ok(preview)
    }
}

fn validate_mint(mint: &StockMintEvidence) -> Result<(), String> {
    if mint.address != STOCK_SOLANA_USDT
        || mint.decimals != 6
        || mint.ui_multiplier != "1"
        || !mint.extensions.is_empty()
        || mint.next_change_at_ms.is_some()
    {
        return Err("USDT 合约、精度或代币扩展与官方普通 SPL 资产不一致".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
