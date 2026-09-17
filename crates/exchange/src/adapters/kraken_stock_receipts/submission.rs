use super::*;
use shared_types::stocks::{StockPeerCancelAck, StockPeerOrderAck, StockPeerOrderReceipt};

impl KrakenSpotPrivateStream {
    pub(in crate::adapters) async fn submit_stock_order(
        &self,
        original: StockPeerOrderReceipt,
    ) -> ExchangeResult<StockPeerOrderReceipt> {
        if let Some(old) = self.state.stocks.get(&original.client_order_id) {
            if old.draft != original.draft {
                return Err(ExchangeError::Parse(
                    "stock client order ID belongs to another intent".into(),
                ));
            }
            return Ok(old);
        }
        let id = next_request_id();
        original
            .kraken_submission("pre-send-check", id, common::time::now_ms())
            .map_err(|e| ExchangeError::Parse(e.into()))?;
        let client = original.client_order_id.clone();
        let Some(_guard) = self.state.stocks.claim_submission(original.clone())? else {
            return self
                .state
                .stocks
                .get(&client)
                .ok_or_else(|| ExchangeError::Parse("original stock receipt missing".into()));
        };
        let token = self.token.clone();
        let response = self
            .session
            .send_fresh(
                move || {
                    original
                        .kraken_submission(token.load().as_ref(), id, common::time::now_ms())
                        .map(|v| v.to_string())
                        .map_err(|e| ExchangeError::Parse(e.into()))
                },
                Box::new(move |text| matches_reply(text, "add_order", id)),
            )
            .await;
        self.state.stocks.update(&client, |receipt| {
            match response
                .and_then(|text| parse_order_ack(&text, &client, id, common::time::now_ms()))
            {
                Ok((ack, order)) => {
                    let _ = receipt.record_submission_ack(ack, order);
                }
                Err(_) => {
                    if !receipt.receipt_complete() && !receipt.evidence_conflict {
                        receipt.problem = Some(
                            "股票提交结果未确认，保留原订单编号；只能查原单，不能重复提交".into(),
                        );
                    }
                }
            }
        })
    }

    pub(in crate::adapters) async fn cancel_stock_order(
        &self,
        client: &str,
    ) -> ExchangeResult<StockPeerCancelAck> {
        let original = self.state.stocks.get(client).ok_or_else(|| {
            ExchangeError::Parse("only a tracked original stock order can be cancelled".into())
        })?;
        if original.receipt_complete() {
            return Err(ExchangeError::Parse(
                "stock order is already settled".into(),
            ));
        }
        let client = original.client_order_id.clone();
        let request_client = client.clone();
        let request_order = original.order_id.clone();
        let token = self.token.clone();
        let id = next_request_id();
        let reply = self
            .session
            .send_fresh(
                move || {
                    let token = token.load();
                    if token.is_empty() {
                        return Err(ExchangeError::Auth("stock WS token missing".into()));
                    }
                    let mut params = json!({"token":token.as_str()});
                    if let Some(order) = &request_order {
                        params["order_id"] = json!([order]);
                    } else {
                        params["cl_ord_id"] = json!([request_client]);
                    }
                    Ok(json!({"method":"cancel_order","req_id":id,"params":params}).to_string())
                },
                Box::new(move |text| matches_reply(text, "cancel_order", id)),
            )
            .await;
        let accepted = reply.ok().and_then(|s| parse_cancel_ack(&s, &original, id));
        Ok(StockPeerCancelAck {
            client_order_id: client,
            accepted,
            received_at_ms: common::time::now_ms(),
            message: match accepted {
                Some(true) => "撤单请求已接收；仍以原订单成交/取消终态和实际费用为准",
                Some(false) => "交易所未接受撤单请求；保留原订单继续核对",
                None => "撤单回复未确认；保留原订单继续核对，不将 ACK 当作取消终态",
            }
            .into(),
        })
    }
}

fn parse_cancel_ack(text: &str, original: &StockPeerOrderReceipt, id: u64) -> Option<bool> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v["method"] != "cancel_order" || v["req_id"].as_u64() != Some(id) {
        return None;
    }
    if v["success"] == false {
        return Some(false);
    }
    let result = &v["result"];
    let matching = result
        .get("cl_ord_id")
        .is_none_or(|c| c.as_str() == Some(original.client_order_id.as_str()))
        && result["order_id"].as_str().is_some_and(|o| {
            !o.is_empty()
                && o.len() <= 128
                && original.order_id.as_deref().is_none_or(|old| old == o)
        });
    (v["success"] == true && matching).then_some(true)
}

fn matches_reply(text: &str, method: &str, id: u64) -> ExchangeResult<bool> {
    let v: Value = serde_json::from_str(text)
        .map_err(|_| ExchangeError::Parse("stock WS reply decoding failed".into()))?;
    Ok(v["method"] == method && v["req_id"].as_u64() == Some(id))
}

fn parse_order_ack(
    text: &str,
    client: &str,
    id: u64,
    now: i64,
) -> ExchangeResult<(StockPeerOrderAck, Option<String>)> {
    if !matches_reply(text, "add_order", id)? {
        return Err(ExchangeError::Parse("stock ACK request ID mismatch".into()));
    }
    let v: Value = serde_json::from_str(text)
        .map_err(|_| ExchangeError::Parse("stock ACK decoding failed".into()))?;
    let result = &v["result"];
    if !result.is_null() && !result.is_object() {
        return Err(ExchangeError::Parse(
            "stock ACK result shape is invalid".into(),
        ));
    }
    if result
        .get("cl_ord_id")
        .is_some_and(|c| c.as_str() != Some(client))
    {
        return Err(ExchangeError::Parse("stock ACK client ID mismatch".into()));
    }
    match v["success"].as_bool() {
        Some(true) => {
            let id = result["order_id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .ok_or_else(|| ExchangeError::Parse("stock ACK lacks original order ID".into()))?;
            Ok((
                StockPeerOrderAck {
                    accepted: true,
                    request_id: v["req_id"].as_u64().unwrap(),
                    received_at_ms: now,
                    message: "股票下单已接收，仍等待实际成交和费用回执".into(),
                },
                Some(id.into()),
            ))
        }
        Some(false)
            if result.get("order_id").is_none_or(Value::is_null)
                && v["error"].as_str().is_some_and(|s| !s.is_empty()) =>
        {
            let error = v["error"].as_str().unwrap();
            let reason = if error.contains("Insufficient funds") {
                "股票下单被拒绝：可用余额不足"
            } else if error.contains("Permission denied") {
                "股票下单被拒绝：账户权限不足"
            } else {
                "交易所拒绝本次股票下单；未创建订单"
            };
            Ok((
                StockPeerOrderAck {
                    accepted: false,
                    request_id: id,
                    received_at_ms: now,
                    message: reason.into(),
                },
                None,
            ))
        }
        _ => Err(ExchangeError::Parse("stock ACK result is ambiguous".into())),
    }
}

#[cfg(test)]
#[path = "submission_tests.rs"]
mod tests;
