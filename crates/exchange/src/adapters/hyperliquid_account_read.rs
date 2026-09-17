//! Independent Hyperliquid perp-margin and spot-truth account reads.

use super::{AssetCtx, Hyperliquid, HyperliquidAccountAbstraction, SpotMetaWrapper};
use crate::adapters::hyperliquid_market_data::{
    spot_context_for_entry, spot_contexts_by_coin, spot_pair, spot_token_names,
};
use crate::adapters::hyperliquid_private_data::{
    parse_account_summary, parse_perp_balance, ClearinghouseState,
};
use crate::error::{ExchangeError, ExchangeResult};
use crate::live::{venue_balance_rows, VenueAccountRead, VenueAccountReadIssue};
use common::time::now_ms;
use moka::future::Cache;
use serde_json::json;
use shared_types::{
    is_usd_pegged_settlement_currency, AccountEquityScope, BalanceInfo, VenueAccountSummary,
    VenueAssetValuation,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

type AccountAbstraction = Arc<HyperliquidAccountAbstraction>;
static ACCOUNT_ABSTRACTIONS: OnceLock<Cache<String, AccountAbstraction>> = OnceLock::new();

const ACCOUNT_ABSTRACTION_TTL: Duration = Duration::from_secs(5 * 60);
const ACCOUNT_ABSTRACTION_OPERATION: &str = "account_abstraction";
const PERP_MARGIN_OPERATION: &str = "perp_margin";
const SPOT_TRUTH_OPERATION: &str = "spot_truth";
const SPOT_VALUATION_OPERATION: &str = "spot_valuation";
const SPOT_TRUTH_VENUE: &str = "hyperliquid:spot";
const SPOT_VALUATION_SOURCE: &str =
    "hyperliquid.POST /info spotClearinghouseState + spotMetaAndAssetCtxs.markPx";

impl Hyperliquid {
    pub(super) async fn get_partial_account_read(
        &self,
        currency: Option<&str>,
    ) -> crate::error::ExchangeResult<VenueAccountRead> {
        let user = self.require_user()?;
        let core_spot = self.config.market.dex().is_none();
        let (abstraction, spot) = if core_spot {
            tokio::join!(self.cached_account_abstraction(&user), async {
                Some(tokio::join!(
                    self.get_spot_balance(None),
                    self.spot_context()
                ))
            })
        } else {
            (self.cached_account_abstraction(&user).await, None)
        };
        let consolidated = abstraction
            .as_ref()
            .is_ok_and(|state| is_consolidated_account(&state.user_abstraction));
        let account_type = abstraction
            .as_ref()
            .ok()
            .map(|state| state.user_abstraction.as_str())
            .unwrap_or("spot")
            .to_owned();
        if consolidated && !core_spot {
            return Ok(VenueAccountRead::default());
        }
        let perp = if consolidated {
            None
        } else {
            let body = self.private_state_body("clearinghouseState", &user);
            Some(self.post_info::<ClearinghouseState>(body).await)
        };
        let mut read = VenueAccountRead::default();
        if let Some(perp) = perp {
            match perp {
                Ok(state) => self.extend_perp_account_read(&mut read, &state, currency),
                Err(error) => read.issues.push(VenueAccountReadIssue::new(
                    self.adapter_name(),
                    PERP_MARGIN_OPERATION,
                    error,
                )),
            }
        }
        if let Some((spot, context)) = spot {
            match spot {
                Ok(balances) => {
                    extend_spot_balance_rows(&mut read, &balances, currency);
                    match context {
                        Ok(context) => {
                            let (meta, contexts) = context.as_ref();
                            match spot_account_evidence(
                                &balances,
                                meta,
                                contexts,
                                &account_type,
                                now_ms(),
                            ) {
                                Ok((summary, valuations)) => {
                                    read.summaries.push(summary);
                                    read.asset_valuations.extend(valuations);
                                }
                                Err(error) => read.issues.push(VenueAccountReadIssue::new(
                                    SPOT_TRUTH_VENUE,
                                    SPOT_VALUATION_OPERATION,
                                    error,
                                )),
                            }
                        }
                        Err(error) => read.issues.push(VenueAccountReadIssue::new(
                            SPOT_TRUTH_VENUE,
                            SPOT_VALUATION_OPERATION,
                            error,
                        )),
                    }
                }
                Err(error) => read.issues.push(VenueAccountReadIssue::new(
                    SPOT_TRUTH_VENUE,
                    SPOT_TRUTH_OPERATION,
                    error,
                )),
            }
        }
        if core_spot {
            if let Err(error) = abstraction {
                read.issues.push(VenueAccountReadIssue::new(
                    self.adapter_name(),
                    ACCOUNT_ABSTRACTION_OPERATION,
                    error,
                ));
            }
        }
        Ok(read)
    }

    async fn cached_account_abstraction(&self, user: &str) -> ExchangeResult<AccountAbstraction> {
        let key = format!("{}:{user}", self.base_url);
        account_abstractions()
            .try_get_with(key, async {
                self.account_abstraction_state_rest(user)
                    .await
                    .map(Arc::new)
            })
            .await
            .map_err(|error| ExchangeError::Api {
                exchange: self.adapter_name().to_owned(),
                code: "account_abstraction_fetch_failed".to_owned(),
                message: error.to_string(),
            })
    }

    async fn account_abstraction_state_rest(
        &self,
        user: &str,
    ) -> ExchangeResult<HyperliquidAccountAbstraction> {
        let user_abstraction = crate::adapters::hyperliquid_public_rest::post_info::<String>(
            &self.http,
            &self.base_url,
            json!({
                "type": "userAbstraction",
                "user": user,
            }),
        )
        .await?;
        if user_abstraction.trim().is_empty() {
            return Err(ExchangeError::Parse(
                "hyperliquid userAbstraction returned an empty state".into(),
            ));
        }
        Ok(HyperliquidAccountAbstraction {
            account_address: user.to_owned(),
            user_abstraction,
            user_dex_abstraction: None,
        })
    }

    fn extend_perp_account_read(
        &self,
        read: &mut VenueAccountRead,
        state: &ClearinghouseState,
        currency: Option<&str>,
    ) {
        let venue = self.adapter_name();
        match parse_account_summary(state, venue, now_ms()) {
            Ok(summary) => read.summaries.push(summary),
            Err(error) => read.issues.push(VenueAccountReadIssue::new(
                venue,
                PERP_MARGIN_OPERATION,
                error,
            )),
        }
        match parse_perp_balance(state, currency) {
            Ok(balances) => read.balances.extend(venue_balance_rows(venue, balances)),
            Err(error) => read.issues.push(VenueAccountReadIssue::new(
                venue,
                PERP_MARGIN_OPERATION,
                error,
            )),
        }
    }
}

fn extend_spot_balance_rows(
    read: &mut VenueAccountRead,
    balances: &HashMap<String, BalanceInfo>,
    currency: Option<&str>,
) {
    let mut selected = balances.clone();
    if let Some(currency) = currency {
        selected.retain(|coin, _| coin.eq_ignore_ascii_case(currency));
    }
    read.balances
        .extend(venue_balance_rows(SPOT_TRUTH_VENUE, selected));
}

fn spot_account_evidence(
    balances: &HashMap<String, BalanceInfo>,
    meta: &SpotMetaWrapper,
    contexts: &[AssetCtx],
    account_type: &str,
    observed_at_ms: i64,
) -> ExchangeResult<(VenueAccountSummary, Vec<VenueAssetValuation>)> {
    let prices = spot_usd_prices(meta, contexts);
    let mut rows = balances.values().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.currency.cmp(&right.currency));
    let missing = rows
        .iter()
        .filter(|row| row.total > 0.0)
        .filter(|row| usd_price(&row.currency, &prices).is_none())
        .map(|row| row.currency.clone())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(ExchangeError::Parse(format!(
            "hyperliquid spot assets have no direct USD-pegged mark price: {}",
            missing.join(",")
        )));
    }

    let mut total_equity_usd = 0.0;
    let mut total_available_balance_usd = 0.0;
    let mut valuations = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(price) = usd_price(&row.currency, &prices) else {
            continue;
        };
        let usd_value = row.total * price;
        total_equity_usd += usd_value;
        total_available_balance_usd += row.available * price;
        valuations.push(VenueAssetValuation {
            venue: SPOT_TRUTH_VENUE.to_owned(),
            currency: row.currency.clone(),
            usd_value,
            source: SPOT_VALUATION_SOURCE.to_owned(),
            observed_at_ms,
        });
    }
    let equity_scope = if is_consolidated_account(account_type) {
        AccountEquityScope::Unified
    } else {
        AccountEquityScope::Spot
    };
    let summary = VenueAccountSummary {
        venue: SPOT_TRUTH_VENUE.to_owned(),
        account_type: account_type.to_owned(),
        equity_scope,
        total_equity_usd,
        total_available_balance_usd,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: 0.0,
        total_maintenance_margin_usd: 0.0,
        account_im_rate: 0.0,
        account_mm_rate: 0.0,
        source: SPOT_VALUATION_SOURCE.to_owned(),
        observed_at_ms,
        freshness_ms: Some(0),
        problem: None,
    };
    Ok((summary, valuations))
}

