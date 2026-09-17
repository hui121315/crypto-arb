use super::*;
use serde_json::json;

impl BackpackStocks {
    pub(crate) fn with_funding_store(mut self, path: std::path::PathBuf) -> Self {
        self.funding_store =
            funding_store::FundingStore::load(Some(path), self.wallet_claims.clone());
        self
    }

    pub(crate) async fn build_funding_plan(
        self: &Arc<Self>,
        request: StockFundingPlanRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let input_hub = hub.clone();
        self.build_funding_plan_with(
            request,
            hub,
            move |service, request, generation| async move {
                service
                    .read_preflight_inputs(&request, generation, &input_hub)
                    .await
            },
        )
        .await
    }

    pub(super) async fn build_funding_plan_with<I, F>(
        self: &Arc<Self>,
        mut request: StockFundingPlanRequest,
        hub: &realtime::WsHub,
        read_inputs: I,
    ) -> Result<StockMarketSnapshot, String>
    where
        I: FnOnce(Arc<Self>, StockPreflightRequest, u64) -> F,
        F: std::future::Future<Output = Result<preflight::Inputs, String>>,
    {
        request.wallet_address = request.wallet_address.trim().into();
        if !(16..=128).contains(&request.request_id.len())
            || !request
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err("补库请求编号无效".into());
        }
        stock_inventory::validate_owner(&request.wallet_address)?;
        let keys = (self.credential_loader)()?;
        let fingerprint = keys.fingerprint();
        if self
            .funding_store
            .previous(&request, &fingerprint)?
            .is_some()
        {
            return Ok(self.snapshot());
        }
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票账户读取进行中，请等待结果")?;
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.security_asset)?;
        let initial = self.snapshot();
        let report = initial
            .preflight
            .filter(|p| {
                p.asset == request.security_asset
                    && p.checked_at_ms == request.preflight_at_ms
                    && p.wallet_address.as_deref() == Some(&request.wallet_address)
            })
            .ok_or("请先检查当前钱包库存，再保存对应的补库计划")?;
        let now = common::time::now_ms();
        if now < report.checked_at_ms || now - report.checked_at_ms > 30_000 {
            return Err("补库检查已过期，请重新检查库存".into());
        }
        if !report
            .funding
            .iter()
            .filter(|r| r.direction == request.direction)
            .flat_map(|r| &r.needs)
            .any(|n| n.asset == request.funding_asset && n.target == request.target.label())
        {
            return Err("原检查没有该资产的补库缺口".into());
        }
        let read = StockPreflightRequest {
            asset: request.security_asset.clone(),
            wallet_address: Some(request.wallet_address.clone()),
        };
        let inputs = read_inputs(self.clone(), read, generation).await?;
        if inputs.fingerprint.as_deref() != Some(&fingerprint) {
            return Err("当前账户库存未读取或账户已变化".into());
        }
        let wallet = inputs.wallet.ok_or("钱包库存未核实，未保存补库计划")?;
        let (capacity, address) = self.funding_endpoints(&request, &keys).await?;
        self.ensure_generation(generation, &request.security_asset)?;
        if (self.credential_loader)()?.fingerprint() != fingerprint {
            return Err("账户已变化，旧补库准备已丢弃".into());
        }
        {
            let account = self.account.read();
            let snapshot = self.snapshot.read();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err("股票选择已变化，未保存补库计划".into());
            }
            let account = account
                .evidence
                .as_ref()
                .filter(|a| a.fingerprint == fingerprint)
                .ok_or("账户余额已失效")?;
            let now = common::time::now_ms();
            let plan = prepare(
                request,
                &snapshot,
                account,
                &wallet,
                &report.directions,
                capacity,
                address,
                now,
            )?;
            self.funding_store.insert(plan, now)?;
        }
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(crate) fn cancel_funding_plan(
        &self,
        request: StockPlanRevisionRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        self.funding_store
            .cancel(&request, common::time::now_ms())?;
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(in crate::services::backpack_stocks) async fn funding_endpoints(
        &self,
        request: &StockFundingPlanRequest,
        keys: &credentials::Credentials,
    ) -> Result<(Option<StockWithdrawalCapacity>, Option<StockDepositAddress>), String> {
        match request.target {
            StockFundingTarget::Solana => Ok((
                Some(
                    self.withdrawal_capacity(&request.funding_asset, keys)
                        .await?,
                ),
                None,
            )),
            StockFundingTarget::Backpack => Ok((
                None,
                Some(StockDepositAddress {
                    asset: request.security_asset.clone(),
                    address: self.fetch_deposit_address(keys).await?,
                    blockchain: "Solana".into(),
                    account_fingerprint: keys.fingerprint(),
                    checked_at_ms: common::time::now_ms(),
                }),
            )),
        }
    }

    pub(in crate::services::backpack_stocks) async fn withdrawal_capacity(
        &self,
        asset: &str,
        keys: &credentials::Credentials,
    ) -> Result<StockWithdrawalCapacity, String> {
        let bytes = tokio::time::timeout(
            Duration::from_secs(6),
            self.signed_rfq_request(
                keys,
                reqwest::Method::GET,
                "/api/v1/account/limits/withdrawal",
                "maxWithdrawalQuantity",
                &json!({"symbol":asset,"autoBorrow":false,"autoLendRedeem":false}),
            ),
        )
        .await
        .map_err(|_| "账户可提上限读取超时，未预留或转账")?
        .map_err(|_| "无法读取账户不借款可提上限，请检查读取权限与连接")?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "可提上限响应无法解析")?;
        if value["symbol"].as_str() != Some(asset)
            || value["autoBorrow"].as_bool() != Some(false)
            || value["autoLendRedeem"].as_bool() != Some(false)
        {
            return Err("可提上限资产或借款/赎回口径未核实".into());
        }
        let quantity = decimal(
            value["maxWithdrawalQuantity"]
                .as_str()
                .ok_or("可提上限未知")?,
        )?;
        Ok(StockWithdrawalCapacity {
            asset: asset.into(),
            quantity: exact(quantity),
            checked_at_ms: common::time::now_ms(),
        })
    }
}
