use super::*;

impl BackpackStocks {
    pub(crate) async fn peer_funding(
        &self,
        request: StockPeerFundingRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票检查正在进行，请稍后再试")?;
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        self.refresh_peer(common::time::now_ms());
        let snapshot = self.snapshot();
        let peer = snapshot
            .peer
            .as_ref()
            .filter(|p| p.selection == request.selection && p.share_unit_verified)
            .ok_or("先选择已核实的股票现货市场")?;
        if !self.peer_feed_enabled(&peer.selection) {
            return Err("所选交易所订阅已关闭".into());
        }
        let (aggregator, _) = self.peer_feed.as_ref().ok_or("交易所尚未接入")?;
        let adapter = aggregator
            .get(&peer.selection.venue)
            .ok_or("所选交易所尚未接入")?;
        let routes = tokio::time::timeout(
            Duration::from_secs(18),
            adapter.stock_funding_methods(&peer.selection.native_symbol),
        )
        .await
        .map_err(|_| "股票充提读取超时；旧资料不可当作当前状态")?
        .map_err(|_| "股票充提尚未接入或读取失败，请检查现货 Funds Query 权限")?;
        self.ensure_generation(generation, &request.asset)?;
        if aggregator
            .get(&peer.selection.venue)
            .is_none_or(|a| !Arc::ptr_eq(&a, &adapter))
        {
            return Err("交易所配置已变化，旧充提资料已丢弃".into());
        }
        let now = common::time::now_ms();
        let report = StockPeerFunding {
            asset: request.asset,
            selection: request.selection,
            routes,
            checked_at_ms: now,
        };
        let mut s = self.snapshot.write();
        if generation != self.generation.load(Ordering::SeqCst) || !report.current(&s, now) {
            return Err("市场选择、资料身份或时效已变化，未保存旧充提结果".into());
        }
        s.peer_funding = Some(report);
        s.observed_at_ms = now.max(s.observed_at_ms.saturating_add(1));
        drop(s);
        self.publish(hub);
        Ok(self.snapshot())
    }
}