fn spot_usd_prices(meta: &SpotMetaWrapper, contexts: &[AssetCtx]) -> BTreeMap<String, (u8, f64)> {
    let token_names = spot_token_names(&meta.tokens);
    let contexts_by_coin = spot_contexts_by_coin(contexts);
    let mut prices = BTreeMap::new();
    for token in &meta.tokens {
        let currency = token.name.trim().to_ascii_uppercase();
        if is_usd_pegged_settlement_currency(&currency) {
            prices.insert(currency, (0, 1.0));
        }
    }
    for (index, entry) in meta.universe.iter().enumerate() {
        let Some(context) =
            spot_context_for_entry(entry, index, contexts, contexts_by_coin.as_ref())
        else {
            continue;
        };
        let Some((base, quote)) = spot_pair(entry, &token_names) else {
            continue;
        };
        let Some(priority) = usd_quote_priority(&quote) else {
            continue;
        };
        let Some(price) = context
            .mark_px
            .parse::<f64>()
            .ok()
            .filter(|price| price.is_finite() && *price > 0.0)
        else {
            continue;
        };
        let replace = prices
            .get(&base)
            .is_none_or(|(current_priority, _)| priority < *current_priority);
        if replace {
            prices.insert(base, (priority, price));
        }
    }
    prices
}

