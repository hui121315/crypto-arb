use super::*;
use shared_types::{p0_hedge_leg_product, FeeProduct, HedgeTicket, OrderBookInfo};

pub(super) fn orderbook_freshness_rejection(
    state: &AppState,
    ticket: &HedgeTicket,
    prefix: &str,
) -> Option<String> {
    let now_ms = common::time::now_ms();
    let blockers = [&ticket.long_leg, &ticket.short_leg]
        .into_iter()
        .filter_map(|leg| {
            let read = leg_orderbook_read(state, ticket, leg, now_ms);
            orderbook_recheck_blocker(leg, read.as_ref())
        })
        .collect::<Vec<_>>();
    (!blockers.is_empty()).then(|| format!("{prefix}: {}", blockers.join("; ")))
}

fn leg_orderbook_read(
    state: &AppState,
    ticket: &HedgeTicket,
    leg: &HedgeLegQuote,
    now_ms: i64,
) -> Option<crate::services::market_data::MarketRead<OrderBookInfo>> {
    match ticket_leg_product(ticket, leg) {
        Some(FeeProduct::Spot) => Some(state.market_data().spot_orderbook_read(
            &leg.exchange,
            &leg.symbol,
            now_ms,
        )),
        Some(FeeProduct::Perp) => Some(state.market_data().orderbook_read(
            &leg.exchange,
            &leg.symbol,
            now_ms,
        )),
        Some(FeeProduct::Margin | FeeProduct::Unknown) | None => None,
    }
}

fn ticket_leg_product(ticket: &HedgeTicket, leg: &HedgeLegQuote) -> Option<FeeProduct> {
    p0_hedge_leg_product(ticket.strategy, ticket.spot_leg_mode, leg.role)
}

fn orderbook_recheck_blocker(
    leg: &HedgeLegQuote,
    read: Option<&crate::services::market_data::MarketRead<OrderBookInfo>>,
) -> Option<String> {
    match read {
        Some(read) => orderbook_read_blocker(leg, read),
        None => Some(format!(
            "{} {} 执行产品未解析，等待现货腿方向证据",
            leg.exchange, leg.symbol
        )),
    }
}

pub(super) fn orderbook_read_blocker(
    leg: &HedgeLegQuote,
    read: &crate::services::market_data::MarketRead<OrderBookInfo>,
) -> Option<String> {
    if read.quality == crate::services::market_data::MarketQuality::Fresh && read.value.is_some() {
        return None;
    }
    Some(match read.quality {
        crate::services::market_data::MarketQuality::Fresh => {
            format!("{} {} 执行前盘口复检为空", leg.exchange, leg.symbol)
        }
        crate::services::market_data::MarketQuality::Warming => {
            format!("{} {} 执行前盘口 WS 仍在等待首帧", leg.exchange, leg.symbol)
        }
        crate::services::market_data::MarketQuality::StaleAllowed => format!(
            "{} {} 执行前盘口仅有旧缓存({})，等待 fresh orderbook",
            leg.exchange,
            leg.symbol,
            display_millis(read.freshness_ms)
        ),
        crate::services::market_data::MarketQuality::RateLimited => format!(
            "{} {} 执行前盘口限频退避，{}后重试",
            leg.exchange,
            leg.symbol,
            display_retry_delay(read.retry_after_ms)
        ),
        crate::services::market_data::MarketQuality::CircuitOpen => {
            format!("{} {} 执行前盘口交易所熔断中", leg.exchange, leg.symbol)
        }
        crate::services::market_data::MarketQuality::Unsupported => {
            format!("{} {} 执行前盘口接口不支持", leg.exchange, leg.symbol)
        }
        crate::services::market_data::MarketQuality::Missing => {
            format!("{} {} 执行前盘口暂无 fresh 数据", leg.exchange, leg.symbol)
        }
    })
}

fn display_millis(value: Option<i64>) -> String {
    value
        .map(|ms| format!("{}ms", ms.max(0)))
        .unwrap_or_else(|| "未知".to_owned())
}

fn display_retry_delay(value: Option<i64>) -> String {
    value
        .map(|ms| format!("{}ms", ms.max(0)))
        .unwrap_or_else(|| "未知时间".to_owned())
}
