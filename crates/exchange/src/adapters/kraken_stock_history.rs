//! Cold recovery only; streaming receipts stay on the existing executions WS.
//! https://docs.kraken.com/api-reference/account-data/query-orders-info
//! https://docs.kraken.com/api-reference/account-data/query-trades-info
//! https://docs.kraken.com/api-reference/account-data/get-closed-orders
use super::kraken::Kraken;
use crate::{ExchangeError, ExchangeResult};
use reqwest::Method;
use rust_decimal::{prelude::ToPrimitive, Decimal};
use serde_json::{json, Map, Value};
use shared_types::stocks::*;
use std::collections::BTreeSet;

fn invalid(message: &str) -> ExchangeError {
    ExchangeError::Parse(format!("kraken stock history: {message}"))
}
fn object(body: &str) -> ExchangeResult<Map<String, Value>> {
    if body.len() > 1_048_576 {
        return Err(invalid("response exceeds limit"));
    }
    let v: Value = serde_json::from_str(body).map_err(|_| invalid("invalid JSON"))?;
    if !v["error"].as_array().is_some_and(|a| a.is_empty()) {
        return Err(invalid(
            "history read rejected; check closed order and trade read permissions",
        ));
    }
    v["result"]
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("missing result"))
}
fn text(v: &Value) -> ExchangeResult<&str> {
    v.as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .ok_or_else(|| invalid("missing identity"))
}
fn number(v: &Value) -> ExchangeResult<Decimal> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(invalid("missing decimal; not treated as zero")),
    };
    stock_exact_decimal(&s).map_err(invalid)
}
fn amount(v: &Value) -> ExchangeResult<String> {
    Ok(number(v)?.normalize().to_string())
}
fn millis(v: &Value) -> ExchangeResult<i64> {
    number(v)?
        .checked_mul(Decimal::from(1000))
        .and_then(|n| n.to_i64())
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("invalid exchange timestamp"))
}
fn sum(a: Decimal, b: Decimal) -> ExchangeResult<Decimal> {
    a.checked_add(b)
        .filter(|n| n.checked_sub(a) == Some(b) && n.checked_sub(b) == Some(a))
        .ok_or_else(|| invalid("receipt arithmetic overflow"))
}

impl Kraken {
    pub(super) async fn read_stock_order_history(
        &self,
        original: &StockPeerOrderReceipt,
    ) -> ExchangeResult<Option<StockPeerOrderReceipt>> {
        original.validate_stored().map_err(invalid)?;
        let id = if let Some(id) = &original.order_id {
            id.clone()
        } else {
            // Never search by ticker/amount or infer non-submission from an empty result.
            let mut found = None;
            for (path, key) in [
                ("/0/private/ClosedOrders", "closed"),
                ("/0/private/OpenOrders", "open"),
            ] {
                let mut args = json!({"cl_ord_id":original.client_order_id,"trades":false,"rebase_multiplier":"rebased"});
                if key == "closed" {
                    args["start"] = json!(original.draft.prepared_at_ms / 1000 - 1);
                    args["closetime"] = json!("open");
                }
                let r = object(&self.stock_signed_read(path, args).await?)?;
                let rows = r
                    .get(key)
                    .and_then(Value::as_object)
                    .ok_or_else(|| invalid("missing orders"))?;
                if rows.len() > 1
                    || key == "closed"
                        && r.get("count").and_then(Value::as_u64) != Some(rows.len() as u64)
                {
                    return Err(invalid("ambiguous original client order ID"));
                }
                if let Some((id, row)) = rows.iter().next() {
                    if row["cl_ord_id"].as_str() != Some(&original.client_order_id)
                        || found.is_some()
                    {
                        return Err(invalid("original client order ID mismatch or reuse"));
                    }
                    found = Some(id.clone());
                }
            }
            let Some(id) = found else {
                return Ok(None);
            };
            id
        };
        let r = object(&self.stock_signed_read("/0/private/QueryOrders",
            json!({"txid":id,"trades":true,"consolidate_taker":false,"rebase_multiplier":"rebased"})).await?)?;
        if r.len() != 1 || !r.contains_key(&id) {
            return Err(invalid("original order not returned"));
        }
        let order = &r[&id];
        if matches!(order["status"].as_str(), Some("pending" | "open")) {
            return Ok(None);
        }
        let ids = trade_ids(order)?;
        let mut trades = Map::new();
        for chunk in ids.chunks(20) {
            let batch = object(
                &self
                    .stock_signed_read(
                        "/0/private/QueryTrades",
                        json!({"txid":chunk.join(","),"rebase_multiplier":"rebased"}),
                    )
                    .await?,
            )?;
            if batch.len() != chunk.len() || !chunk.iter().all(|id| batch.contains_key(id)) {
                return Err(invalid("incomplete original trade history"));
            }
            trades.extend(batch);
        }
        let native = &original.draft.request.selection.native_symbol;
        let mut url = url::Url::parse(&format!("{}/0/public/AssetPairs", self.spot_base_url))
            .map_err(|_| invalid("metadata URL"))?;
        let asset_class = if original.draft.purpose.is_equity() {
            "tokenized_asset"
        } else {
            "currency"
        };
        url.query_pairs_mut()
            .append_pair("pair", native)
            .append_pair("aclass_base", asset_class);
        let response = self
            .http
            .execute_with_retry_fresh(Method::GET, url.as_str(), || {
                Ok(self.http.request(Method::GET, url.as_str()))
            })
            .await?;
        if !response.status().is_success() {
            return Err(invalid("market metadata unavailable"));
        }
        let pairs = object(
            &response
                .text()
                .await
                .map_err(|_| invalid("market metadata decoding"))?,
        )?;
        let (key, spec) = pairs
            .iter()
            .find(|(_, v)| {
                v["wsname"] == *native
                    && v["aclass_base"] == asset_class
                    && v["quote"].as_str().is_some_and(|q| {
                        super::kraken_symbols::canonical_asset(q) == original.draft.quote_asset
                    })
            })
            .ok_or_else(|| invalid("exact official stock market missing"))?;
        let aliases = [key.as_str(), text(&spec["altname"])?, native.as_str()];
        parse_history(
            original,
            &id,
            order,
            &trades,
            &aliases,
            common::time::now_ms(),
        )
        .map(Some)
    }
}

