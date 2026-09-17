//! Candidate-only Kraken Funding API methods, identities and costs.
//! <https://docs.kraken.com/api-reference/funding-beta/list-funding-methods>
use super::kraken_config::KrakenSpotCredentials;
use super::kraken_funding_rest::{invalid, rows};
use super::kraken_symbols::{canonical_asset, funding_asset};
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use crate::{canonical_network_id, parse_optional_decimal, CurrencyTransferNetwork};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
pub(super) struct Asset {
    pub class: String,
    pub name: String,
}
#[derive(Debug, Deserialize)]
pub(super) struct Amount {
    pub asset: Asset,
    pub amount: String,
}
impl Amount {
    pub(super) fn exact(&self, currency: &str) -> ExchangeResult<Decimal> {
        if self.asset.class != "currency"
            || canonical_asset(&self.asset.name) != canonical_asset(currency)
        {
            return Err(invalid("Funding amount asset mismatch"));
        }
        Decimal::from_str_exact(&self.amount)
            .ok()
            .filter(|v| *v >= Decimal::ZERO)
            .ok_or_else(|| invalid("Funding amount is not an exact nonnegative decimal"))
    }
}
#[derive(Debug, Deserialize)]
pub(super) struct Fees {
    pub base: Amount,
    #[serde(rename = "included")]
    pub _included: bool,
    pub percentage: Option<String>,
    pub min: Option<Amount>,
    pub max: Option<Amount>,
}
#[derive(Debug, Deserialize)]
pub(super) struct Network {
    pub network_id: String,
    pub network_name: String,
    pub contract_address: Option<String>,
}
#[derive(Debug, Deserialize)]
pub(super) struct FundingMethod {
    pub asset: Asset,
    pub method_id: String,
    pub minimum_amount: Option<String>,
    pub maximum_amount: Option<String>,
    pub fees: Fees,
    pub network: Option<Network>,
}
impl FundingMethod {
    pub(super) fn zero_deposit_fee(&self) -> bool {
        self.fees.base.exact(&self.asset.name).ok() == Some(Decimal::ZERO)
            && Decimal::from_str_exact(self.fees.percentage.as_deref().unwrap_or("0")).ok()
                == Some(Decimal::ZERO)
            && self
                .fees
                .min
                .as_ref()
                .is_none_or(|fee| fee.exact(&self.asset.name).ok() == Some(Decimal::ZERO))
            && self
                .fees
                .max
                .as_ref()
                .is_none_or(|fee| fee.exact(&self.asset.name).ok() == Some(Decimal::ZERO))
    }
}
pub(super) async fn methods(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    currency: &str,
    direction: &str,
) -> ExchangeResult<Vec<FundingMethod>> {
    if !matches!(direction, "deposit" | "withdraw") {
        return Err(invalid("invalid funding direction"));
    }
    let methods: Vec<FundingMethod> = rows(
        http,
        base,
        &format!("/funding/v1/methods/{direction}"),
        vec![
            ("asset[class]".into(), "currency".into()),
            ("asset[name]".into(), funding_asset(currency)),
            ("limit".into(), "100".into()),
        ],
        "methods",
        credentials,
    )
    .await?;
    let mut ids = BTreeSet::new();
    for method in &methods {
        if method.asset.class != "currency"
            || canonical_asset(&method.asset.name) != canonical_asset(currency)
            || method.method_id.trim().is_empty()
            || !ids.insert(&method.method_id)
        {
            return Err(invalid("Funding method identity mismatch or duplicate"));
        }
    }
    Ok(methods)
}
pub(super) async fn fetch(
    http: &HttpClient,
    base: &str,
    credentials: &KrakenSpotCredentials,
    currencies: &[String],
) -> ExchangeResult<Vec<CurrencyTransferNetwork>> {
    let requested = currencies
        .iter()
        .map(|c| canonical_asset(c))
        .filter(|c| !c.is_empty())
        .collect::<BTreeSet<_>>();
    if requested.is_empty() {
        return Err(invalid("Funding methods require candidate currencies"));
    }
    let mut result = Vec::new();
    for currency in requested {
        for direction in ["deposit", "withdraw"] {
            let methods = methods(http, base, credentials, &currency, direction).await?;
            let step = if direction == "withdraw" && !methods.is_empty() {
                Some(super::kraken_withdrawals::asset_step(http, base, &currency).await?)
            } else {
                None
            };
            result.extend(
                methods
                    .iter()
                    .filter_map(|method| project(method, direction, base, step)),
            );
        }
    }
    Ok(result)
}
fn project(
    method: &FundingMethod,
    direction: &str,
    base: &str,
    step: Option<Decimal>,
) -> Option<CurrencyTransferNetwork> {
    let network = method.network.as_ref()?;
    if network.network_id.trim().is_empty() || network.network_name.trim().is_empty() {
        return None;
    }
    let withdraw = direction == "withdraw";
    let fixed = method.fees.base.exact(&method.asset.name).ok();
    let rate = method
        .fees
        .percentage
        .as_deref()
        .unwrap_or("0")
        .parse::<Decimal>()
        .ok()
        .filter(|value| *value >= Decimal::ZERO)
        .map(|value| value / Decimal::from(100));
    let simple_fee = method.fees.min.is_none() && method.fees.max.is_none();
    let minimum = parse_optional_decimal(method.minimum_amount.as_deref());
    Some(CurrencyTransferNetwork {
        venue: "kraken".into(),
        currency: canonical_asset(&method.asset.name),
        // A method ID binds one asset, network and direction; display names do not.
        network: method.method_id.clone(),
        canonical_network: canonical_network_id(&network.network_name),
        contract_address: network
            .contract_address
            .clone()
            .filter(|v| !v.trim().is_empty()),
        deposit_enabled: !withdraw,
        withdraw_enabled: withdraw,
        withdrawal_fee: (withdraw && simple_fee).then_some(fixed).flatten(),
        withdrawal_fee_rate: (withdraw && simple_fee).then_some(rate).flatten(),
        withdrawal_step: step,
        min_withdraw: withdraw.then_some(minimum).flatten(),
        min_deposit: (!withdraw).then_some(minimum).flatten(),
        requires_tag: false,
        credit_confirmations: None,
        unlock_confirmations: None,
        network_status: (!withdraw && !method.zero_deposit_fee())
            .then(|| "充值有费用，需要核对净到账".into()),
        checked_at_ms: common::time::now_ms(),
        source_url: format!("{base}/funding/v1/methods/{direction}"),
    })
}
