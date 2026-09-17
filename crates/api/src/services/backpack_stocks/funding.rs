use super::*;

impl BackpackStocks {
    pub(crate) async fn read_deposit_address(
        &self,
        request: StockDepositAddressRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票账户读取进行中，请等待结果")?;
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        let keys = match (self.credential_loader)() {
            Ok(keys) => keys,
            Err(_) => {
                self.snapshot.write().deposit_address = None;
                self.publish_plan(hub);
                return Err("请配置 Backpack 读取凭证，再查询当前账户的充值地址".into());
            }
        };
        let fingerprint = keys.fingerprint();
        let now = common::time::now_ms();
        let cached = self.snapshot.read().deposit_address.clone();
        if cached.as_ref().is_some_and(|a| {
            a.asset == request.asset
                && a.account_fingerprint == fingerprint
                && a.blockchain == "Solana"
                && now >= a.checked_at_ms
                && now - a.checked_at_ms <= 30_000
        }) {
            return Ok(self.snapshot());
        }
        {
            let mut s = self.snapshot.write();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err("股票选择已变化，未读取充值地址".into());
            }
            s.deposit_address = None;
        }
        self.publish_plan(hub);
        let address = self.fetch_deposit_address(&keys).await?;
        if (self.credential_loader)()?.fingerprint() != fingerprint {
            return Err("账户凭证已变化，旧充值地址已丢弃".into());
        }
        self.ensure_generation(generation, &request.asset)?;
        {
            let mut s = self.snapshot.write();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err("股票选择已变化，旧充值地址已丢弃".into());
            }
            s.deposit_address = Some(StockDepositAddress {
                asset: request.asset,
                address,
                blockchain: "Solana".into(),
                account_fingerprint: fingerprint,
                checked_at_ms: common::time::now_ms(),
            });
        }
        self.publish_plan(hub);
        Ok(self.snapshot())
    }

    pub(super) async fn fetch_deposit_address(
        &self,
        keys: &credentials::Credentials,
    ) -> Result<String, String> {
        let response = tokio::time::timeout(
            Duration::from_secs(6),
            self.signed_rfq_request(
                keys,
                reqwest::Method::GET,
                "/wapi/v1/capital/deposit/address",
                "depositAddressQuery",
                &serde_json::json!({"blockchain":"Solana"}),
            ),
        )
        .await
        .map_err(|_| "充值地址读取超时，没有发起转账".to_owned())?
        .map_err(|_| "Backpack 充值地址读取失败，请检查 KYC、账户充值权限与连接".to_owned())?;
        let response: serde_json::Value =
            serde_json::from_slice(&response).map_err(|_| "官方充值地址响应无法解析")?;
        let address = response
            .get("address")
            .and_then(serde_json::Value::as_str)
            .ok_or("官方充值地址响应不完整")?;
        super::super::onchain_comparison::stock_inventory::validate_owner(address)?;
        Ok(address.into())
    }
}

#[cfg(test)]
mod tests;
