use super::*;
use credentials::Credentials;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) struct RfqRequestError {
    pub(super) problem: String,
    pub(super) rejected: bool,
    pub(super) status: Option<u16>,
}
impl From<String> for RfqRequestError {
    fn from(problem: String) -> Self {
        Self {
            problem,
            rejected: false,
            status: None,
        }
    }
}
impl From<&str> for RfqRequestError {
    fn from(problem: &str) -> Self {
        problem.to_owned().into()
    }
}
impl From<RfqRequestError> for String {
    fn from(error: RfqRequestError) -> Self {
        error.problem
    }
}

fn transport_error(error: exchange::ExchangeError) -> RfqRequestError {
    match error {
        exchange::ExchangeError::Http { status, body } => {
            let value = serde_json::from_str::<Value>(&body).ok();
            let code = value
                .as_ref()
                .filter(|v| v["message"].is_string())
                .and_then(|v| v["code"].as_str());
            let rejected = (400..500).contains(&status)
                && matches!(
                    code,
                    Some(
                        "UNAUTHORIZED"
                            | "FORBIDDEN"
                            | "INVALID_SIGNATURE"
                            | "INVALID_CLIENT_REQUEST"
                            | "INVALID_QUANTITY"
                            | "INVALID_SYMBOL"
                            | "INVALID_MARKET"
                            | "INVALID_ASSET"
                            | "INSUFFICIENT_FUNDS"
                            | "INSUFFICIENT_MARGIN"
                            | "TRADING_PAUSED"
                            | "ACCOUNT_DEACTIVATED"
                    )
                );
            RfqRequestError {
                rejected,
                status: Some(status),
                problem: if rejected {
                    format!(
                        "Backpack RFQ 被拒绝：{}（HTTP {status}）",
                        code.unwrap_or_default()
                    )
                } else {
                    format!("Backpack RFQ HTTP {status}；保留原请求，不重发")
                },
            }
        }
        exchange::ExchangeError::RateLimited { retry_after_secs } => {
            format!("Backpack RFQ 限频，{retry_after_secs}s 后可核对原请求").into()
        }
        _ => "Backpack RFQ 请求未获明确回执；请核对原请求，不重复提交".into(),
    }
}

fn local_unsent(r: &StockRfq) -> bool {
    r.phase == StockRfqPhase::NotSent && r.client_id == 0 && r.account_fingerprint.is_empty()
        && r.symbol.is_empty() && r.rfq_id.is_none() && r.acceptance.is_none()
}

