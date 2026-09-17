use super::*;
use crate::services::onchain_comparison::{stock_costs, stock_inventory, stock_quotes};
use std::future::Future;

impl BackpackStocks {
    pub(crate) async fn build_plan(
        self: &Arc<Self>,
        request: StockPlanBuildRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let input_hub = hub.clone();
        self.build_plan_with(
            request,
            hub,
            move |service, request, generation| async move {
                service
                    .refresh_build_mint(generation, &request.asset)
                    .await?;
                service
                    .read_preflight_inputs(&request, generation, &input_hub)
                    .await
            },
            |request, comparison| async move { stock_costs::read(&request, &comparison).await },
        )
        .await
    }

    async fn build_plan_with<I, IFut, C, CFut>(
        self: &Arc<Self>,
        mut request: StockPlanBuildRequest,
        hub: &realtime::WsHub,
        inputs: I,
        cost: C,
    ) -> Result<StockMarketSnapshot, String>
    where
        I: FnOnce(Arc<Self>, StockPreflightRequest, u64) -> IFut,
        IFut: Future<Output = Result<preflight::Inputs, String>>,
        C: FnOnce(StockChainCostRequest, StockComparison) -> CFut,
        CFut: Future<Output = Result<StockChainCost, String>>,
    {
        request.wallet_address = request.wallet_address.trim().to_owned();
        stock_inventory::validate_owner(&request.wallet_address)?;
        if !(16..=128).contains(&request.request_id.len())
            || !request
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || request
                .input_raw
                .parse::<u64>()
                .ok()
                .is_none_or(|n| n == 0 || n.to_string() != request.input_raw)
        {
            return Err("股票构建请求标识或原始数量无效".into());
        }
        let fingerprint = (self.credential_loader)()?.fingerprint();
        let _preflight = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票计划正在构建，请等待当前结果")?;
        let _quote = self
            .quote_lock
            .try_lock()
            .map_err(|_| "股票询价进行中，请等待完整结果")?;
        if self
            .plan_store
            .previous_build(&request, &fingerprint)?
            .is_some()
        {
            return Ok(self.snapshot());
        }
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        bind_input(&request, &self.snapshot())?;
        self.wallet_claims
            .check("solana", &request.wallet_address, common::time::now_ms())?;
        if self
            .plan_store
            .records()
            .iter()
            .any(|p| p.holds_funds(common::time::now_ms()))
        {
            return Err("已有股票计划占用资金，请先核对或取消原计划".into());
        }
        let preflight_request = StockPreflightRequest {
            asset: request.asset.clone(),
            wallet_address: Some(request.wallet_address.clone()),
        };
        // Warm slow account/RPC reads before requesting the short-lived taker transaction.
        let inputs = inputs(self.clone(), preflight_request.clone(), generation).await?;
        self.ensure_generation(generation, &request.asset)?;
        if inputs.fingerprint.as_deref() != Some(&fingerprint)
            || (self.credential_loader)()?.fingerprint() != fingerprint
        {
            return Err("构建期间账户凭证变化或读取失败，未预留资金".into());
        }
        if !inputs.problems.is_empty()
            || inputs
                .wallet
                .as_ref()
                .is_none_or(|w| !w.problems.is_empty())
        {
            let problem = inputs
                .problems
                .first()
                .cloned()
                .unwrap_or_else(|| "钱包库存未核实".into());
            {
                let account = self.account.read();
                let mut snapshot = self.snapshot.write();
                if self.generation.load(Ordering::SeqCst) != generation {
                    return Err("股票已切换，旧库存结果已丢弃".into());
                }
                snapshot.preflight = Some(preflight::report(
                    &preflight_request,
                    &snapshot,
                    account.evidence.as_ref(),
                    inputs,
                    common::time::now_ms(),
                ));
            }
            self.publish_plan(hub);
            return Err(problem);
        }
        let baseline = bind_input(&request, &self.snapshot())?;
        let cost = cost(
            StockChainCostRequest {
                asset: request.asset.clone(),
                direction: request.direction,
                wallet_address: request.wallet_address.clone(),
            },
            baseline.clone(),
        )
        .await?;
        self.ensure_generation(generation, &request.asset)?;
        if (self.credential_loader)()?.fingerprint() != fingerprint {
            return Err("构建期间账户凭证变化，未预留资金".into());
        }
        let result =
            self.finish_plan_build(generation, request, baseline, inputs, cost, &fingerprint);
        self.publish_plan(hub);
        result?;
        Ok(self.snapshot())
    }