fn trade_ids(order: &Value) -> ExchangeResult<Vec<String>> {
    let empty = Vec::new();
    let rows = match order.get("trades") {
        None if number(&order["vol_exec"])? == Decimal::ZERO => &empty,
        Some(v) => v.as_array().ok_or_else(|| invalid("invalid trade list"))?,
        _ => return Err(invalid("missing original trade list")),
    };
    if rows.len() > MAX_STOCK_PEER_FILLS {
        return Err(invalid("trade history exceeds bounded recovery limit"));
    }
    let ids = rows
        .iter()
        .map(|v| text(v).map(str::to_owned))
        .collect::<ExchangeResult<Vec<_>>>()?;
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(invalid("duplicate history trade ID"));
    }
    Ok(ids)
}

fn parse_history(
    original: &StockPeerOrderReceipt,
    id: &str,
    order: &Value,
    trades: &Map<String, Value>,
    aliases: &[&str],
    now: i64,
) -> ExchangeResult<StockPeerOrderReceipt> {
    let d = &original.draft;
    let side = if d.request.direction == StockChainDirection::Buy {
        "sell"
    } else {
        "buy"
    };
    let phase = match order["status"].as_str() {
        Some("closed") => StockCexOrderPhase::Filled,
        Some("canceled") => StockCexOrderPhase::Cancelled,
        Some("expired") => StockCexOrderPhase::Expired,
        _ => return Err(invalid("order is not terminal")),
    };
    let open = millis(&order["opentm"])?;
    let close = millis(&order["closetm"])?;
    let flags = text(&order["oflags"])?.split(',').collect::<BTreeSet<_>>();
    if original.order_id.as_deref().is_some_and(|old| old != id)
        || order["cl_ord_id"].as_str() != Some(&original.client_order_id)
        || !aliases.contains(&text(&order["descr"]["pair"])?)
        || order["descr"]["type"] != side
        || order["descr"]["ordertype"] != "limit"
        || order["descr"]["leverage"] != "none"
        || number(&order["descr"]["price"])?
            != stock_exact_decimal(&d.limit_price).map_err(invalid)?
        || number(&order["vol"])? != stock_exact_decimal(&d.quantity).map_err(invalid)?
        || !flags.contains("fciq")
        || flags.contains("fcib")
        || flags.contains("viqc")
        || open < d.prepared_at_ms
        || close < open
        || close > now
    {
        return Err(invalid("history does not match original cash limit order"));
    }
    let ids = trade_ids(order)?;
    if trades.len() != ids.len() || !ids.iter().all(|id| trades.contains_key(id)) {
        return Err(invalid("original fills incomplete"));
    }
    let mut next = StockPeerOrderReceipt::pending(d.clone(), original.client_order_id.clone())
        .map_err(invalid)?;
    next.apply(StockPeerExecutionPatch {
        order_id: id.into(),
        client_order_id: Some(original.client_order_id.clone()),
        native_symbol: Some(d.request.selection.native_symbol.clone()),
        side: Some(side.into()),
        order_quantity: Some(d.quantity.clone()),
        phase: Some(phase),
        cumulative_quantity: Some(amount(&order["vol_exec"])?),
        cumulative_cost: Some(amount(&order["cost"])?),
        fill: None,
        occurred_at_ms: close,
    })
    .map_err(invalid)?;
    let mut total_fee = Decimal::ZERO;
    let mut numbers = BTreeSet::new();
    for trade in ids {
        let t = &trades[&trade];
        let at = millis(&t["time"])?;
        let trade_id = t["trade_id"]
            .as_u64()
            .ok_or_else(|| invalid("missing numeric trade ID"))?;
        if t["ordertxid"] != id
            || !aliases.contains(&text(&t["pair"])?)
            || t["type"] != side
            || t["ordertype"] != "limit"
            || number(&t["margin"])? != Decimal::ZERO
            || at < open
            || at > close
            || !numbers.insert(trade_id)
        {
            return Err(invalid(
                "fill belongs to another original order or quantity unit",
            ));
        }
        total_fee = sum(total_fee, number(&t["fee"])?)?;
        next.apply(StockPeerExecutionPatch {
            order_id: id.into(),
            client_order_id: Some(original.client_order_id.clone()),
            native_symbol: None,
            side: None,
            order_quantity: None,
            phase: None,
            cumulative_quantity: None,
            cumulative_cost: None,
            fill: Some(StockPeerFill {
                execution_id: trade,
                trade_id: Some(trade_id),
                quantity: amount(&t["vol"])?,
                price: amount(&t["price"])?,
                cost: Some(amount(&t["cost"])?),
                fees: Some(vec![StockTradeFee {
                    asset: d.quote_asset.clone(),
                    quantity: amount(&t["fee"])?,
                }]),
                occurred_at_ms: at,
            }),
            occurred_at_ms: at,
        })
        .map_err(invalid)?;
    }
    if total_fee != number(&order["fee"])? || !next.receipt_complete() {
        return Err(invalid(
            "order totals, constituent fills or native fees do not reconcile",
        ));
    }
    next.validate_stored().map_err(invalid)?;
    Ok(next)
}

#[cfg(test)]
mod tests;
