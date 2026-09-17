use super::*;
use crate::api::rest::{HistoryResponse, OpportunityHistoryRow};
use crate::panels::modules::market_evidence::retry_label;
use crate::panels::modules::opportunity_format::missing_quote_label;
use crate::state::section::problem_context;
use shared_types::{ApiProblem, OrderBookInfo, RowCapEvidence};

pub(in crate::panels::modules::opportunities) fn book_line(
    book: OrderBookInfo,
    health: String,
) -> BookLine {
    BookLine {
        bid: price(book.best_bid()),
        ask: price(book.best_ask()),
        spread: spread(book.spread()),
        venue: book.exchange,
        health,
    }
}

pub(in crate::panels::modules::opportunities) fn empty_book_line(
    venue: &str,
    health: String,
) -> BookLine {
    BookLine {
        venue: venue.into(),
        bid: missing_quote_label().into(),
        ask: missing_quote_label().into(),
        spread: "待价差".into(),
        health,
    }
}

pub(in crate::panels::modules::opportunities) fn with_problem_context_label(
    base: &str,
    problem: &ApiProblem,
) -> String {
    problem_context(problem)
        .map_or_else(|| base.to_owned(), |context| format!("{base} · {context}"))
}

pub(in crate::panels::modules::opportunities) fn history_lines(
    resp: HistoryResponse<OpportunityHistoryRow>,
    health: &str,
) -> Vec<HistoryLine> {
    resp.rows
        .into_iter()
        .map(|row| HistoryLine {
            time: minutes_ago(row.occurred_at_ms),
            route: format!("{} / {}", row.long_exchange, row.short_exchange),
            edge: format!("{:.3}%", row.net_yield * 100.0),
            health: health.to_owned(),
        })
        .collect()
}

pub(in crate::panels::modules::opportunities) fn history_health_label<T>(
    resp: &HistoryResponse<T>,
) -> String {
    if let Some(problem) = first_history_problem(resp) {
        let label = format!("{} · {}", history_source(&resp.source), problem.message);
        let label = with_row_cap_label(&label, resp.row_cap.as_ref());
        let label = with_problem_context_label(&label, &problem);
        return if problem.retry_after_ms.is_some() {
            label
        } else {
            retry_label(&label, resp.retry_after_ms)
        };
    }
    let base = format!("{} · {}条", history_source(&resp.source), resp.count);
    let label = resp.freshness_ms.map_or(base.clone(), |freshness| {
        format!("{base} · {}", duration_label(freshness))
    });
    let label = with_row_cap_label(&label, resp.row_cap.as_ref());
    retry_label(&label, resp.retry_after_ms)
}

pub(in crate::panels::modules::opportunities) fn with_row_cap_label(
    base: &str,
    row_cap: Option<&RowCapEvidence>,
) -> String {
    row_cap.map_or_else(
        || base.to_owned(),
        |cap| format!("{base} · {}", row_cap_label(cap)),
    )
}

pub(in crate::panels::modules::opportunities) fn row_cap_label(cap: &RowCapEvidence) -> String {
    let total = if cap.total_rows_is_lower_bound {
        format!("至少{}条", cap.total_rows)
    } else {
        format!("{}条", cap.total_rows)
    };
    let truncation = if cap.truncated {
        format!("已截断至少{}条", cap.truncated_count)
    } else {
        "未截断".into()
    };
    format!(
        "行证据 {} 返回 {}/{} 上限 {} {}",
        cap.source, cap.returned_count, total, cap.max_rows, truncation
    )
}

pub(in crate::panels::modules::opportunities) fn first_history_problem<T>(
    resp: &HistoryResponse<T>,
) -> Option<ApiProblem> {
    resp.problem
        .clone()
        .or_else(|| resp.problems.first().cloned())
        .or_else(|| {
            resp.storage_health
                .as_ref()
                .and_then(|health| health.problem.clone())
        })
}

pub(in crate::panels::modules::opportunities) fn missing_orderbook_problem(
    health: &shared_types::MarketDataHealth,
    venue: &str,
) -> ApiProblem {
    health.problem.clone().unwrap_or_else(|| {
        ApiProblem::new(
            "MARKET_DATA_MISSING",
            format!("{venue} orderbook data missing"),
        )
        .with_retry_after_ms(health.retry_after_ms)
        .with_source("opportunity-detail")
    })
}

pub(in crate::panels::modules::opportunities) fn orderbook_is_deferred(
    envelope: &shared_types::MarketDataEnvelope<Option<OrderBookInfo>>,
) -> bool {
    envelope.data.is_none()
        && envelope.health.quality == shared_types::MarketDataQuality::Unverified
        && envelope.health.source == shared_types::MarketDataSourceKind::LocalCache
        && envelope.health.problem.is_none()
        && envelope
            .health
            .coverage
            .as_ref()
            .is_some_and(|coverage| coverage.requested == 0)
}

pub(in crate::panels::modules::opportunities) fn history_source(source: &str) -> &str {
    match source {
        "memory" => "历史 memory",
        "postgres" => "历史 postgres",
        "disabled" => "历史关闭",
        "" => "历史未知",
        _ => source,
    }
}

pub(in crate::panels::modules::opportunities) fn price(value: Option<f64>) -> String {
    value
        .filter(|price| price.is_finite() && *price > f64::EPSILON)
        .map_or_else(
            || missing_quote_label().into(),
            |price| format!("{price:.4}"),
        )
}

pub(in crate::panels::modules::opportunities) fn spread(value: Option<f64>) -> String {
    value
        .filter(|spread| spread.is_finite() && *spread >= 0.0)
        .map_or_else(|| "待价差".into(), |spread| format!("{spread:.4}"))
}

pub(in crate::panels::modules::opportunities) fn duration_label(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        format!("{:.1}m", ms as f64 / 60_000.0)
    }
}

pub(in crate::panels::modules::opportunities) fn minutes_ago(ms: i64) -> String {
    let mins = ((js_sys::Date::now() as i64 - ms).max(0) / 60_000).max(1);
    if mins < 60 {
        format!("{mins}m")
    } else {
        format!("{:.1}h", mins as f64 / 60.0)
    }
}