    pub(super) async fn refresh_build_mint(&self, generation: u64, asset: &str) -> Result<(), String> {
        self.ensure_generation(generation, asset)?;
        let snapshot = self.snapshot();
        let (address, _, decimals) = comparison::issuer(&snapshot)?;
        let baseline = snapshot.comparison.as_ref().ok_or("请先取得双向链上报价")?;
        let now = common::time::now_ms();
        if baseline.mint.address == address
            && baseline.mint.decimals == decimals
            && now >= baseline.mint.checked_at_ms
            && now - baseline.mint.checked_at_ms < 30_000
            && baseline.mint.next_change_at_ms.is_none_or(|t| now < t)
        {
            return Ok(());
        }
        let mint = stock_quotes::mint(address, decimals).await?;
        let mut snapshot = self.snapshot.write();
        if self.generation.load(Ordering::SeqCst) != generation
            || snapshot.comparison.as_ref() != Some(baseline)
        {
            return Err("股票已切换，合约读取结果已丢弃".into());
        }
        snapshot.comparison.as_mut().ok_or("股票报价已丢失")?.mint = mint;
        snapshot.preflight = None;
        snapshot.chain_costs.clear();
        Ok(())
    }

    fn finish_plan_build(
        &self,
        generation: u64,
        request: StockPlanBuildRequest,
        baseline: StockComparison,
        inputs: preflight::Inputs,
        cost: StockChainCost,
        fingerprint: &str,
    ) -> Result<(), String> {
        let _rfq = self.rfq_state_lock.lock();
        let account = self.account.read();
        let evidence = account
            .evidence
            .as_ref()
            .filter(|a| a.fingerprint == fingerprint)
            .ok_or("当前账户已失效，请重新构建")?;
        let mut current = self.snapshot.write();
        if self.generation.load(Ordering::SeqCst) != generation
            || current
                .security
                .as_ref()
                .is_none_or(|s| s.asset != request.asset)
            || current.comparison.as_ref() != Some(&baseline)
        {
            return Err("股票或询价参数已变化，旧构建已丢弃".into());
        }
        let seed = request
            .direction
            .quote(&baseline)
            .ok_or("原方向报价已丢失")?;
        if cost.asset != request.asset
            || cost.direction != request.direction
            || cost.wallet_address != request.wallet_address
            || cost.mint != baseline.mint
            || cost.quote.input_raw != request.input_raw
            || cost.quote.input_mint != seed.input_mint
            || cost.quote.output_mint != seed.output_mint
        {
            return Err("费用交易与所选股票、方向、钱包或金额不符，未预留资金".into());
        }
        let mut snapshot = current.clone();
        chain_cost::apply(&mut snapshot, cost)?;
        snapshot.rfqs = self.visible_rfqs();
        snapshot.rfq_connected = self.rfq_subscription.borrow().as_deref() == Some(fingerprint);
        snapshot.rfq_problem = self
            .rfq_store
            .problem()
            .or_else(|| self.rfq_problem.read().clone());
        let now = common::time::now_ms();
        let preflight_request = StockPreflightRequest {
            asset: request.asset.clone(),
            wallet_address: Some(request.wallet_address.clone()),
        };
        snapshot.preflight = Some(preflight::report(
            &preflight_request,
            &snapshot,
            Some(evidence),
            inputs,
            now,
        ));
        let plan = plans::prepare(
            StockPlanRequest {
                request_id: request.request_id.clone(),
                asset: request.asset.clone(),
                direction: request.direction,
                wallet_address: request.wallet_address.clone(),
                preflight_at_ms: now,
                build: Some(request),
            },
            &snapshot,
            evidence,
            now,
        )
        .and_then(|p| self.plan_store.reserve(p, now));
        // Retain useful rejection evidence, but a reservation is durable only after all checks pass.
        snapshot.observed_at_ms = now.max(snapshot.observed_at_ms.saturating_add(1));
        *current = snapshot;
        plan.map(|_| ())
    }
}

fn bind_input(
    request: &StockPlanBuildRequest,
    snapshot: &StockMarketSnapshot,
) -> Result<StockComparison, String> {
    let comparison = snapshot
        .comparison
        .as_ref()
        .ok_or("先选择股票并取得链上比较报价")?;
    if comparison.asset != request.asset
        || comparison.keyed != request.keyed
        || request
            .direction
            .quote(comparison)
            .is_none_or(|q| q.input_raw != request.input_raw)
    {
        return Err("所选股票、Provider 或金额已变化，请按当前参数重新构建".into());
    }
    Ok(comparison.clone())
}

#[cfg(test)]
mod tests;