impl BackpackStocks {
    pub(crate) async fn finish_unsent_rfq(
        self: &Arc<Self>, mut request: StockRfqRequest, hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self.rfq_lock.try_lock().map_err(|_| "原询价仍在处理中，请稍后核对")?;
        if let Some(problem) = self.plan_store.problem().or_else(|| self.rfq_store.problem()) {
            return Err(problem);
        }
        request.quantity = shared_types::stocks::comparison::positive(&request.quantity)
            .ok_or("询价股数无效")?.normalize().to_string();
        if request.asset.is_empty() || request.asset.len() > 64 || !request.asset.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'.'|b'-'|b'_')) {
            return Err("股票标识无效".into());
        }
        let id = request.request_id.clone();
        if let Some(record) = self.stock_rfq(&id) {
            if record.request != request { return Err("原询价参数不一致，请核对原记录".into()); }
        } else {
            self.rfq_store.finish_unsent(request, common::time::now_ms())?;
        }
        self.publish_rfq(&hub);
        Ok(self.snapshot_for_rfq(&id))
    }

    fn snapshot_for_rfq(&self, id: &str) -> StockMarketSnapshot {
        let mut snapshot = self.snapshot();
        // Explicit recovery must include its receipt even when it is outside recent history.
        if !snapshot.rfqs.iter().any(|r| r.request.request_id == id) {
            if let Some(record) = self.stock_rfq(id) {
                snapshot.rfqs.insert(0, record);
                snapshot.rfqs.truncate(48);
            }
        }
        snapshot
    }

    pub(crate) async fn request_rfq(
        self: &Arc<Self>,
        mut request: StockRfqRequest,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self
            .rfq_lock
            .try_lock()
            .map_err(|_| "RFQ 操作进行中，请等待回执")?;
        request.quantity = shared_types::stocks::comparison::positive(&request.quantity)
            .ok_or("询价股数无效")?
            .normalize()
            .to_string();
        if let Some(old) = self.stock_rfq(&request.request_id).filter(local_unsent) {
            if old.request != request { return Err("RFQ 请求标识与参数冲突".into()); }
            return Ok(self.snapshot_for_rfq(&request.request_id));
        }
        let keys = (self.credential_loader)()?;
        let fingerprint = keys.fingerprint();
        if let Some(old) = self.stock_rfq(&request.request_id) {
            if old.request != request || old.account_fingerprint != fingerprint {
                return Err("RFQ 请求标识与参数或账户冲突".into());
            }
            return Ok(self.snapshot_for_rfq(&request.request_id));
        }
        let generation = self.generation.load(Ordering::SeqCst);
        self.ensure_generation(generation, &request.asset)?;
        self.refresh_context(generation).await?;
        self.ensure_generation(generation, &request.asset)?;
        let snapshot = self.snapshot();
        request.quantity =
            rfq_protocol::validate_quantity(&request, &snapshot, common::time::now_ms())?;
        let symbol = snapshot.security.ok_or("股票未选择")?.rfq_symbol;
        let (record, replayed) = self.rfq_store.claim_with_receipts(
            request,
            &fingerprint,
            symbol,
            common::time::now_ms(),
            &self.plan_store.accepted_rfqs(),
        )?;
        if replayed {
            return Ok(self.snapshot_for_rfq(&record.request.request_id));
        }
        self.ensure_rfq_started(hub.clone());
        self.publish_rfq(&hub);
        let ready = self
            .wait_rfq_subscription(&fingerprint)
            .await
            .and_then(|()| {
                self.ensure_generation(generation, &record.request.asset)?;
                rfq_protocol::validate_quantity(
                    &record.request,
                    &self.snapshot(),
                    common::time::now_ms(),
                )?;
                Ok(())
            });
        if let Err(problem) = ready {
            self.rfq_store
                .change(&record.request.request_id, true, |row| {
                    if row.phase != StockRfqPhase::SubmissionUnknown || row.rfq_id.is_some() {
                        return Ok(false);
                    }
                    row.phase = StockRfqPhase::NotSent;
                    row.needs_recheck = false;
                    row.problem = Some(format!("询价未发送：{problem}"));
                    row.updated_at_ms = common::time::now_ms();
                    Ok(true)
                })?;
            self.publish_rfq(&hub);
            return Ok(self.snapshot_for_rfq(&record.request.request_id));
        }
        let body = rfq_protocol::submit_body(&record);
        let result = self
            .signed_rfq_request(
                &keys,
                reqwest::Method::POST,
                "/api/v1/rfq",
                "rfqSubmit",
                &body,
            )
            .await
            .and_then(|bytes| rfq_protocol::acknowledgement(&bytes).map_err(RfqRequestError::from));
        match result {
            Ok(native) => {
                self.rfq_store
                    .change(&record.request.request_id, true, |row| {
                        rfq_protocol::apply_rest(row, native, common::time::now_ms())
                    })?;
            }
            Err(error) => {
                if error.rejected {
                    self.rfq_store
                        .change(&record.request.request_id, true, |r| {
                            if r.phase != StockRfqPhase::SubmissionUnknown || r.rfq_id.is_some() {
                                return Ok(false);
                            }
                            r.phase = StockRfqPhase::Rejected;
                            r.needs_recheck = false;
                            r.problem = Some(error.problem);
                            r.updated_at_ms = common::time::now_ms();
                            Ok(true)
                        })?;
                } else {
                    self.rfq_problem_record(&record.request.request_id, error.problem)?;
                }
            }
        }
        self.publish_rfq(&hub);
        Ok(self.snapshot_for_rfq(&record.request.request_id))
    }

    pub(crate) async fn recheck_rfq(
        self: &Arc<Self>,
        id: &str,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self.rfq_lock.try_lock().map_err(|_| "RFQ 操作进行中")?;
        if self.stock_rfq(id).is_some_and(|r| local_unsent(&r)) {
            return Ok(self.snapshot_for_rfq(id));
        }
        let keys = (self.credential_loader)()?;
        self.owned_rfq(id, &keys)?;
        let result = self.reconcile_rfq(id, &keys).await;
        if let Err(problem) = &result {
            self.rfq_problem_record(id, problem.clone())?;
        }
        self.ensure_rfq_started(hub.clone());
        self.publish_rfq(&hub);
        result?;
        Ok(self.snapshot_for_rfq(id))
    }

    pub(super) async fn reconcile_rfq(&self, id: &str, keys: &Credentials) -> Result<(), String> {
        self.reconcile_rfq_with_mode(id, keys, false).await
    }

    pub(super) async fn reconcile_rfq_with_mode(
        &self,
        id: &str,
        keys: &Credentials,
        automatic: bool,
    ) -> Result<(), String> {
        let record = self.owned_rfq(id, keys)?;
        if automatic && !record.needs_follow_up() {
            return Ok(());
        }
        if record.acceptance.is_some()
            && automatic
            && !self.begin_settlement_check(id, common::time::now_ms())?
        {
            return Ok(());
        }
        if record.phase == StockRfqPhase::Filled {
            if !automatic {
                self.reconcile_rfq_history(id, keys).await?;
            }
            return self.reconcile_rfq_fills(id, keys, automatic).await;
        }
        if matches!(
            record.phase,
            StockRfqPhase::NotSent | StockRfqPhase::Rejected
        ) {
            return Ok(());
        }
        let mut params = serde_json::json!({"symbol":record.symbol});
        if let Some(remote) = &record.rfq_id {
            params["rfqId"] = Value::String(remote.clone());
        }
        let bytes = self
            .signed_rfq_request(
                keys,
                reqwest::Method::GET,
                "/api/v1/rfqs",
                "rfqQuery",
                &params,
            )
            .await?;
        match rfq_protocol::open_records(&bytes, &record)? {
            Some(native) => {
                self.record_rfq_receipt(id, true, |row| {
                    rfq_protocol::apply_rest(row, native, common::time::now_ms())
                })?;
                if self.stock_rfq(id).is_some_and(|r| {
                    r.acceptance.is_some() && r.phase == StockRfqPhase::AwaitingQuotes
                }) {
                    self.reconcile_rfq_history(id, keys).await?;
                }
            }
            None => {
                if record.rfq_id.is_some() {
                    self.reconcile_rfq_history(id, keys).await?;
                } else {
                    self.rfq_problem_record(
                        id,
                        "未收到官方 RFQ ID，开放列表暂未匹配；保留原请求，不重发".into(),
                    )?;
                }
            }
        }
        self.reconcile_rfq_fills(id, keys, automatic).await
    }

    async fn reconcile_rfq_history(&self, id: &str, keys: &Credentials) -> Result<(), String> {
        let record = self.owned_rfq(id, keys)?;
        let remote = record.rfq_id.as_ref().ok_or("原 RFQ 编号未知")?;
        let bytes = self
            .signed_rfq_request(
                keys,
                reqwest::Method::GET,
                "/wapi/v1/history/rfq",
                "rfqHistoryQueryAll",
                &serde_json::json!({"rfqId":remote,"limit":100}),
            )
            .await?;
        if let Some(native) = super::rfq_history::select(&bytes, &record)? {
            self.record_rfq_receipt(id, true, |r| {
                super::rfq_history::apply(r, native, common::time::now_ms())
            })?;
        } else {
            self.rfq_problem_record(id, "历史查询暂未找到原 RFQ；保留原请求，不重发".into())?;
        }
        Ok(())
    }

    async fn reconcile_rfq_fills(
        &self,
        id: &str,
        keys: &Credentials,
        automatic: bool,
    ) -> Result<(), String> {
        let record = self.owned_rfq(id, keys)?;
        if record.phase == StockRfqPhase::Filled && (record.settlement_pending() || !automatic) {
            if automatic
                && record.acceptance.is_none()
                && !self.begin_settlement_check(id, common::time::now_ms())?
            {
                return Ok(());
            }
            let remote = record
                .rfq_id
                .as_deref()
                .ok_or("已成交 RFQ 缺少原始编号，不能猜测成交身份或重发")?;
            let bytes = self
                .signed_rfq_request(
                    keys,
                    reqwest::Method::GET,
                    "/wapi/v1/history/rfq/fill",
                    "rfqFillHistoryQueryAll",
                    &serde_json::json!({"rfqId":remote,"limit":100}),
                )
                .await?;
            // An absent/partial response is retryable, not conflicting trade evidence.
            let fills = super::rfq_history::parse_fills(&bytes)?;
            self.record_rfq_receipt(id, true, |r| {
                super::rfq_history::apply_fills(r, fills, common::time::now_ms())
            })?;
        }
        Ok(())
    }

    fn begin_settlement_check(&self, id: &str, now: i64) -> Result<bool, String> {
        // Reserve the read attempt before the request so crashes/reconnects cannot reset its budget.
        Ok(self
            .change_rfq(id, true, |r| {
                if !(r.settlement_pending() || (r.acceptance.is_some() && r.unresolved()))
                    || r.settlement.paused
                    || r.acceptance.as_ref().is_some_and(|a| a.evidence_conflict)
                    || r.settlement.next_at_ms.is_some_and(|t| t > now)
                {
                    return Ok(false);
                }
                r.settlement.attempts = r.settlement.attempts.saturating_add(1);
                r.settlement.next_at_ms = Some(now.saturating_add(30_000));
                r.settlement.paused = r.settlement.attempts >= 6;
                r.updated_at_ms = now.max(r.updated_at_ms);
                Ok(true)
            })?
            .is_some())
    }

    pub(crate) async fn cancel_rfq(
        self: &Arc<Self>,
        id: &str,
        hub: realtime::WsHub,
    ) -> Result<StockMarketSnapshot, String> {
        let _guard = self.rfq_lock.try_lock().map_err(|_| "RFQ 操作进行中")?;
        if self.stock_rfq(id).is_some_and(|r| local_unsent(&r)) {
            return Ok(self.snapshot_for_rfq(id));
        }
        let keys = (self.credential_loader)()?;
        let record = self.owned_rfq(id, &keys)?;
        if record.acceptance.is_some() {
            return Err("原计划已记录接受报价；只能核对原 RFQ，不能再次接受或取消".into());
        }
        if record.phase == StockRfqPhase::AcceptedBinding {
            return Err("该 RFQ 已绑定并锁资，官方不允许请求方取消，需等待结算回执".into());
        }
        if record.phase.terminal() || record.cancel_requested {
            return Ok(self.snapshot_for_rfq(id));
        }
        self.change_rfq(id, true, |r| {
            r.cancel_requested = true;
            r.candidate = None;
            r.needs_recheck = true;
            r.problem = Some("已记录取消请求；等待官方终态，超时不重复提交".into());
            Ok(true)
        })?;
        self.publish_rfq(&hub);
        let body = if let Some(remote) = record.rfq_id {
            serde_json::json!({"rfqId":remote})
        } else {
            serde_json::json!({"clientId":record.client_id})
        };
        let result = self
            .signed_rfq_request(
                &keys,
                reqwest::Method::POST,
                "/api/v1/rfq/cancel",
                "rfqCancel",
                &body,
            )
            .await
            .and_then(|b| rfq_protocol::acknowledgement(&b).map_err(RfqRequestError::from));
        match result {
            Ok(native) => {
                self.record_rfq_receipt(id, true, |r| {
                    rfq_protocol::apply_rest(r, native, common::time::now_ms())
                })?;
            }
            Err(error) => {
                if error.rejected {
                    self.change_rfq(id, true, |r| {
                        if r.phase.terminal() || r.phase == StockRfqPhase::AcceptedBinding {
                            return Ok(false);
                        }
                        r.cancel_requested = false;
                        r.needs_recheck = true;
                        r.candidate = None;
                        r.problem = Some(format!("取消请求未被接受：{}", error.problem));
                        r.updated_at_ms = common::time::now_ms();
                        Ok(true)
                    })?;
                } else {
                    self.rfq_problem_record(id, error.problem)?;
                }
            }
        }
        self.ensure_rfq_started(hub.clone());
        self.publish_rfq(&hub);
        Ok(self.snapshot_for_rfq(id))
    }

    fn owned_rfq(&self, id: &str, keys: &Credentials) -> Result<StockRfq, String> {
        if let Some(problem) = self.plan_store.problem() {
            return Err(problem);
        }
        self.stock_rfq(id)
            .filter(|r| r.account_fingerprint == keys.fingerprint())
            .ok_or_else(|| "RFQ 不存在或与当前 Backpack API 账户不匹配".into())
    }
    pub(super) fn rfq_problem_record(&self, id: &str, problem: String) -> Result<(), String> {
        self.change_rfq(id, true, |r| {
            if r.acceptance.as_ref().is_some_and(|a| a.evidence_conflict)
                || (r.phase.terminal() && !r.settlement_pending())
                || (r.candidate.is_some() && !r.needs_recheck)
            {
                return Ok(false);
            }
            if r.problem.as_ref() == Some(&problem) {
                return Ok(false);
            }
            r.needs_recheck = true;
            r.candidate = None;
            r.problem = Some(problem);
            r.updated_at_ms = common::time::now_ms();
            Ok(true)
        })?;
        Ok(())
    }
    pub(super) fn publish_rfq(&self, hub: &realtime::WsHub) {
        let mut snapshot = self.snapshot.write();
        snapshot.observed_at_ms =
            common::time::now_ms().max(snapshot.observed_at_ms.saturating_add(1));
        drop(snapshot);
        self.publish(hub);
    }
    pub(crate) fn resume_rfq(self: &Arc<Self>, hub: realtime::WsHub) {
        if (self.rfq_store.problem().is_none()
            && self.rfq_records().iter().any(StockRfq::needs_follow_up))
            || (self.plan_store.problem().is_none()
                && self
                    .plan_store
                    .accepted_rfqs()
                    .iter()
                    .any(StockRfq::needs_follow_up))
            || (self.plan_store.problem().is_none()
                && self.plan_store.records().iter().any(|p| {
                    p.cex_order
                        .as_ref()
                        .is_some_and(StockCexOrder::needs_follow_up)
                }))
            || (self.exchange_conversion_store.problem().is_none()
                && self
                    .exchange_conversion_store
                    .rows()
                    .iter()
                    .any(|p| p.order.as_ref().is_some_and(StockCexOrder::needs_follow_up)))
        {
            self.ensure_rfq_started(hub);
        }
    }

    pub(super) fn ensure_rfq_started(self: &Arc<Self>, hub: realtime::WsHub) {
        let mut worker = self.rfq_worker.lock();
        if worker.as_ref().is_none_or(|h| h.is_finished()) {
            *worker = Some(tokio::spawn(rfq_runtime::run(
                Arc::downgrade(self),
                hub,
                self.ws_url.clone(),
            )));
        }
    }
    pub(super) async fn wait_rfq_subscription(&self, fingerprint: &str) -> Result<(), String> {
        let mut subscription = self.rfq_subscription.subscribe();
        // Sending SUBSCRIBE is a local ordering barrier, not proof of remote authentication.
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if subscription.borrow_and_update().as_deref() == Some(fingerprint) {
                    return Ok(());
                }
                subscription
                    .changed()
                    .await
                    .map_err(|_| "私有 RFQ 连接已停止".to_owned())?;
            }
        })
        .await
        .map_err(|_| "私有 RFQ 连接未准备好，请检查网络或凭证后重新询价".to_owned())?
    }
    pub(super) async fn signed_rfq_request(
        &self,
        keys: &Credentials,
        method: reqwest::Method,
        path: &str,
        instruction: &str,
        body: &Value,
    ) -> Result<Vec<u8>, RfqRequestError> {
        let params: BTreeMap<String, String> = body
            .as_object()
            .ok_or("RFQ 参数不是对象")?
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect();
        let endpoint = format!("{}{path}", self.root);
        tokio::time::timeout(Duration::from_secs(15), async {
            let mut response = self
                .http
                .execute_once_fresh(method.clone(), &endpoint, || {
                    let now = common::time::now_ms();
                    let signature = keys
                        .signature(instruction, &params, now)
                        .map_err(exchange::ExchangeError::Auth)?;
                    let builder = self
                        .http
                        .request(method.clone(), &endpoint)
                        .header("X-API-Key", &keys.public)
                        .header("X-Signature", signature)
                        .header("X-Timestamp", now.to_string())
                        .header("X-Window", credentials::WINDOW.to_string());
                    Ok(if method == reqwest::Method::GET {
                        builder.query(&params)
                    } else {
                        builder.json(body)
                    })
                })
                .await
                .map_err(transport_error)?;
            let status = response.status();
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "Backpack RFQ 响应未完整收到")?
            {
                if bytes.len() + chunk.len() > 1024 * 1024 {
                    return Err("Backpack RFQ 响应过大".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            if !status.is_success() {
                return Err(transport_error(exchange::ExchangeError::Http {
                    status: status.as_u16(),
                    body: String::from_utf8(bytes).unwrap_or_default(),
                }));
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| "Backpack RFQ 查询超时；保留原请求，不重复提交")?
    }
}
