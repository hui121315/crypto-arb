use super::*;

impl BackpackStocks {
    pub(crate) async fn check_peer_order(
        &self,
        request: StockPeerOrderCheckRequest,
        hub: &realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .preflight_lock
            .try_lock()
            .map_err(|_| "股票预检正在进行")?;
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        self.refresh_peer(common::time::now_ms());
        if !self.peer_feed_enabled(&request.selection) {
            return Err("所选场所行情订阅已关闭".into());
        }
        let snapshot = self.snapshot();
        let now = common::time::now_ms();
        if snapshot
            .peer_order_checks
            .iter()
            .any(|r| now < r.draft.prepared_at_ms || now - r.draft.prepared_at_ms < 5000)
        {
            return Err("股票订单验证间隔至少 5 秒，不会自动重试".into());
        }
        let draft = prepare_peer_order_check(&snapshot, request.clone(), now)?;
        let (aggregator, _) = self.peer_feed.as_ref().ok_or("交易所接入未配置")?;
        let adapter = aggregator
            .get(&request.selection.venue)
            .ok_or("所选交易所未接入")?;
        let pending = StockPeerOrderCheck {
            draft: draft.clone(),
            completed_at_ms: None,
            status: StockPeerOrderCheckStatus::Unknown,
            message: "股票订单仅验证已请求，等待交易所返回；不会成交".into(),
        };
        {
            let mut s = self.snapshot.write();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err("股票选择已改变".into());
            }
            s.peer_order_checks
                .retain(|r| r.draft.request.direction != request.direction);
            s.peer_order_checks.push(pending);
            s.observed_at_ms = now.max(s.observed_at_ms.saturating_add(1));
        }
        self.publish(hub);
        let mut report = match tokio::time::timeout(
            Duration::from_secs(12),
            adapter.validate_stock_order(&draft),
        )
        .await
        {
            Ok(Ok(r)) if r.draft == draft && r.completed_at_ms.is_some() => r,
            _ => StockPeerOrderCheck {
                draft,
                completed_at_ms: Some(common::time::now_ms()),
                status: StockPeerOrderCheckStatus::Unknown,
                message:
                    "股票订单验证未取得完整回复；请检查现货 WS 权限和连接，未认定通过，不自动重试"
                        .into(),
            },
        };
        self.ensure_generation(generation, &request.asset)?;
        if aggregator
            .get(&request.selection.venue)
            .is_none_or(|a| !Arc::ptr_eq(&adapter, &a))
        {
            report.status = StockPeerOrderCheckStatus::Unknown;
            report.completed_at_ms = Some(common::time::now_ms());
            report.message = "交易所配置已变化，旧验证结果不再有效；请重新验证".into();
        }
        let mut s = self.snapshot.write();
        if self.generation.load(Ordering::SeqCst) != generation
            || s.peer
                .as_ref()
                .is_none_or(|p| p.selection != request.selection)
        {
            return Err("对比市场已变化，旧验证结果已丢弃".into());
        }
        if let Some(saved) = s
            .peer_order_checks
            .iter_mut()
            .find(|r| r.draft == report.draft)
        {
            *saved = report;
        }
        s.observed_at_ms = common::time::now_ms().max(s.observed_at_ms.saturating_add(1));
        drop(s);
        self.publish(hub);
        Ok(self.snapshot())
    }
}
