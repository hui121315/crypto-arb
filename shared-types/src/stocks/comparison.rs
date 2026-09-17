use super::*;
use rust_decimal::Decimal;
use std::str::FromStr;

pub const SOLANA_USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const STOCK_QUOTE_MAX_AGE_MS: i64 = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub struct StockDirectionEstimate {
    pub direction: &'static str,
    pub shares: Option<String>,
    pub gross_usdc: Option<String>,
    pub cex_notional_usdc: Option<String>,
    pub minimum_output: Option<String>,
    pub remainder_shares: Option<String>,
    pub blockers: Vec<String>,
}

pub fn positive(value: &str) -> Option<Decimal> {
    Decimal::from_str(value).ok().filter(|d| *d > Decimal::ZERO)
}

pub fn shares(raw: &str, mint: &StockMintEvidence) -> Option<Decimal> {
    let raw = raw.parse::<u64>().ok()?;
    let scale = Decimal::from(10_u64.checked_pow(u32::from(mint.decimals))?);
    Decimal::from(raw)
        .checked_div(scale)?
        .checked_mul(positive(&mint.ui_multiplier)?)
}

pub fn quote_current(quote: &StockDexQuote, now: i64) -> bool {
    now >= quote.requested_at_ms
        && now.saturating_sub(quote.requested_at_ms) <= STOCK_QUOTE_MAX_AGE_MS
        && quote.expires_at_ms.is_none_or(|expiry| now < expiry)
}

pub fn evaluate(snapshot: &StockMarketSnapshot, now: i64) -> Vec<StockDirectionEstimate> {
    let Some(c) = snapshot.comparison.as_ref() else {
        return vec![];
    };
    let market = snapshot
        .security
        .as_ref()
        .filter(|s| s.asset == c.asset)
        .and_then(|s| s.order_books.iter().find(|m| m.quote == "USDC"));
    let book = market.and_then(|m| snapshot.books.iter().find(|b| b.symbol == m.symbol));
    [true, false]
        .into_iter()
        .map(|chain_buy| {
            let mut row = StockDirectionEstimate {
                direction: if chain_buy {
                    "链买 / Backpack 卖"
                } else {
                    "Backpack 买 / 链卖"
                },
                shares: None,
                gross_usdc: None,
                cex_notional_usdc: None,
                minimum_output: None,
                remainder_shares: None,
                blockers: vec!["尚缺账户费率、Gas、可用库存预检；不是可执行净利润".into()],
            };
            let quote = if chain_buy {
                Some(&c.buy)
            } else {
                c.sell.as_ref()
            };
            let Some(q) = quote else {
                row.blockers.push(
                    c.sell_problem
                        .clone()
                        .unwrap_or_else(|| "缺少链上卖出报价".into()),
                );
                return row;
            };
            if !quote_current(q, now) {
                row.blockers.push("链上询价已陈旧，需重新询价".into());
            }
            if now < c.mint.checked_at_ms
                || now.saturating_sub(c.mint.checked_at_ms) > 60_000
                || c.mint.next_change_at_ms.is_some_and(|t| now >= t)
            {
                row.blockers.push("股数倍率需重新核验".into());
                return row;
            }
            let identities_match = if chain_buy {
                q.input_mint == SOLANA_USDC && q.output_mint == c.mint.address
            } else {
                q.input_mint == c.mint.address && q.output_mint == SOLANA_USDC
            };
            if !identities_match {
                row.blockers.push("报价合约不匹配".into());
                return row;
            }
            let Some(token_shares) = shares(
                if chain_buy {
                    &q.minimum_output_raw
                } else {
                    &q.input_raw
                },
                &c.mint,
            ) else {
                return row;
            };
            row.minimum_output = if chain_buy {
                Some(token_shares.normalize().to_string())
            } else {
                q.minimum_output_raw.parse::<u64>().ok().map(|n| {
                    (Decimal::from(n) / Decimal::from(1_000_000))
                        .normalize()
                        .to_string()
                })
            };
            let Some(route) = snapshot
                .trading_route
                .as_ref()
                .filter(|r| r.valid_until_ms > now)
            else {
                row.blockers.push(
                    snapshot
                        .trading_route
                        .as_ref()
                        .map(|r| {
                            if r.kind == StockRouteKind::Unknown {
                                r.reason.clone()
                            } else {
                                "官方交易通道状态已过期，等待日历重查".into()
                            }
                        })
                        .unwrap_or_else(|| "官方交易通道尚未核实".into()),
                );
                return row;
            };
            if route.kind == StockRouteKind::Rfq {
                let Some((quantity, price)) =
                    rfq_quote(snapshot, route, chain_buy, token_shares, now)
                else {
                    row.blockers.push(
                        "缺少当前有效、方向与股数匹配的私有 RFQ 报价；不能用参考价或订单簿替代"
                            .into(),
                    );
                    return row;
                };
                row.shares = Some(quantity.normalize().to_string());
                row.remainder_shares =
                    Some((quantity - token_shares).abs().normalize().to_string());
                row.blockers
                    .push("RFQ taker 价已含报价费；未接受，不是成交或已锁定利润".into());
                finish_difference(&mut row, q, quantity, price, chain_buy, now);
                return row;
            }
            if route.kind != StockRouteKind::OrderBook {
                row.blockers.push(route.reason.clone());
                return row;
            }
            let (Some(m), Some(b)) = (market, book) else {
                row.blockers
                    .push("没有同一 USDC 订单簿报价；需另取 RFQ，不能用参考价替代".into());
                return row;
            };
            if route.symbol.as_deref() != Some(m.symbol.as_str()) {
                row.blockers.push("交易通道与当前订单簿不一致".into());
                return row;
            }
            if !snapshot.connected
                || snapshot.problem.is_some()
                || now < b.source_at_ms
                || now.saturating_sub(b.source_at_ms) > 3_000
            {
                row.blockers.push("交易所盘口已失效，差额不再计算".into());
                return row;
            }
            if m.state != "Open" {
                row.blockers.push("股票订单簿当前未开放".into());
                return row;
            }
            let (Some(step), Some(minimum)) = (positive(&m.step_size), positive(&m.min_quantity))
            else {
                return row;
            };
            let Some(lots) = token_shares.checked_div(step) else {
                return row;
            };
            // Do not count unsellable dust as revenue, or buy less than the chain sell consumes.
            let Some(quantity) =
                (if chain_buy { lots.floor() } else { lots.ceil() }).checked_mul(step)
            else {
                return row;
            };
            row.shares = Some(quantity.normalize().to_string());
            row.remainder_shares = Some((quantity - token_shares).abs().normalize().to_string());
            if quantity < minimum {
                row.blockers
                    .push("该金额不足交易所最小股数；不能对齐两边数量".into());
                return row;
            }
            let price = (if chain_buy { &b.bid } else { &b.ask })
                .as_deref()
                .and_then(positive);
            let depth = (if chain_buy {
                &b.bid_quantity
            } else {
                &b.ask_quantity
            })
            .as_deref()
            .and_then(positive);
            let (Some(price), Some(depth)) = (price, depth) else {
                row.blockers.push("对应买卖一档缺失".into());
                return row;
            };
            if depth < quantity {
                row.blockers.push("一档数量不足，需要执行前深度报价".into());
                return row;
            }
            finish_difference(&mut row, q, quantity, price, chain_buy, now);
            row
        })
        .collect()
}

