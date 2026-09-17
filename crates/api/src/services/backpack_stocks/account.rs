use super::*;
use rust_decimal::Decimal;
use serde_json::Value;
use std::{collections::BTreeMap, str::FromStr};

#[derive(Default)]
pub(super) struct AccountCache {
    pub fingerprint: String,
    pub evidence: Option<StockAccountEvidence>,
    updates: BTreeMap<String, StockAccountBalance>,
    epoch: u64,
}

impl BackpackStocks {
    pub(super) fn invalidate_funding_inventory(&self, fingerprint: &str) {
        let mut cache = self.account.write();
        if cache.fingerprint != fingerprint {
            return;
        }
        cache.epoch = cache.epoch.saturating_add(1);
        cache.evidence = None;
        cache.updates.clear();
        drop(cache);
        // Keep private WS subscriptions alive; only the pre-transfer inventory is invalid.
        self.snapshot.write().preflight = None;
    }

    pub(super) fn invalidate_account(&self) {
        self.account_subscription.send_replace(None);
        self.order_subscription.send_replace(None);
        let mut cache = self.account.write();
        cache.epoch = cache.epoch.saturating_add(1);
        cache.evidence = None;
        cache.updates.clear();
        drop(cache);
        let mut snapshot = self.snapshot.write();
        snapshot.deposit_address = None;
        if let Some(report) = snapshot.preflight.as_mut() {
            report.valid_until_ms = common::time::now_ms();
        }
    }

    pub(super) async fn read_account(
        &self,
        keys: &credentials::Credentials,
    ) -> Result<StockAccountEvidence, String> {
        let _guard = self.account_lock.lock().await;
        let fingerprint = keys.fingerprint();
        let start = common::time::now_ms();
        let epoch = {
            let mut cache = self.account.write();
            if cache.fingerprint != fingerprint {
                *cache = AccountCache {
                    fingerprint: fingerprint.clone(),
                    ..Default::default()
                };
            }
            if let Some(e) = cache.evidence.as_ref().filter(|e| {
                start >= e.balances_at_ms
                    && start - e.balances_at_ms < 5_000
                    && start >= e.fees_at_ms
                    && start - e.fees_at_ms < 300_000
            }) {
                return Ok(e.clone());
            }
            cache.epoch
        };
        let params = serde_json::json!({});
        let summary = self
            .signed_rfq_request(
                keys,
                reqwest::Method::GET,
                "/api/v1/account",
                "accountQuery",
                &params,
            )
            .await
            .map_err(String::from)?;
        let balances = self
            .signed_rfq_request(
                keys,
                reqwest::Method::GET,
                "/api/v1/capital",
                "balanceQuery",
                &params,
            )
            .await
            .map_err(String::from)?;
        let mut evidence = parse(&summary, &balances, &fingerprint, start)?;
        if (self.credential_loader)()?.fingerprint() != fingerprint {
            return Err("Backpack 凭证已变化，旧账户结果已丢弃".into());
        }
        let mut cache = self.account.write();
        if cache.epoch != epoch || cache.fingerprint != fingerprint {
            return Err("账户连接在查询期间变化，请重新预检".into());
        }
        // A private update observed during the REST baseline must not be overwritten by it.
        for (asset, update) in &cache.updates {
            if update
                .source_at_us
                .is_some_and(|t| t >= start.saturating_mul(1000))
            {
                evidence.balances.insert(asset.clone(), update.clone());
            }
        }
        cache.evidence = Some(evidence.clone());
        cache.updates.clear();
        Ok(evidence)
    }
}

