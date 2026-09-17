use super::*;

impl BackpackStocks {
    pub(crate) async fn peer_preflight(
        &self,
        mut request: StockPeerPreflightRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票预检正在进行")?;
        request.wallet_address = request
            .wallet_address
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        if let Some(owner) = request.wallet_address.as_deref() {
            super::super::onchain_comparison::stock_inventory::validate_owner(owner)?;
        }
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        self.refresh_peer(common::time::now_ms());
        let snapshot = self.snapshot();
        let peer = snapshot
            .peer
            .as_ref()
            .filter(|p| p.selection == request.selection && p.share_unit_verified)
            .ok_or("先选择已核实股数口径的股票市场")?;
        if !self.peer_feed_enabled(&peer.selection) {
            return Err("所选场所订阅已关闭".into());
        }
        let (aggregator, _) = self.peer_feed.as_ref().ok_or("交易所接入未配置")?;
        let adapter = aggregator
            .get(&peer.selection.venue)
            .ok_or("所选交易所尚未接入")?;
        let mut report = StockPeerPreflight {
            asset: request.asset.clone(),
            selection: request.selection.clone(),
            checked_at_ms: common::time::now_ms(),
            account: None,
            wallet: None,
            problems: vec![],
        };
        match tokio::time::timeout(
            Duration::from_secs(12),
            adapter.stock_cash_account(&peer.selection.native_symbol),
        )
        .await
        {
            Ok(Ok(account)) => report.account = Some(account),
            _ => report.problems.push(
                "所选交易所股票账户读取未完成；请检查现货 API 读取权限和连接，未借用 Backpack 余额"
                    .into(),
            ),
        }
        self.ensure_generation(generation, &request.asset)?;
        if aggregator
            .get(&peer.selection.venue)
            .is_none_or(|a| !Arc::ptr_eq(&adapter, &a))
        {
            return Err("交易所配置已变化，旧账户预检已丢弃".into());
        }
        if let (Some(owner), Some(comparison)) = (
            request.wallet_address.as_deref(),
            snapshot.comparison.as_ref(),
        ) {
            match tokio::time::timeout(
                Duration::from_secs(8),
                super::super::onchain_comparison::stock_inventory::read(owner, &comparison.mint),
            )
            .await
            {
                Ok(Ok(wallet)) => {
                    report.problems.extend(wallet.problems.clone());
                    report.wallet = Some(wallet);
                }
                _ => report
                    .problems
                    .push("Solana 钱包库存读取失败或超时，没有按零余额处理".into()),
            }
            if let Err(problem) = self
                .wallet_claims
                .check("solana", owner, common::time::now_ms())
            {
                report.problems.push(problem);
            }
        } else {
            report
                .problems
                .push("未读取链上钱包库存；请填写钱包并取得链上报价".into());
        }
        self.ensure_generation(generation, &request.asset)?;
        if aggregator
            .get(&peer.selection.venue)
            .is_none_or(|a| !Arc::ptr_eq(&adapter, &a))
        {
            return Err("交易所配置已变化，旧账户预检已丢弃".into());
        }
        let mut s = self.snapshot.write();
        if self.generation.load(Ordering::SeqCst) != generation
            || s.peer
                .as_ref()
                .is_none_or(|p| p.selection != request.selection)
        {
            return Err("市场选择已变化，旧预检已丢弃".into());
        }
        report.checked_at_ms = common::time::now_ms();
        s.peer_preflight = Some(report);
        s.observed_at_ms = common::time::now_ms().max(s.observed_at_ms.saturating_add(1));
        drop(s);
        self.publish(hub);
        Ok(self.snapshot())
    }
}
