use super::*;
use serde_json::{json, Value};

impl BackpackStocks {
    // Internal leg transport. A future two-leg coordinator must authorize the
    // complete plan; deliberately no public endpoint can submit this leg alone.
    #[allow(dead_code)]
    pub(super) async fn send_orderbook_leg(
        self: &Arc<Self>,
        id: &str,
        hub: realtime::WsHub,
    ) -> Result<StockExecutionPlan, String> {
        let _guard = self
            .order_lock
            .try_lock()
            .map_err(|_| "股票订单正在处理，请核对原计划")?;
        let keys = (self.credential_loader)()?;
        let fingerprint = keys.fingerprint();
        let old = self.owned_stock_order(id, &fingerprint, false)?;
        if old.cex_order.is_some() {
            return Ok(old);
        }
        if !matches!(
            old.terms.cex_instruction,
            Some(StockCexInstruction::OrderBook { .. })
        ) {
            return Err("RFQ 接受与两腿协调尚未接通，不能使用订单簿提交".into());
        }
        self.order_tracking_until_ms.store(
            common::time::now_ms().saturating_add(30_000),
            Ordering::SeqCst,
        );
        self.ensure_rfq_started(hub.clone());
        let mut subscription = self.order_subscription.subscribe();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if subscription.borrow_and_update().as_deref() == Some(&fingerprint) {
                    return Ok::<_, String>(());
                }
                subscription
                    .changed()
                    .await
                    .map_err(|_| "股票私有订单连接已停止".to_owned())?;
            }
        })
        .await
        .map_err(|_| "股票私有订单订阅未就绪，未提交".to_owned())??;
        let (plan, send) = self.begin_stock_order(id, &fingerprint)?;
        if !send {
            return Ok(plan);
        }
        self.submit_prepared_order(&keys, &plan, &hub).await
    }

    pub(super) async fn submit_prepared_order(
        &self,
        keys: &credentials::Credentials,
        plan: &StockExecutionPlan,
        hub: &realtime::WsHub,
    ) -> Result<StockExecutionPlan, String> {
        let id = &plan.plan_id;
        self.publish_rfq(&hub);
        let instruction = plan
            .terms
            .cex_instruction
            .as_ref()
            .ok_or("原计划交易指令丢失")?;
        let result = self
            .signed_rfq_request(
                keys,
                reqwest::Method::POST,
                instruction.path(),
                instruction.signing_instruction(),
                &instruction.request_body(),
            )
            .await;
        let now = common::time::now_ms();
        let applied = match result {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
                .map_err(|_| "股票下单回执未完整解析；只核对原订单，不重发".to_owned())
                .and_then(|v| {
                    self.record_order_receipt(id, now, |r, i| {
                        order_protocol::apply_order(r, i, &v, false, now)
                    })
                }),
            Err(error) => self.plan_store.change_order(id, now, |r, _| {
                if error.rejected
                    && r.order_id.is_none()
                    && r.fills.is_empty()
                    && r.phase == StockCexOrderPhase::SubmissionUnknown
                {
                    r.phase = StockCexOrderPhase::Rejected;
                    r.executed_quantity = Some("0".into());
                    r.executed_quote_quantity = Some("0".into());
                    r.recheck.next_at_ms = None;
                }
                r.problem = Some(error.problem.replace("Backpack RFQ", "Backpack 股票订单"));
                r.updated_at_ms = now.max(r.updated_at_ms);
                Ok(true)
            }),
        };
        if let Err(problem) = applied {
            self.stock_order_problem(id, problem)?;
        }
        self.publish_rfq(&hub);
        self.plan_store.get(id)
    }

    fn owned_stock_order(
        &self,
        id: &str,
        fingerprint: &str,
        submitted: bool,
    ) -> Result<StockExecutionPlan, String> {
        let plan = self.plan_store.get(id)?;
        if plan.terms.account_fingerprint != fingerprint {
            return Err("当前 API 凭证不是该股票计划的原账户，请恢复原凭证后核对".into());
        }
        if submitted && plan.cex_order.is_none() {
            return Err("该计划尚未提交订单，没有远端订单可核对".into());
        }
        Ok(plan)
    }

    pub(crate) async fn recheck_stock_order(
        self: &Arc<Self>,
        id: &str,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let plan = self.plan_store.get(id)?;
        if plan.two_leg_started_at_ms.is_some() {
            return self.recheck_stock_pair(id, hub).await;
        }
        if plan.chain_submission.is_some() {
            return self.recheck_stock_chain(id, hub).await;
        }
        if let Some(rfq) = self.plan_store.get(id)?.rfq_acceptance {
            return self.recheck_rfq(&rfq.request.request_id, hub).await;
        }
        let _guard = self
            .order_lock
            .try_lock()
            .map_err(|_| "原股票订单正在核对")?;
        let keys = (self.credential_loader)()?;
        self.owned_stock_order(id, &keys.fingerprint(), true)?;
        let result = self.reconcile_stock_order(id, &keys, false).await;
        if let Err(problem) = &result {
            self.stock_order_problem(id, problem.clone())?;
        }
        self.ensure_rfq_started(hub.clone());
        self.publish_rfq(&hub);
        result?;
        Ok(self.snapshot())
    }

    pub(super) fn stock_order_problem(&self, id: &str, problem: String) -> Result<(), String> {
        let now = common::time::now_ms();
        self.plan_store.change_order(id, now, |r, _| {
            if r.evidence_conflict || r.problem.as_ref() == Some(&problem) {
                return Ok(false);
            }
            r.problem = Some(problem);
            r.updated_at_ms = now.max(r.updated_at_ms);
            Ok(true)
        })?;
        Ok(())
    }

    pub(super) async fn reconcile_stock_order(
        &self,
        id: &str,
        keys: &credentials::Credentials,
        automatic: bool,
    ) -> Result<(), String> {
        let plan = self.owned_stock_order(id, &keys.fingerprint(), true)?;
        let record = plan.cex_order.as_ref().ok_or("缺少原订单")?;
        if plan.phase == StockPlanPhase::Settled
            || (record.phase == StockCexOrderPhase::Rejected && record.order_id.is_none())
        {
            return Ok(());
        }
        let now = common::time::now_ms();
        if automatic
            && (!record.needs_follow_up() || record.recheck.next_at_ms.is_some_and(|t| t > now))
        {
            return Ok(());
        }
        // Automatic retries consume a durable budget; an unchanged manual read need not append.
        if automatic {
            self.plan_store.change_order(id, now, |r, _| {
                r.recheck.attempts = r.recheck.attempts.saturating_add(1).min(6);
                r.recheck.paused = r.recheck.attempts >= 6;
                r.recheck.next_at_ms = (!r.recheck.paused).then_some(now.saturating_add(30_000));
                r.updated_at_ms = now.max(r.updated_at_ms);
                Ok(true)
            })?;
        }
        let Some(StockCexInstruction::OrderBook {
            client_id, symbol, ..
        }) = plan.terms.cex_instruction.as_ref()
        else {
            return Err("原计划不是订单簿交易".into());
        };
        let mut params = json!({"symbol":symbol});
        if let Some(remote) = &record.order_id {
            params["orderId"] = json!(remote);
        } else {
            params["clientId"] = json!(client_id);
        }
        // The open-order endpoint does not cover a filled FOK; use its original history ID.
        let refresh_history = !automatic && record.phase.terminal();
        if !record.phase.terminal() {
            let open = self
                .signed_rfq_request(
                    keys,
                    reqwest::Method::GET,
                    "/api/v1/order",
                    "orderQuery",
                    &params,
                )
                .await;
            match open {
                Ok(bytes) => {
                    let v: Value =
                        serde_json::from_slice(&bytes).map_err(|_| "股票订单查询回执格式无效")?;
                    self.apply_stock_order(id, &v)?;
                }
                Err(error) if error.status == Some(404) => {}
                Err(error) => {
                    return Err(error.problem.replace("Backpack RFQ", "Backpack 股票订单"))
                }
            }
        }
        let current = self.plan_store.get(id)?;
        let order = current.cex_order.as_ref().ok_or("原订单记录缺失")?;
        if refresh_history || !order.phase.terminal() || order.order_id.is_none() {
            let mut query = json!({"symbol":symbol,"limit":100,"sortDirection":"Desc"});
            if let Some(remote) = &order.order_id {
                query["orderId"] = json!(remote);
            }
            // History has no clientId filter. Search bounded pages for the exact
            // original client, never treat absence as permission to resubmit.
            let mut found = None;
            for page in 0..3 {
                query["offset"] = json!(page * 100);
                let bytes = self
                    .signed_rfq_request(
                        keys,
                        reqwest::Method::GET,
                        "/wapi/v1/history/orders",
                        "orderHistoryQueryAll",
                        &query,
                    )
                    .await
                    .map_err(String::from)?;
                let rows: Vec<Value> =
                    serde_json::from_slice(&bytes).map_err(|_| "股票历史订单回执格式无效")?;
                for v in &rows {
                    if order_protocol::client(&v["clientId"]) == Some(*client_id) {
                        if found.is_some() {
                            return Err("历史中出现多个相同 clientId 的订单，不能自动归账".into());
                        }
                        found = Some(v.clone());
                    }
                }
                if rows.len() < 100 || order.order_id.is_some() {
                    break;
                }
            }
            if let Some(v) = found {
                self.apply_stock_order(id, &v)?;
            } else {
                return Err("暂未查到原股票订单；保留资金占用，不重复提交".into());
            }
        }
        let current = self.plan_store.get(id)?;
        let remote = current
            .cex_order
            .as_ref()
            .and_then(|o| o.order_id.as_ref())
            .ok_or("原订单编号仍待确认")?;
        let mut fills = vec![];
        for page in 0..6 {
            let query = json!({"orderId":remote,"symbol":symbol,"limit":100,"offset":page*100,"sortDirection":"Asc"});
            let bytes = self
                .signed_rfq_request(
                    keys,
                    reqwest::Method::GET,
                    "/wapi/v1/history/fills",
                    "fillHistoryQueryAll",
                    &query,
                )
                .await
                .map_err(String::from)?;
            let rows: Vec<Value> =
                serde_json::from_slice(&bytes).map_err(|_| "股票成交历史回执格式无效")?;
            let full = rows.len() >= 100;
            fills.extend(rows);
            if fills.len() > 512 || (full && page == 5) {
                return Err("原订单成交记录超出读取预算，保留占用并人工核对".into());
            }
            if !full {
                break;
            }
        }
        let now = common::time::now_ms();
        let current = self.record_order_receipt(id, now, |r, i| {
            order_protocol::apply_fills(r, i, &fills, now)
        })?;
        if current
            .cex_order
            .as_ref()
            .is_some_and(|o| !o.receipt_complete())
        {
            self.stock_order_problem(
                id,
                "订单或原币费用明细尚未完整；保留占用，不等于整笔套利完成".into(),
            )?;
        }
        Ok(())
    }

    fn apply_stock_order(&self, id: &str, value: &Value) -> Result<(), String> {
        let now = common::time::now_ms();
        self.record_order_receipt(id, now, |r, i| {
            order_protocol::apply_order(r, i, value, false, now)
        })?;
        Ok(())
    }

    fn record_order_receipt(
        &self,
        id: &str,
        now: i64,
        apply: impl FnOnce(&mut StockCexOrder, &StockCexInstruction) -> Result<bool, String>,
    ) -> Result<StockExecutionPlan, String> {
        match self.plan_store.change_order(id, now, apply) {
            Ok(plan) => Ok(plan),
            Err(problem) => {
                let known = self
                    .plan_store
                    .get(id)?
                    .cex_order
                    .is_some_and(|r| r.order_id.is_some() || !r.fills.is_empty());
                if known {
                    self.plan_store.change_order(id, now, |r, _| {
                        if r.evidence_conflict
                            && r.recheck.paused
                            && r.problem.as_ref() == Some(&problem)
                        {
                            return Ok(false);
                        }
                        r.evidence_conflict = true;
                        r.recheck.paused = true;
                        r.recheck.next_at_ms = None;
                        r.problem = Some(problem.clone());
                        r.updated_at_ms = now.max(r.updated_at_ms);
                        Ok(true)
                    })?;
                }
                Err(problem)
            }
        }
    }
}

pub(super) fn apply_frame(
    s: &BackpackStocks,
    text: &str,
    fingerprint: &str,
    now: i64,
) -> Result<bool, String> {
    let envelope: Value = serde_json::from_str(text).map_err(|_| "股票私有 WS 回执格式无效")?;
    let v = envelope.get("data").unwrap_or(&envelope);
    if !matches!(
        v["e"].as_str(),
        Some("orderAccepted" | "orderCancelled" | "orderExpired" | "orderFill" | "orderModified")
    ) {
        return Ok(false);
    }
    let plan=s.plan_store.records().into_iter().find(|p|p.terms.account_fingerprint==fingerprint && p.cex_order.is_some() && matches!(p.terms.cex_instruction.as_ref(),Some(StockCexInstruction::OrderBook{client_id,..}) if Some(*client_id)==order_protocol::client(&v["c"])));
    let Some(plan) = plan else {
        return Ok(false);
    };
    let updated = s.record_order_receipt(&plan.plan_id, now, |r, i| {
        order_protocol::apply_order(r, i, v, true, now)
    });
    updated.map(|p| p.revision != plan.revision)
}

#[cfg(test)]
mod tests;
