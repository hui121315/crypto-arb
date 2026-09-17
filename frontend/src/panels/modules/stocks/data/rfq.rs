use super::*;
use crate::api::rest::{ApiClient, MutationRequestContext};
use gloo_storage::{SessionStorage, Storage};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(in crate::panels::modules::stocks) struct RfqData {
    pub quantity: RwSignal<String>,
    pub side: RwSignal<StockRfqSide>,
    pub pending: RwSignal<bool>,
    pub attempt: RwSignal<Option<StockRfqRequest>>,
    pub submit: Callback<()>,
    pub finish_unsent: Callback<()>,
    pub action: Callback<(String, bool)>,
}

pub(super) fn use_rfq(
    client: ApiClient,
    market: RwSignal<LoadState<StockMarketSnapshot>>,
    notice: RwSignal<Option<String>>,
) -> RfqData {
    let base = client.base_url();
    let storage_key = Arc::new(format!("stocks.rfq.attempt.v1:{base}"));
    let restored = SessionStorage::get::<StockRfqRequest>(storage_key.as_str()).ok();
    let quantity = RwSignal::new(
        restored
            .as_ref()
            .map(|r| r.quantity.clone())
            .unwrap_or_default(),
    );
    let side = RwSignal::new(
        restored
            .as_ref()
            .map(|r| r.side)
            .unwrap_or(StockRfqSide::Ask),
    );
    let pending = RwSignal::new(false);
    let attempt = RwSignal::new(restored);
    // A late WS receipt can resolve a lost HTTP response without sending another RFQ.
    Effect::new({
        let storage_key = Arc::clone(&storage_key);
        move |_| {
            if let Some(request) = attempt.get() {
                if market.with(|m| m.value().is_some_and(|s| has_receipt(s, &request))) {
                    clear_attempt(attempt, &storage_key, &request.request_id);
                }
            }
        }
    });
    let submit = Callback::new({
        let client = client.clone();
        let base = base.clone();
        let storage_key = Arc::clone(&storage_key);
        move |()| {
            if pending.get_untracked() {
                return;
            }
            if client.base_url() != base {
                notice.set(Some(
                    "后端地址已变更，请重新进入股票套利后再操作原请求".into(),
                ));
                return;
            }
            let request = match attempt.get_untracked() {
                Some(request) => request,
                None => {
                    let result = market.with_untracked(|m| {
                        let snapshot = m.value().ok_or("股票行情尚未就绪".to_owned())?;
                        let mut request = StockRfqRequest {
                            request_id: MutationRequestContext::new_idempotent_attempt("stock-rfq")
                                .request_id()
                                .to_owned(),
                            asset: snapshot
                                .security
                                .as_ref()
                                .ok_or("请先选择股票".to_owned())?
                                .asset
                                .clone(),
                            side: side.get_untracked(),
                            quantity: quantity.get_untracked(),
                        };
                        request.quantity = validate_rfq_quantity(
                            &request,
                            snapshot,
                            super::super::super::timestamp::now_ms(),
                        )?;
                        Ok::<_, String>(request)
                    });
                    match result {
                        Ok(request) => request,
                        Err(problem) => {
                            notice.set(Some(problem));
                            return;
                        }
                    }
                }
            };
            if SessionStorage::set(storage_key.as_str(), &request).is_err() {
                notice.set(Some(
                    "无法保存原询价，请检查浏览器存储；本次请求未发送".into(),
                ));
                return;
            }
            attempt.set(Some(request.clone()));
            pending.set(true);
            notice.set(None);
            let client = client.clone();
            let storage_key = Arc::clone(&storage_key);
            spawn_local(async move {
                match client.request_stock_rfq(&request).await {
                    Ok(snapshot) => {
                        if has_receipt(&snapshot, &request) {
                            clear_attempt(attempt, &storage_key, &request.request_id);
                        } else {
                            notice.try_set(Some(
                                "尚未找到与原请求一致的询价回执，请继续核对原请求".into(),
                            ));
                        }
                        apply_snapshot(market, snapshot);
                    }
                    Err(error) => {
                        notice.try_set(Some(error.problem.message));
                    }
                }
                pending.try_set(false);
            });
        }
    });
    let finish_unsent = Callback::new({
        let client = client.clone();
        let base = base.clone();
        let storage_key = Arc::clone(&storage_key);
        move |()| {
            if pending.get_untracked() || client.base_url() != base {
                return;
            }
            let Some(request) = attempt.get_untracked() else {
                return;
            };
            pending.set(true);
            notice.set(None);
            let client = client.clone();
            let storage_key = Arc::clone(&storage_key);
            spawn_local(async move {
                match client.finish_unsent_stock_rfq(&request).await {
                    Ok(snapshot) => {
                        if has_receipt(&snapshot, &request) {
                            clear_attempt(attempt, &storage_key, &request.request_id);
                        }
                        apply_snapshot(market, snapshot);
                    }
                    Err(error) => {
                        notice.try_set(Some(error.problem.message));
                    }
                }
                pending.try_set(false);
            });
        }
    });
    let action = Callback::new(move |(id, cancel): (String, bool)| {
        if pending.get_untracked() {
            return;
        }
        if client.base_url() != base {
            notice.set(Some(
                "后端地址已变更，请重新进入股票套利后再操作原请求".into(),
            ));
            return;
        }
        pending.set(true);
        notice.set(None);
        let client = client.clone();
        let storage_key = Arc::clone(&storage_key);
        spawn_local(async move {
            let result = if cancel {
                client.cancel_stock_rfq(&id).await
            } else {
                client.recheck_stock_rfq(&id).await
            };
            match result {
                Ok(snapshot) => {
                    if attempt
                        .try_get_untracked()
                        .flatten()
                        .is_some_and(|r| r.request_id == id && has_receipt(&snapshot, &r))
                    {
                        clear_attempt(attempt, &storage_key, &id);
                    }
                    apply_snapshot(market, snapshot);
                }
                Err(error) => {
                    notice.try_set(Some(error.problem.message));
                }
            }
            pending.try_set(false);
        });
    });
    RfqData {
        quantity,
        side,
        pending,
        attempt,
        submit,
        finish_unsent,
        action,
    }
}

fn has_receipt(snapshot: &StockMarketSnapshot, request: &StockRfqRequest) -> bool {
    snapshot.rfqs.iter().any(|r| {
        r.request.request_id == request.request_id
            && r.request.asset == request.asset
            && r.request.side == request.side
            && shared_types::stocks::comparison::positive(&request.quantity).is_some_and(|q| {
                Some(q) == shared_types::stocks::comparison::positive(&r.request.quantity)
            })
    })
}

fn clear_attempt(attempt: RwSignal<Option<StockRfqRequest>>, key: &str, id: &str) {
    if SessionStorage::get::<StockRfqRequest>(key)
        .ok()
        .is_some_and(|r| r.request_id == id)
    {
        SessionStorage::delete(key);
    }
    if attempt
        .try_get_untracked()
        .flatten()
        .is_some_and(|r| r.request_id == id)
    {
        attempt.try_set(None);
    }
}

#[cfg(test)]
impl RfqData {
    pub(in crate::panels::modules::stocks) fn fixture() -> Self {
        Self {
            quantity: RwSignal::new("1".into()),
            side: RwSignal::new(StockRfqSide::Ask),
            pending: RwSignal::new(false),
            attempt: RwSignal::new(None),
            submit: Callback::new(|_| {}),
            finish_unsent: Callback::new(|_| {}),
            action: Callback::new(|_| {}),
        }
    }
}