fn rfq_quote(
    snapshot: &StockMarketSnapshot,
    route: &StockTradingRoute,
    chain_buy: bool,
    token_shares: Decimal,
    now: i64,
) -> Option<(Decimal, Decimal)> {
    let s = snapshot.security.as_ref()?;
    if route.symbol.as_deref() != Some(&s.rfq_symbol) || snapshot.rfq_problem.is_some() {
        return None;
    }
    let session = route.session.as_ref()?;
    let min = positive(&session.min_quantity)?;
    let step = positive(&session.step_size)?;
    let side = if chain_buy {
        StockRfqSide::Ask
    } else {
        StockRfqSide::Bid
    };
    snapshot
        .rfqs
        .iter()
        .filter(|r| {
            r.request.asset == s.asset && r.symbol == s.rfq_symbol && r.request.side == side
        })
        .filter_map(|r| {
            let candidate = r.current_candidate(snapshot.rfq_connected, now)?;
            let quantity = positive(&r.request.quantity)?;
            if quantity < min
                || !quantity.checked_rem(step)?.is_zero()
                || session
                    .max_quantity
                    .as_deref()
                    .is_some_and(|max| positive(max).is_none_or(|max| quantity > max))
                || (chain_buy && quantity > token_shares)
                || (!chain_buy && quantity < token_shares)
            {
                return None;
            }
            Some((quantity, positive(&candidate.taker_price)?))
        })
        .min_by_key(|(quantity, _)| (*quantity - token_shares).abs())
}

fn finish_difference(
    row: &mut StockDirectionEstimate,
    q: &StockDexQuote,
    quantity: Decimal,
    price: Decimal,
    chain_buy: bool,
    now: i64,
) {
    let Some(notional) = quantity.checked_mul(price) else {
        return;
    };
    let Some(usdc_raw) = (if chain_buy {
        &q.input_raw
    } else {
        &q.minimum_output_raw
    })
    .parse::<u64>()
    .ok() else {
        return;
    };
    let usdc = Decimal::from(usdc_raw) / Decimal::from(1_000_000);
    let gross = if chain_buy {
        notional.checked_sub(usdc)
    } else {
        usdc.checked_sub(notional)
    };
    if quote_current(q, now) {
        row.cex_notional_usdc = Some(notional.normalize().to_string());
        row.gross_usdc = gross.map(|g| g.normalize().to_string());
    }
    if q.fee_bps.is_none() || q.fee_mint.is_none() {
        row.blockers.push("Jupiter 费用字段未完整返回".into());
    }
}

#[cfg(test)]
pub(super) mod tests;