fn usd_price(currency: &str, prices: &BTreeMap<String, (u8, f64)>) -> Option<f64> {
    let currency = currency.trim().to_ascii_uppercase();
    if is_usd_pegged_settlement_currency(&currency) {
        return Some(1.0);
    }
    prices.get(&currency).map(|(_, price)| *price)
}

fn usd_quote_priority(quote: &str) -> Option<u8> {
    match quote.trim().to_ascii_uppercase().as_str() {
        "USDC" => Some(1),
        "USDT" => Some(2),
        "USDT0" => Some(3),
        "USDH" => Some(4),
        "USD" => Some(5),
        _ => None,
    }
}

fn is_consolidated_account(account_type: &str) -> bool {
    matches!(
        account_type.trim().to_ascii_lowercase().as_str(),
        "unifiedaccount" | "portfoliomargin"
    )
}

fn account_abstractions() -> &'static Cache<String, AccountAbstraction> {
    ACCOUNT_ABSTRACTIONS.get_or_init(|| {
        Cache::builder()
            .max_capacity(32)
            .time_to_live(ACCOUNT_ABSTRACTION_TTL)
            .build()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> (SpotMetaWrapper, Vec<AssetCtx>) {
        serde_json::from_str(include_str!(
            "../../fixtures/hyperliquid/spot_meta_and_asset_ctxs_purr_hfun.json"
        ))
        .unwrap_or_else(|error| panic!("official spot fixture must decode: {error}"))
    }

    #[test]
    fn spot_account_summary_values_non_stable_assets_from_official_mark_prices() {
        let (meta, mut contexts) = context();
        contexts.reverse();
        let balances = HashMap::from([
            (
                "USDC".to_owned(),
                BalanceInfo {
                    currency: "USDC".to_owned(),
                    total: 10.0,
                    available: 8.0,
                    frozen: 2.0,
                    unrealized_pnl: 0.0,
                },
            ),
            (
                "PURR".to_owned(),
                BalanceInfo {
                    currency: "PURR".to_owned(),
                    total: 2.0,
                    available: 2.0,
                    frozen: 0.0,
                    unrealized_pnl: 0.0,
                },
            ),
        ]);

        let (summary, valuations) =
            spot_account_evidence(&balances, &meta, &contexts, "unifiedAccount", 7).unwrap_or_else(
                |error| panic!("officially priced spot balances must produce evidence: {error}"),
            );

        assert!((summary.total_equity_usd - 10.42).abs() < 1e-9);
        assert!((summary.total_available_balance_usd - 8.42).abs() < 1e-9);
        assert_eq!(summary.equity_scope, AccountEquityScope::Unified);
        assert_eq!(summary.account_type, "unifiedAccount");
        assert_eq!(valuations.len(), 2);
    }

    #[test]
    fn standard_account_keeps_spot_equity_separate_from_perp_margin() {
        let (meta, contexts) = context();
        let balances = HashMap::from([(
            "USDC".to_owned(),
            BalanceInfo {
                currency: "USDC".to_owned(),
                total: 10.0,
                available: 10.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            },
        )]);

        let (summary, _) = spot_account_evidence(&balances, &meta, &contexts, "default", 7)
            .unwrap_or_else(|error| panic!("priced standard spot balance must decode: {error}"));

        assert_eq!(summary.equity_scope, AccountEquityScope::Spot);
    }

    #[test]
    fn nonzero_unpriced_spot_asset_fails_closed() {
        let (meta, contexts) = context();
        let balances = HashMap::from([(
            "UNKNOWN".to_owned(),
            BalanceInfo {
                currency: "UNKNOWN".to_owned(),
                total: 1.0,
                available: 1.0,
                frozen: 0.0,
                unrealized_pnl: 0.0,
            },
        )]);

        let error = spot_account_evidence(&balances, &meta, &contexts, "default", 7)
            .expect_err("unpriced nonzero asset must not produce complete NAV evidence");

        assert!(error.to_string().contains("UNKNOWN"));
    }
}
