//! arbitrage REST 的路径构造、查询编码与市场数据 problem 收敛。
//! 客户端方法见父模块 `arbitrage.rs`。

use super::super::*;

const OPPORTUNITIES_LIST_PATH: &str = "/api/v3/arbitrage/opportunities/list";
pub(super) const INDEX_COMPOSITIONS_ENVELOPE_PATH: &str = "/api/venues/index-compositions/envelope";

pub(super) fn opportunity_list_path(
    page_size: usize,
    symbol: Option<&str>,
    cursor: Option<&str>,
) -> String {
    let strategies = p0_strategy_query_param();
    opportunity_list_path_with_strategies(page_size, symbol, cursor, &strategies)
}

pub(super) fn opportunity_list_path_for_strategy(
    page_size: usize,
    symbol: Option<&str>,
    cursor: Option<&str>,
    strategy: shared_types::StrategyKind,
) -> String {
    opportunity_list_path_with_strategies(page_size, symbol, cursor, strategy.as_query_value())
}

fn opportunity_list_path_with_strategies(
    page_size: usize,
    symbol: Option<&str>,
    cursor: Option<&str>,
    strategies: &str,
) -> String {
    let symbol = symbol
        .map(|value| format!("&symbol={}", encode_query_component(value)))
        .unwrap_or_default();
    let cursor = cursor
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("&cursor={}", encode_query_component(value)))
        .unwrap_or_default();
    format!(
        "{OPPORTUNITIES_LIST_PATH}?pageSize={page_size}&fast=true&sortKey=net_single_yield{symbol}{cursor}&strategy={strategies}"
    )
}

pub(super) fn opportunity_detail_path(id: &str) -> String {
    format!(
        "/api/v3/arbitrage/opportunities/{}/detail?historyLimit=6",
        encode_path_segment(id)
    )
}

pub(super) fn hedge_preview_path(opportunity_id: &str) -> String {
    let opportunity_id = encode_path_segment(opportunity_id);
    format!("/api/arbitrage/opportunities/{opportunity_id}/preview")
}

pub(super) fn hedge_confirm_path(opportunity_id: &str) -> String {
    let opportunity_id = encode_path_segment(opportunity_id);
    format!("/api/arbitrage/opportunities/{opportunity_id}/confirm")
}

pub(super) fn orderbook_path(venue: &str, symbol: &str, depth: usize) -> String {
    let venue = encode_path_segment(venue);
    let symbol = encode_query_component(symbol);
    format!(
        "/api/exchanges/{venue}/orderbook?symbol={symbol}&depth={}",
        depth.max(1)
    )
}

fn p0_strategy_query_param() -> String {
    shared_types::P0_EXECUTABLE_STRATEGY_KINDS
        .into_iter()
        .map(shared_types::StrategyKind::as_query_value)
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn orderbook_problem(
    health: shared_types::MarketDataHealth,
    venue: &str,
    symbol: &str,
) -> ApiError {
    market_data_problem(health, venue, symbol, "orderbook")
}

pub(super) fn market_data_problem(
    health: shared_types::MarketDataHealth,
    venue: &str,
    symbol: &str,
    operation: &str,
) -> ApiError {
    health.problem.map_or_else(
        || {
            ApiError::client(
                "MARKET_DATA_MISSING",
                format!("{venue} {symbol} {operation} unavailable"),
            )
        },
        ApiError::from_problem,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        hedge_confirm_path, hedge_preview_path, opportunity_detail_path, opportunity_list_path,
        opportunity_list_path_for_strategy, orderbook_path, p0_strategy_query_param,
        INDEX_COMPOSITIONS_ENVELOPE_PATH,
    };

    #[test]
    fn futures_strategy_param_uses_shared_p0_allowlist() {
        assert_eq!(
            p0_strategy_query_param(),
            "perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross"
        );
    }

    #[test]
    fn index_composition_list_uses_envelope_endpoint() {
        assert_eq!(
            INDEX_COMPOSITIONS_ENVELOPE_PATH,
            "/api/venues/index-compositions/envelope"
        );
    }

    #[test]
    fn opportunity_list_path_uses_server_window_contract() {
        assert_eq!(
            opportunity_list_path(120, Some("MU"), Some("240")),
            "/api/v3/arbitrage/opportunities/list?pageSize=120&fast=true&sortKey=net_single_yield&symbol=MU&cursor=240&strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross"
        );
    }

    #[test]
    fn futures_strategy_path_scopes_server_pagination_to_active_tab() {
        assert_eq!(
            opportunity_list_path_for_strategy(
                50,
                None,
                Some("50"),
                shared_types::StrategyKind::PerpCross,
            ),
            "/api/v3/arbitrage/opportunities/list?pageSize=50&fast=true&sortKey=net_single_yield&cursor=50&strategy=perp_cross"
        );
    }

    #[test]
    fn opportunity_detail_path_uses_combined_endpoint() {
        let path = opportunity_detail_path("opp:MU/1");
        let route = path
            .trim_end_matches("?historyLimit=6")
            .replace("opp%3AMU%2F1", ":id");

        assert!(path.contains("opp%3AMU%2F1"));
        assert!(path.ends_with("/detail?historyLimit=6"));
        assert!(!path.contains("depth="));
        assert_eq!(route, "/api/v3/arbitrage/opportunities/:id/detail");
    }

    #[test]
    fn opportunity_mutation_paths_encode_the_identifier_as_one_segment() {
        let id = "opp/MU?mode=live#leg %";

        assert_eq!(
            hedge_preview_path(id),
            "/api/arbitrage/opportunities/opp%2FMU%3Fmode%3Dlive%23leg%20%25/preview"
        );
        assert_eq!(
            hedge_confirm_path(id),
            "/api/arbitrage/opportunities/opp%2FMU%3Fmode%3Dlive%23leg%20%25/confirm"
        );
    }

    #[test]
    fn orderbook_path_separates_path_and_query_components() {
        assert_eq!(
            orderbook_path("venue/a?x", "BTC/USDT & perp", 0),
            "/api/exchanges/venue%2Fa%3Fx/orderbook?symbol=BTC%2FUSDT%20%26%20perp&depth=1"
        );
    }

    #[test]
    fn opportunity_list_path_encodes_dynamic_query_values() {
        assert_eq!(
            opportunity_list_path(20, Some("BTC/USDT & perp"), Some("next/page?1")),
            "/api/v3/arbitrage/opportunities/list?pageSize=20&fast=true&sortKey=net_single_yield&symbol=BTC%2FUSDT%20%26%20perp&cursor=next%2Fpage%3F1&strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross"
        );
    }
}
