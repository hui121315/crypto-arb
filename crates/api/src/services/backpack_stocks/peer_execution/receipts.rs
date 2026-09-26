use super::*;

impl BackpackStocks {
    pub(in crate::services::backpack_stocks) fn resume_peer_receipts(
        self: &Arc<Self>,
        hub: &realtime::WsHub,
    ) {
        let Some(adapter) = self
            .peer_feed
            .as_ref()
            .and_then(|(agg, _)| agg.get("kraken"))
        else {
            return;
        };
        let Some(fingerprint) = adapter.stock_account_fingerprint() else {
            return;
        };
        if self.peer_plan_store.pending(&fingerprint).is_empty() {
            return;
        }
        if let Ok(rx) = adapter.subscribe_stock_receipts() {
            self.start_peer_receipts(adapter, fingerprint, rx, hub.clone());
        }
    }

    pub(in crate::services::backpack_stocks) fn start_peer_receipts(
        self: &Arc<Self>,
        adapter: Arc<dyn ExchangeAdapter>,
        fingerprint: String,
        mut rx: broadcast::Receiver<StockPeerOrderReceipt>,
        hub: realtime::WsHub,
    ) {
        let mut worker = self.peer_receipt_worker.lock();
        // A rebuilt adapter has a different event channel even with the same credentials.
        let worker_key = format!("{fingerprint}:{:p}", Arc::as_ptr(&adapter));
        if worker
            .as_ref()
            .is_some_and(|(key, h)| key == &worker_key && !h.is_finished())
        {
            return;
        }
        if let Some((_, h)) = worker.take() {
            h.abort();
        }
        let weak = Arc::downgrade(self);
        let key = fingerprint.clone();
        *worker = Some((
            worker_key,
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(10));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    let event = tokio::select! {
                        event = rx.recv() => match event {
                            Ok(row) => Some(row),
                            Err(broadcast::error::RecvError::Lagged(_)) => None,
                            Err(broadcast::error::RecvError::Closed) => break,
                        },
                        _ = tick.tick() => None,
                    };
                    let Some(s) = weak.upgrade() else {
                        break;
                    };
                    let pending = s.peer_plan_store.pending(&key);
                    if pending.is_empty() || s.peer_plan_store.problem().is_some() {
                        break;
                    }
                    if s.peer_execution_adapter(&pending[0]).is_err()
                        || s.peer_feed
                            .as_ref()
                            .and_then(|(agg, _)| agg.get("kraken"))
                            .is_none_or(|a| !Arc::ptr_eq(&a, &adapter))
                    {
                        break;
                    }
                    let mut changed = false;
                    if let Some(row) = event {
                        if let Some(p) = pending.iter().find(|p| {
                            owned_receipts(p).any(|r| r.client_order_id == row.client_order_id)
                        }) {
                            match save_receipt(&s, p, &row) {
                                Ok(next) => changed |= next != *p,
                                Err(_) if s.peer_plan_store.get(&p.plan_id)
                                    .is_ok_and(|p| p.phase == StockPeerPlanPhase::Settled) => continue,
                                Err(_) => break,
                            }
                        }
                    } else {
                        // A fresh submission registers itself immediately before send.
                        // Restoring it earlier would suppress that first send.
                        if let Ok(_guard) = s.submission_lock.try_lock() {
                            for p in &pending {
                                for original in owned_receipts(p) {
                                    if adapter.track_stock_order(original.clone()).is_err() {
                                        continue;
                                    }
                                    if let Some(row) =
                                        adapter.stock_order_receipt(&original.client_order_id)
                                    {
                                        if let Ok(next) = save_receipt(&s, p, &row) {
                                            changed |= next != *p;
                                        }
                                    }
                                }
                            }
                            let now = common::time::now_ms();
                            if let Some(p) = pending.iter().find(|p| {
                                now >= p.terms.created_at_ms.saturating_add(30_000)
                                    && p.cex_history.attempts < 6
                                    && now >= p.cex_history.next_check_at_ms
                                    && p.cex_order.as_ref().is_some_and(|r| !r.receipt_complete())
                            }) {
                                if s.recover_peer_history(&p.plan_id, &adapter).await.is_ok() {
                                    changed = true;
                                }
                            } else if let Some((p, index)) = pending.iter().find_map(|p| {
                                p.inventory_orders
                                    .iter()
                                    .enumerate()
                                    .find(|(_, c)| {
                                        now >= c.draft.prepared_at_ms.saturating_add(30_000)
                                            && c.history.attempts < 6
                                            && now >= c.history.next_check_at_ms
                                            && c.order
                                                .as_ref()
                                                .is_some_and(|o| !o.receipt_complete())
                                    })
                                    .map(|(index, _)| (p, index))
                            }) {
                                if s.recover_peer_inventory(&p.plan_id, index, &adapter)
                                    .await
                                    .is_ok()
                                {
                                    changed = true;
                                }
                            } else if let Some((p, index)) = pending.iter().find_map(|p| {
                                p.conversions
                                    .iter()
                                    .enumerate()
                                    .find(|(_, c)| {
                                        now >= c.draft.prepared_at_ms.saturating_add(30_000)
                                            && c.history.attempts < 6
                                            && now >= c.history.next_check_at_ms
                                            && c.order
                                                .as_ref()
                                                .is_some_and(|o| !o.receipt_complete())
                                    })
                                    .map(|(index, _)| (p, index))
                            }) {
                                if s.recover_peer_conversion(&p.plan_id, index, &adapter)
                                    .await
                                    .is_ok()
                                {
                                    changed = true;
                                }
                            }
                        }
                    }
                    if changed {
                        s.publish_plan(&hub);
                    }
                    drop(s);
                    // Shares the account's existing socket/token; it is not an order,
                    // cancellation or a second market-data subscription.
                    let _ =
                        tokio::time::timeout(Duration::from_secs(8), adapter.warm_stock_receipts())
                            .await;
                }
            }),
        ));
    }
}

fn owned_receipts(p: &StockPeerPlan) -> impl Iterator<Item = &StockPeerOrderReceipt> {
    p.cex_order
        .iter()
        .chain(p.conversions.iter().filter_map(|c| c.order.as_ref()))
        .chain(p.inventory_orders.iter().filter_map(|c| c.order.as_ref()))
}
fn save_receipt(
    s: &BackpackStocks,
    p: &StockPeerPlan,
    row: &StockPeerOrderReceipt,
) -> Result<StockPeerPlan, String> {
    if p.cex_order
        .as_ref()
        .is_some_and(|o| o.client_order_id == row.client_order_id)
    {
        s.peer_plan_store.receipt(&p.plan_id, row)
    } else if let Some(index) = p.inventory_orders.iter().position(|c| {
        c.order
            .as_ref()
            .is_some_and(|o| o.client_order_id == row.client_order_id)
    }) {
        s.peer_plan_store.inventory_receipt(&p.plan_id, index, row)
    } else {
        let index = p
            .conversions
            .iter()
            .position(|c| {
                c.order
                    .as_ref()
                    .is_some_and(|o| o.client_order_id == row.client_order_id)
            })
            .ok_or("回执不属于原计划")?;
        s.peer_plan_store.conversion_receipt(&p.plan_id, index, row)
    }
}