fn dec(v: &Value, label: &str, nonnegative: bool) -> Result<String, String> {
    let s = v
        .as_str()
        .ok_or_else(|| format!("Backpack {label} 未返回数值字符串"))?;
    let n = Decimal::from_str(s).map_err(|_| format!("Backpack {label} 数值无效"))?;
    if nonnegative && n < Decimal::ZERO {
        return Err(format!("Backpack {label} 不能为负数"));
    }
    Ok(n.normalize().to_string())
}
fn balance(v: &Value, now: i64) -> Result<StockAccountBalance, String> {
    Ok(StockAccountBalance {
        available: dec(&v["available"], "available", true)?,
        locked: dec(&v["locked"], "locked", true)?,
        staked: dec(&v["staked"], "staked", true)?,
        observed_at_ms: now,
        source_at_us: None,
    })
}
pub(super) fn parse(
    summary: &[u8],
    balances: &[u8],
    fingerprint: &str,
    now: i64,
) -> Result<StockAccountEvidence, String> {
    let summary: Value =
        serde_json::from_slice(summary).map_err(|_| "Backpack 账户摘要结构异常")?;
    let maker = dec(&summary["spotMakerFee"], "spotMakerFee", false)?;
    let taker = dec(&summary["spotTakerFee"], "spotTakerFee", true)?;
    if Decimal::from_str(&taker).unwrap() > Decimal::from(10_000) {
        return Err("Backpack 现货费率超出支持范围".into());
    }
    let liquidating = summary["liquidating"]
        .as_bool()
        .ok_or("账户清算状态未返回")?;
    let balances: Value = serde_json::from_slice(balances).map_err(|_| "Backpack 余额结构异常")?;
    let object = balances
        .as_object()
        .filter(|v| v.len() <= 2048)
        .ok_or("Backpack 余额列表无效或过大")?;
    let rows = object
        .iter()
        .map(|(asset, v)| {
            if asset.is_empty() || asset.len() > 64 {
                return Err("Backpack 余额资产标识无效".into());
            }
            Ok((asset.clone(), balance(v, now)?))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    Ok(StockAccountEvidence {
        fingerprint: fingerprint.into(),
        spot_maker_fee_bps: maker,
        spot_taker_fee_bps: taker,
        liquidating,
        fees_at_ms: now,
        balances_at_ms: now,
        balances: rows,
    })
}

pub(super) fn apply_frame(
    s: &BackpackStocks,
    text: &str,
    fingerprint: &str,
    now: i64,
) -> Result<bool, String> {
    let value: Value = serde_json::from_str(text).map_err(|_| "Backpack 私有账户帧格式异常")?;
    let frame = if let Some(data) = value.get("data") {
        if value["stream"].as_str() != Some("account.balanceUpdate") {
            return Ok(false);
        }
        data
    } else {
        &value
    };
    if frame["e"] != "balanceUpdate" {
        return Ok(false);
    }
    let asset = frame["a"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or("账户事件缺少资产标识")?;
    let source = frame["T"]
        .as_i64()
        .filter(|t| *t > 0 && *t <= now.saturating_add(5_000).saturating_mul(1000))
        .ok_or("账户事件时间无效")?;
    let mut row = balance(
        &serde_json::json!({"available":frame["A"],"locked":frame["L"],"staked":frame["S"]}),
        now,
    )?;
    row.source_at_us = Some(source);
    let mut cache = s.account.write();
    if cache.fingerprint != fingerprint {
        return Ok(false);
    }
    let old = cache
        .updates
        .get(asset)
        .or_else(|| cache.evidence.as_ref().and_then(|e| e.balances.get(asset)));
    if old.is_some_and(|r| {
        r.source_at_us.is_some_and(|t| t > source)
            || (r.source_at_us == Some(source)
                && r.available == row.available
                && r.locked == row.locked
                && r.staked == row.staked)
            || r.source_at_us.is_none() && source < r.observed_at_ms.saturating_mul(1000)
    }) {
        return Ok(false);
    }
    if (cache.updates.len() >= 2048 && !cache.updates.contains_key(asset))
        || cache
            .evidence
            .as_ref()
            .is_some_and(|e| e.balances.len() >= 2048 && !e.balances.contains_key(asset))
    {
        return Err("账户事件资产数量超出上限".into());
    }
    // One settlement may emit multiple updates at the same engine timestamp, in wire order.
    cache.updates.insert(asset.into(), row.clone());
    if let Some(e) = cache.evidence.as_mut() {
        e.balances.insert(asset.into(), row);
    }
    drop(cache);
    let mut snapshot = s.snapshot.write();
    if asset == "USDC" || snapshot.security.as_ref().is_some_and(|s| s.asset == asset) {
        if let Some(report) = snapshot.preflight.as_mut() {
            report.valid_until_ms = now;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
