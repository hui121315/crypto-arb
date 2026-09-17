use super::*;
use rust_decimal::{prelude::ToPrimitive, Decimal};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingAsset {
    pub asset: String,
    pub tokens: Vec<StockChainToken>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockDepositAddress {
    pub asset: String,
    pub address: String,
    pub blockchain: String,
    pub account_fingerprint: String,
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockDepositAddressRequest {
    pub asset: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingDirection {
    pub direction: StockChainDirection,
    pub needs: Vec<StockFundingNeed>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockFundingNeed {
    pub asset: String,
    pub target: String,
    pub source: String,
    pub required: Option<String>,
    pub available: Option<String>,
    pub shortfall: Option<String>,
    pub source_available: Option<String>,
    pub source_spare: Option<String>,
    // Asset units, not USD. This is an inventory check, never a transfer authorization.
    pub conservative_source_budget: Option<String>,
    pub source_sufficient: Option<bool>,
    pub token: Option<StockChainToken>,
    pub metadata_at_ms: Option<i64>,
    pub blockers: Vec<String>,
}

fn decimal(s: &str) -> Option<Decimal> {
    Decimal::from_str_exact(s)
        .ok()
        .filter(|n| *n >= Decimal::ZERO)
}
fn amount(n: Decimal) -> String {
    n.normalize().to_string()
}
fn fresh(at: i64, now: i64, age: i64) -> bool {
    at > 0 && now >= at && now.saturating_sub(at) <= age
}
fn canonical(asset: &str) -> &str {
    match asset {
        "SOL / 保守周转余额" => "SOL",
        "USDC / SOL 补仓" => "USDC",
        _ => asset,
    }
}

pub fn evaluate_funding(
    s: &StockMarketSnapshot,
    rows: &[StockPreflightDirection],
    account: Option<&StockAccountEvidence>,
    wallet: Option<&StockWalletEvidence>,
    now: i64,
) -> Vec<StockFundingDirection> {
    let Some(c) = s.comparison.as_ref() else {
        return vec![];
    };
    let mint_current = fresh(c.mint.checked_at_ms, now, 60_000)
        && c.mint.next_change_at_ms.is_none_or(|at| now < at);
    let account = account.filter(|a| fresh(a.balances_at_ms, now, 30_000) && !a.liquidating);
    let wallet = wallet.filter(|w| {
        w.mint == c.mint.address && w.problems.is_empty() && fresh(w.checked_at_ms, now, 30_000)
    });
    let balances = |location: &str, asset: &str| -> Option<Decimal> {
        if location == "Backpack" {
            let b = account?.balances.get(asset)?;
            return fresh(b.observed_at_ms, now, 30_000)
                .then(|| decimal(&b.available))
                .flatten();
        }
        let w = wallet?;
        match asset {
            "USDC" => decimal(w.usdc_raw.as_deref()?)?.checked_div(Decimal::from(1_000_000)),
            "SOL" => decimal(w.sol_lamports.as_deref()?)?.checked_div(Decimal::from(1_000_000_000)),
            asset if asset == c.asset && mint_current => {
                comparison::shares(w.stock_raw.as_deref()?, &c.mint)
            }
            _ => None,
        }
    };
    rows.iter()
        .filter_map(|r| {
            let direction = [StockChainDirection::Buy, StockChainDirection::Sell]
                .into_iter()
                .find(|d| d.label() == r.direction)?;
            let mut needs = vec![];
            for i in &r.inventory {
                let asset = canonical(&i.asset);
                let required = i.required.as_deref().and_then(decimal);
                let available = balances(&i.location, asset);
                let shortfall = required
                    .zip(available)
                    .and_then(|(q, a)| q.checked_sub(a))
                    .map(|n| n.max(Decimal::ZERO));
                if shortfall == Some(Decimal::ZERO) {
                    continue;
                }
                let source = if i.location == "Backpack" {
                    "Solana"
                } else {
                    "Backpack"
                };
                // Keep this direction's trading reserves in place; do not fund a deficit by creating another.
                let reserved = r
                    .inventory
                    .iter()
                    .filter(|j| j.location == source && canonical(&j.asset) == asset)
                    .try_fold(Decimal::ZERO, |n, j| {
                        n.checked_add(decimal(j.required.as_deref()?)?)
                    });
                let source_available = balances(source, asset);
                let source_spare = source_available
                    .zip(reserved)
                    .and_then(|(a, q)| a.checked_sub(q))
                    .map(|n| n.max(Decimal::ZERO));
                let mut blockers = vec![];
                if shortfall.is_none() {
                    blockers.push("所需数量或目标余额未知，不能把缺失余额当零".into());
                }
                if source_spare.is_none() {
                    blockers.push("来源余额或本方向备款未知，不能预支套利后的到账".into());
                }
                let metadata_current = s.token_metadata_problem.is_none()
                    && s.token_metadata_at_ms
                        .is_some_and(|at| fresh(at, now, 30_000));
                let rows = if asset == c.asset {
                    Some(&s.tokens)
                } else {
                    s.funding_assets
                        .iter()
                        .find(|a| a.asset == asset)
                        .map(|a| &a.tokens)
                };
                let mut matching = rows
                    .into_iter()
                    .flatten()
                    .filter(|t| t.blockchain == "Solana");
                let token = matching
                    .next()
                    .filter(|t| {
                        matching.next().is_none()
                            && match asset {
                                "USDC" => {
                                    t.contract_address.as_deref() == Some(comparison::SOLANA_USDC)
                                        && t.native_decimals == Some(6)
                                }
                                "SOL" => {
                                    t.contract_address.as_deref() == Some("So1")
                                        && t.native_decimals == Some(9)
                                }
                                _ => {
                                    t.contract_address.as_deref() == Some(&c.mint.address)
                                        && t.native_decimals == Some(c.mint.decimals)
                                }
                            }
                    })
                    .filter(|_| metadata_current)
                    .cloned();
                if token.is_none() {
                    blockers.push("缺少新鲜的同链同合约充提证据".into());
                }
                let mut budget = None;
                if let (Some(t), Some(shortfall)) = (&token, shortfall) {
                    let to_cex = i.location == "Backpack";
                    let enabled = if to_cex {
                        t.deposit_enabled
                    } else {
                        t.withdraw_enabled
                    };
                    if enabled != Some(true) {
                        blockers.push(
                            if to_cex {
                                "Backpack 该资产充值未开放或未知"
                            } else {
                                "Backpack 该资产提现未开放或未知"
                            }
                            .into(),
                        );
                    }
                    let min = if to_cex {
                        &t.minimum_deposit
                    } else {
                        &t.minimum_withdrawal
                    }
                    .as_deref()
                    .and_then(decimal);
                    if min.is_none() {
                        blockers.push("官方最低充提量未知".into());
                    }
                    if to_cex {
                        budget = min.map(|min| shortfall.max(min));
                        let multiplier = if asset == c.asset {
                            mint_current
                                .then(|| comparison::positive(&c.mint.ui_multiplier))
                                .flatten()
                        } else {
                            Some(Decimal::ONE)
                        };
                        budget = budget.zip(multiplier).and_then(|(n, multiplier)| {
                            let scale =
                                Decimal::from(10_u64.checked_pow(u32::from(t.native_decimals?))?);
                            let raw = n
                                .checked_div(multiplier)?
                                .checked_mul(scale)?
                                .ceil()
                                .to_u64()?;
                            Decimal::from(raw)
                                .checked_div(scale)?
                                .checked_mul(multiplier)
                        });
                        blockers.push("转账 Gas、到账确认及实际入账仍须核验".into());
                    } else {
                        let fee = t.withdrawal_fee.as_deref().and_then(decimal);
                        if fee.is_none() {
                            blockers.push("提现手续费未知，不能当零".into());
                        }
                        // Cover both fee-included and fee-added interpretations until a withdrawal quote proves the debit.
                        budget = min.zip(fee).and_then(|(min, fee)| {
                            shortfall.checked_add(fee)?.max(min).checked_add(fee)
                        });
                        if t.maximum_withdrawal.as_deref().is_some_and(|v| {
                            decimal(v).is_none_or(|max| budget.is_none_or(|n| n > max))
                        }) {
                            blockers.push("保守备款超过官方单笔上限或上限不可解析".into());
                        }
                        blockers.push(
                            "须核实提现数量含费口径、地址白名单/2FA 和到账；未生成提币请求".into(),
                        );
                    }
                }
                let sufficient = source_spare.zip(budget).map(|(a, q)| a >= q);
                if shortfall.is_some() && budget.is_none() {
                    blockers.push("补库数量、手续费或精度尚不能安全计算".into());
                }
                if sufficient == Some(false) {
                    blockers.push("来源可调余额不足，需外部补入；不使用另一腿预期收入垫资".into());
                }
                if asset == "USDC"
                    && sufficient != Some(true)
                    && source == "Backpack"
                    && balances("Backpack", "USDT").is_some_and(|n| n > Decimal::ZERO)
                {
                    blockers.push(
                        "来源有 USDT；需单独取得 USDT→USDC 兑换报价，不能按 1:1 计入备款".into(),
                    );
                }
                needs.push(StockFundingNeed {
                    asset: asset.into(),
                    target: i.location.clone(),
                    source: source.into(),
                    required: required.map(amount),
                    available: available.map(amount),
                    shortfall: shortfall.map(amount),
                    source_available: source_available.map(amount),
                    source_spare: source_spare.map(amount),
                    conservative_source_budget: budget.map(amount),
                    source_sufficient: sufficient,
                    token,
                    metadata_at_ms: metadata_current.then_some(s.token_metadata_at_ms).flatten(),
                    blockers,
                });
            }
            Some(StockFundingDirection { direction, needs })
        })
        .collect()
}

#[cfg(test)]
mod tests;
