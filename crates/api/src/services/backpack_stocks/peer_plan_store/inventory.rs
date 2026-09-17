use super::*;

pub(in crate::services::backpack_stocks) fn client_id(p: &StockPeerPlan, index: usize) -> String {
    let text = format!("{}:{index}:{}", p.plan_id, p.terms.account_fingerprint);
    format!(
        "si{}",
        &common::signing::hmac_sha256_hex(b"stock-peer-inventory-v1", text.as_bytes())[..16]
    )
}
impl Store {
    pub(in crate::services::backpack_stocks) fn inventory_receipt(
        &self,
        id: &str,
        index: usize,
        row: &StockPeerOrderReceipt,
    ) -> Result<StockPeerPlan, String> {
        self.update(id, |p| {
            let c = p
                .inventory_orders
                .get_mut(index)
                .ok_or("原库存恢复不存在")?;
            let old = c.order.as_mut().ok_or("库存恢复尚未提交")?;
            let _ = old.merge_snapshot(row);
            if old.receipt_complete() {
                c.history.problem = None;
            }
            Ok(())
        })
    }
}
pub(super) fn validate_history(p: &StockPeerPlan) -> Result<(), String> {
    if p.inventory_orders.len() > MAX_STOCK_PEER_INVENTORY_ORDERS
        || (!p.inventory_orders.is_empty() && p.phase != StockPeerPlanPhase::SubmissionUnknown)
    {
        return Err("库存恢复历史数量或父计划状态无效".into());
    }
    for (index, c) in p.inventory_orders.iter().enumerate() {
        let at = c.draft.prepared_at_ms;
        if c.request.revision >= p.revision || at < p.terms.created_at_ms || at > p.updated_at_ms {
            return Err("库存恢复历史版本或时间无效".into());
        }
        let prefix = historical_prefix(p, c.request.revision);
        let rebuilt = StockPeerInventory::compile(
            &prefix,
            c.request.clone(),
            c.market.clone(),
            c.quote.clone(),
            c.account.clone(),
            c.mint.clone(),
            at,
        )?;
        let mut base = c.clone();
        base.cancelled_at_ms = None;
        base.order = None;
        base.history = Default::default();
        if base != rebuilt
            || c.cancelled_at_ms
                .is_some_and(|t| t < at || t > p.updated_at_ms || c.order.is_some())
        {
            return Err("库存恢复原始参数或取消记录变化".into());
        }
        if let Some(o) = &c.order {
            if o.draft != c.draft || o.client_order_id != client_id(p, index) {
                return Err("库存恢复原订单身份变化".into());
            }
            o.validate_stored()?;
        }
        let h = &c.history;
        if h.problem.as_ref().is_some_and(|s| s.len() > 2048)
            || (h.attempts == 0 && *h != StockPeerHistoryCheck::default())
            || h.attempts > 0 && (c.order.is_none() || h.next_check_at_ms <= at)
            || h.checked_at_ms
                .is_some_and(|t| t < at || t > p.updated_at_ms)
        {
            return Err("库存恢复查询历史无效".into());
        }
    }
    Ok(())
}
pub(super) fn transition(a: &StockPeerPlan, b: &StockPeerPlan) -> bool {
    if b.inventory_orders.len() < a.inventory_orders.len()
        || b.inventory_orders.len() > a.inventory_orders.len() + 1
    {
        return false;
    }
    for (old, new) in a.inventory_orders.iter().zip(&b.inventory_orders) {
        let mut original = new.clone();
        original.order = old.order.clone();
        original.cancelled_at_ms = old.cancelled_at_ms;
        original.history = old.history.clone();
        if original != *old
            || old
                .cancelled_at_ms
                .is_some_and(|t| new.cancelled_at_ms != Some(t))
            || new.history.attempts < old.history.attempts
            || new.history.attempts > old.history.attempts.saturating_add(1)
            || new.history.next_check_at_ms < old.history.next_check_at_ms
            || old
                .history
                .checked_at_ms
                .is_some_and(|t| new.history.checked_at_ms.is_none_or(|n| n < t))
        {
            return false;
        }
        match (&old.order, &new.order) {
            (Some(o), Some(n)) => {
                let mut merged = o.clone();
                let _ = merged.merge_snapshot(n);
                if merged != *n {
                    return false;
                }
            }
            (Some(_), None) => return false,
            (None, Some(n)) => {
                if old.cancelled_at_ms.is_some()
                    || b.updated_at_ms >= old.valid_until_ms
                    || a.peer_inventory_gap().is_err()
                    || StockPeerOrderReceipt::pending(n.draft.clone(), n.client_order_id.clone())
                        .as_ref()
                        != Ok(n)
                {
                    return false;
                }
            }
            _ => {}
        }
    }
    if let Some(c) = b.inventory_orders.get(a.inventory_orders.len()) {
        if c.request.revision != a.revision
            || c.draft.prepared_at_ms < a.updated_at_ms
            || c.order.is_some()
            || c.cancelled_at_ms.is_some()
            || StockPeerInventory::compile(
                a,
                c.request.clone(),
                c.market.clone(),
                c.quote.clone(),
                c.account.clone(),
                c.mint.clone(),
                c.draft.prepared_at_ms,
            )
            .as_ref()
                != Ok(c)
        {
            return false;
        }
    }
    a.inventory_orders == b.inventory_orders
        || (a.cex_order == b.cex_order
            && a.chain_submission == b.chain_submission
            && a.recoveries == b.recoveries
            && a.native_topups == b.native_topups
            && a.conversions == b.conversions)
}
