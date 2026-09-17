use super::*;

impl BackpackStocks {
    // Caller holds submission_lock. Persist the rate limit before starting IO so
    // repeated clicks, timeouts and restart cannot bypass it.
    pub(super) async fn recover_peer_history(
        &self,
        id: &str,
        adapter: &Arc<dyn ExchangeAdapter>,
    ) -> Result<(), String> {
        let old = self.peer_plan_store.get(id)?;
        let original = old.cex_order.as_ref().ok_or("原股票订单丢失")?;
        let now = common::time::now_ms();
        if original.receipt_complete() || now < old.cex_history.next_check_at_ms {
            return Ok(());
        }
        self.peer_plan_store.update(id, |p| {
            p.cex_history.attempts = p
                .cex_history
                .attempts
                .checked_add(1)
                .ok_or("历史查询次数溢出")?;
            let delay = (15_000_i64 * i64::from(p.cex_history.attempts.min(4))).min(60_000);
            p.cex_history.next_check_at_ms = now.saturating_add(delay);
            p.cex_history.problem = Some("已登记原订单历史查询；未核实前保留占用，不重发".into());
            Ok(())
        })?;
        let found = tokio::time::timeout(
            Duration::from_secs(12),
            adapter.reconcile_stock_order(original),
        )
        .await;
        if !Arc::ptr_eq(adapter, &self.peer_execution_adapter(&old)?) {
            return Err("历史查询期间 Kraken 账户发生变化，未合并结果".into());
        }
        let problem = match found {
            Ok(Ok(Some(row))) => {
                let p = self.peer_plan_store.receipt(id, &row)?;
                if p.cex_order.as_ref().is_some_and(|r| r.receipt_complete()) {
                    None
                } else {
                    Some("历史与已有回执未完全核齐；保留占用，不重发".into())
                }
            }
            Ok(Ok(None)) => Some("原订单终态尚未找到，不代表未下单；保留占用，稍后继续核对".into()),
            Ok(Err(_)) => Some(
                "Kraken 原订单历史读取或核验失败；请检查订单及成交历史读取权限，原占用保留".into(),
            ),
            Err(_) => Some("Kraken 原订单历史查询超时；原占用保留，不重发".into()),
        };
        self.peer_plan_store.update(id, |p| {
            p.cex_history.checked_at_ms = Some(common::time::now_ms());
            p.cex_history.problem = if p.cex_order.as_ref().is_some_and(|r| r.receipt_complete()) {
                None
            } else {
                problem
            };
            Ok(())
        })?;
        Ok(())
    }
}
