//! Candidate-only tokenized-asset funding, in base token rather than equity units.
//! https://docs.kraken.com/api-reference/funding-beta/list-funding-methods
use super::{
    kraken::Kraken,
    kraken_funding_rest::{invalid, rows},
    kraken_transfer_networks::{Amount, FundingMethod},
};
use crate::{ExchangeError, ExchangeResult};
use rust_decimal::Decimal;
use shared_types::stocks::*;
use std::collections::BTreeSet;
use std::time::Duration;

fn decimal(value: &str) -> ExchangeResult<String> {
    value
        .parse::<Decimal>()
        .ok()
        .filter(|n| *n >= Decimal::ZERO)
        .map(|n| n.normalize().to_string())
        .ok_or_else(|| invalid("stock funding amount invalid"))
}
fn amount(value: Amount) -> ExchangeResult<StockPeerFundingAmount> {
    if !matches!(value.asset.class.as_str(), "currency" | "tokenized_asset")
        || value.asset.name.trim().is_empty()
    {
        return Err(invalid("stock funding fee asset unknown"));
    }
    Ok(StockPeerFundingAmount {
        asset_class: value.asset.class,
        asset: value.asset.name,
        amount: decimal(&value.amount)?,
    })
}
fn project(
    methods: Vec<FundingMethod>,
    asset: &str,
    class: &str,
) -> ExchangeResult<Vec<StockPeerFundingMethod>> {
    let mut ids = BTreeSet::new();
    methods
        .into_iter()
        .map(|m| {
            if m.asset.class != class
                || m.asset.name != asset
                || m.method_id.trim().is_empty()
                || !ids.insert(m.method_id.clone())
            {
                return Err(invalid("stock funding method asset or ID mismatch"));
            }
            let network = m
                .network
                .ok_or_else(|| invalid("stock funding network missing"))?;
            if network.network_id.trim().is_empty() || network.network_name.trim().is_empty() {
                return Err(invalid("stock funding network identity missing"));
            }
            let minimum = m.minimum_amount.as_deref().map(decimal).transpose()?;
            let maximum = m.maximum_amount.as_deref().map(decimal).transpose()?;
            if minimum
                .as_ref()
                .zip(maximum.as_ref())
                .is_some_and(|(a, b)| a.parse::<Decimal>().unwrap() > b.parse::<Decimal>().unwrap())
            {
                return Err(invalid("stock funding limits reversed"));
            }
            Ok(StockPeerFundingMethod {
                method_id: m.method_id,
                network_id: network.network_id,
                network_name: network.network_name,
                contract_address: network.contract_address.filter(|v| !v.trim().is_empty()),
                minimum_amount: minimum,
                maximum_amount: maximum,
                fees: StockPeerFundingFees {
                    base: amount(m.fees.base)?,
                    included: m.fees._included,
                    percentage: m.fees.percentage.as_deref().map(decimal).transpose()?,
                    minimum: m.fees.min.map(amount).transpose()?,
                    maximum: m.fees.max.map(amount).transpose()?,
                },
            })
        })
        .collect()
}

impl Kraken {
    pub(super) async fn read_stock_funding_methods(
        &self,
        native: &str,
    ) -> ExchangeResult<Vec<StockPeerFundingRoute>> {
        let keys = self
            .config
            .credentials
            .as_ref()
            .and_then(|c| c.spot.as_ref())
            .ok_or_else(|| {
                ExchangeError::Auth("Kraken Spot Funds Query credentials missing".into())
            })?;
        let (stock, _) = native
            .split_once('/')
            .filter(|(b, q)| {
                !b.is_empty()
                    && b.len() <= 64
                    && b.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
                    && matches!(*q, "USD" | "USDC" | "USDT")
            })
            .ok_or_else(|| invalid("exact stock market required"))?;
        let mut routes = Vec::new();
        for (asset, class) in [(stock, "tokenized_asset"), ("USDC", "currency")] {
            for direction in [
                StockPeerFundingDirection::Deposit,
                StockPeerFundingDirection::Withdraw,
            ] {
                let path = format!("/funding/v1/methods/{}", direction.as_str());
                let result = tokio::time::timeout(
                    Duration::from_secs(4),
                    rows::<FundingMethod>(
                        &self.http,
                        &self.spot_base_url,
                        &path,
                        vec![
                            ("asset[class]".into(), class.into()),
                            ("asset[name]".into(), asset.into()),
                            ("rebase_multiplier".into(), "base".into()),
                            ("limit".into(), "100".into()),
                        ],
                        "methods",
                        keys,
                    ),
                )
                .await;
                let projected = match result {
                    Ok(Ok(m)) => project(m, asset, class),
                    _ => Err(invalid("stock funding read failed")),
                };
                let (methods,problem)=match projected {Ok(m)=>(m,None),Err(_)=>(vec![],Some("本方向充提资料未取得，请检查 Kraken Funds Query 权限和连接；未当作已关闭".into()))};
                routes.push(StockPeerFundingRoute {
                    asset: asset.into(),
                    asset_class: class.into(),
                    direction,
                    amount_unit: "base".into(),
                    methods,
                    checked_at_ms: common::time::now_ms(),
                    source_url: format!("{}{path}", self.spot_base_url),
                    problem,
                });
            }
        }
        Ok(routes)
    }
}

#[cfg(test)]
mod tests;
