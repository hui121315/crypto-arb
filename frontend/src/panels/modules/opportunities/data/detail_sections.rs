use super::*;
use crate::api::rest::{ApiError, HistoryResponse, OpportunityHistoryRow};
use crate::panels::modules::index_composition::IndexCompositionSnapshotView;
use crate::panels::modules::market_evidence::{market_health_label, market_quality_label, market_source_label};
use leptos::prelude::*;
use shared_types::{ApiProblem, IndexCompositionSnapshot, MarketDataEnvelope, OrderBookInfo};

pub(in crate::panels::modules::opportunities) fn capture_history(
    result: Result<HistoryResponse<OpportunityHistoryRow>, ApiError>,
    symbol: &str,
    request_id: Option<&str>,
    problem: &mut Option<ApiProblem>,
) -> (Vec<HistoryLine>, String, DetailEvidence) {
    match result {
        Ok(response) => {
            if let Some(history_problem) = first_history_problem(&response) {
                problem.get_or_insert(history_problem);
            }
            let mut evidence = history_section_evidence(history_section(symbol), &response, request_id);
            if response.rows.is_empty() && !evidence.retain_on_failure {
                evidence.status = if response.source == "disabled" { "历史未启用" } else { "暂无历史记录" }.into();
            }
            let health = history_health_label(&response);
            (history_lines(response, &health), health, evidence)
        }
        Err(error) => {
            let health = with_problem_context_label(
                &format!("历史错误 · {}", error.problem.message),
                &error.problem,
            );
            let evidence = error_section_evidence(history_section(symbol), &error.problem);
            problem.get_or_insert(error.problem);
            (Vec::new(), health, evidence)
        }
    }
}

pub(in crate::panels::modules::opportunities) fn capture_index(
    result: Result<MarketDataEnvelope<Option<IndexCompositionSnapshot>>, ApiError>,
    role: &str,
    venue: &str,
    symbol: &str,
    request_id: Option<&str>,
    problem: &mut Option<ApiProblem>,
) -> (IndexCompositionSnapshotView, DetailEvidence) {
    match result {
        Ok(envelope) => {
            let mut evidence =
                market_section_evidence(index_section(role, venue), &envelope.health, request_id);
            if envelope.data.is_none() && envelope.health.quality == shared_types::MarketDataQuality::Fresh {
                evidence.status = "未返回指数数据".into();
                evidence.freshness = EvidenceAge::new(None);
            }
            let health = market_health_label(&envelope.health);
            let health = with_row_cap_label(&health, envelope.row_cap.as_ref());
            let health = match envelope.health.problem.as_ref() {
                Some(problem) => with_problem_context_label(&health, problem),
                None => health,
            };
            if let Some(market_problem) = envelope.health.problem.clone() {
                problem.get_or_insert(market_problem);
            }
            if let Some(snapshot) = envelope.data {
                (index_snapshot_view(role, snapshot, &health), evidence)
            } else {
                let message = envelope
                    .health
                    .last_error
                    .clone()
                    .or_else(|| {
                        envelope
                            .health
                            .problem
                            .as_ref()
                            .map(|problem| problem.message.clone())
                    })
                    .unwrap_or_else(|| "指数成分不可用".into());
                (
                    IndexCompositionSnapshotView::unavailable(role, venue, symbol, health, message),
                    evidence,
                )
            }
        }
        Err(error) => {
            let message = error.problem.message.clone();
            let health =
                with_problem_context_label(&format!("接口错误 · {message}"), &error.problem);
            let evidence = error_section_evidence(index_section(role, venue), &error.problem);
            problem.get_or_insert(error.problem);
            (
                IndexCompositionSnapshotView::unavailable(role, venue, symbol, health, message),
                evidence,
            )
        }
    }
}

pub(in crate::panels::modules::opportunities) fn index_snapshot_view(
    role: &str,
    snapshot: IndexCompositionSnapshot,
    read_health: &str,
) -> IndexCompositionSnapshotView {
    let mut view = IndexCompositionSnapshotView::from_snapshot(role, snapshot);
    view.health = format!("{} · 读取 {}", view.health, read_health);
    view
}

pub(in crate::panels::modules::opportunities) fn capture_book(
    result: Result<shared_types::MarketDataEnvelope<Option<OrderBookInfo>>, ApiError>,
    role: &str,
    fallback_venue: &str,
    request_id: Option<&str>,
) -> (BookLine, DetailEvidence) {
    match result {
        Ok(envelope) => {
            let mut evidence = market_section_evidence(
                orderbook_section(role, fallback_venue),
                &envelope.health,
                request_id,
            );
            let base_health = market_health_label(&envelope.health);
            let base_health = with_row_cap_label(&base_health, envelope.row_cap.as_ref());
            if orderbook_is_deferred(&envelope) {
                evidence.status = "构建时核对 · 未读取".into();
                evidence.retain_on_failure = false;
                evidence.freshness = EvidenceAge::new(None);
                (
                    empty_book_line(fallback_venue, "构建时核对 · 未主动读取盘口".into()),
                    evidence,
                )
            } else if let Some(book) = envelope.data {
                let health = match envelope.health.problem.as_ref() {
                    Some(problem) => with_problem_context_label(&base_health, problem),
                    None => base_health,
                };
                let row = book_line(book, health);
                if row.bid == crate::panels::modules::opportunity_format::missing_quote_label()
                    || row.ask == crate::panels::modules::opportunity_format::missing_quote_label() {
                    evidence.status = "双边报价不完整".into();
                }
                (row, evidence)
            } else if envelope.health.quality == shared_types::MarketDataQuality::Unsupported {
                (empty_book_line(fallback_venue, "不支持读取盘口".into()), evidence)
            } else {
                let missing_problem = missing_orderbook_problem(&envelope.health, fallback_venue);
                evidence.status = format!("{} · 未返回盘口", market_quality_label(envelope.health.quality));
                if envelope.health.quality == shared_types::MarketDataQuality::Fresh {
                    evidence.status = "盘口数据缺失".into();
                }
                evidence.freshness = EvidenceAge::new(None);
                evidence.problem = Some(missing_problem.clone());
                let message_health = if envelope.health.problem.is_some() {
                    base_health
                } else {
                    format!("{base_health} · {}", missing_problem.message)
                };
                let health = with_problem_context_label(&message_health, &missing_problem);
                (empty_book_line(fallback_venue, health), evidence)
            }
        }
        Err(error) => {
            let label = with_problem_context_label(
                &format!("错误 · {}", error.problem.message),
                &error.problem,
            );
            let evidence =
                error_section_evidence(orderbook_section(role, fallback_venue), &error.problem);
            (empty_book_line(fallback_venue, label), evidence)
        }
    }
}

pub(in crate::panels::modules::opportunities) fn market_section_evidence(
    section: String,
    health: &shared_types::MarketDataHealth,
    request_id: Option<&str>,
) -> DetailEvidence {
    DetailEvidence {
        section,
        source: market_source_label(health.source).to_owned(),
        freshness: EvidenceAge::new(health.freshness_ms).observed_at(health.observed_at_ms),
        status: format!("读取时{}", market_quality_label(health.quality)),
        retain_on_failure: health.quality != shared_types::MarketDataQuality::Unsupported
            && (health.problem.is_some() || health.last_error.is_some()
                || !matches!(health.quality, shared_types::MarketDataQuality::Fresh | shared_types::MarketDataQuality::Unverified)),
        retained: None,
        request_id: evidence_request_id(health.problem.as_ref(), request_id),
        retry_after: retry_after_text(health.retry_after_ms.or_else(|| {
            health
                .problem
                .as_ref()
                .and_then(|problem| problem.retry_after_ms)
        })),
        problem: health.problem.clone(),
    }
}

pub(in crate::panels::modules::opportunities) fn history_section_evidence<T>(
    section: String,
    response: &HistoryResponse<T>,
    request_id: Option<&str>,
) -> DetailEvidence {
    let problem = first_history_problem(response);
    DetailEvidence {
        section,
        source: history_source(&response.source).to_owned(),
        freshness: EvidenceAge::new(response.freshness_ms).observed_at(response.observed_at_ms),
        status: if problem.is_some() { "部分历史未能读取" } else { "历史读取成功" }.into(),
        retain_on_failure: problem.is_some() && response.source != "disabled",
        retained: None,
        request_id: evidence_request_id(problem.as_ref(), request_id),
        retry_after: retry_after_text(
            response
                .retry_after_ms
                .or_else(|| problem.as_ref().and_then(|problem| problem.retry_after_ms)),
        ),
        problem,
    }
}

pub(in crate::panels::modules::opportunities) fn error_section_evidence(
    section: String,
    problem: &ApiProblem,
) -> DetailEvidence {
    DetailEvidence {
        section,
        source: problem.source.clone().unwrap_or_else(|| "来源未知".into()),
        freshness: EvidenceAge::new(None),
        status: "读取失败".into(),
        retain_on_failure: true,
        retained: None,
        request_id: evidence_request_id(Some(problem), None),
        retry_after: retry_after_text(problem.retry_after_ms),
        problem: Some(problem.clone()),
    }
}

pub(in crate::panels::modules::opportunities) fn request_error_evidence(
    pair: &str,
    long_venue: &str,
    short_venue: &str,
    problem: &ApiProblem,
) -> Vec<DetailEvidence> {
    [
        orderbook_section("买入一边", long_venue),
        orderbook_section("卖出一边", short_venue),
        history_section(pair),
        index_section("买入一边", long_venue),
        index_section("卖出一边", short_venue),
        leg_market_section("买入一边", long_venue),
        leg_market_section("卖出一边", short_venue),
    ]
    .into_iter()
    .map(|section| error_section_evidence(section, problem))
    .collect()
}

pub(in crate::panels::modules::opportunities) fn evidence_request_id(
    problem: Option<&ApiProblem>,
    fallback: Option<&str>,
) -> String {
    problem
        .and_then(|problem| problem.request_id.as_deref())
        .or(fallback)
        .unwrap_or("-")
        .to_owned()
}

pub(in crate::panels::modules::opportunities) fn retry_after_text(
    retry_after_ms: Option<u64>,
) -> String {
    retry_after_ms.map_or_else(
        || "-".into(),
        |retry_after_ms| format!("{retry_after_ms}ms"),
    )
}

pub(in crate::panels::modules::opportunities) fn leg_market_evidence(
    role: &str,
    venue: &str,
    evidence: Option<&shared_types::OpportunityLegMarketEvidence>,
    request_id: Option<&str>,
) -> DetailEvidence {
    evidence.map_or_else(
        || DetailEvidence {
            section: leg_market_section(role, venue),
            source: "数据待确认".into(),
            freshness: EvidenceAge::new(None),
            status: "未取得行情数据依据".into(),
            retain_on_failure: false,
            retained: None,
            request_id: request_id.unwrap_or("-").to_owned(),
            retry_after: "-".into(),
            problem: None,
        },
        |evidence| {
            market_section_evidence(
                leg_market_section(role, &evidence.venue),
                &evidence.health,
                request_id,
            )
        },
    )
}

pub(in crate::panels::modules::opportunities) fn leg_market_section(
    role: &str,
    venue: &str,
) -> String {
    format!("行情 {role} · {venue}")
}

pub(in crate::panels::modules::opportunities) fn orderbook_section(
    role: &str,
    venue: &str,
) -> String {
    format!("订单簿 {role} · {venue}")
}

pub(in crate::panels::modules::opportunities) fn index_section(role: &str, venue: &str) -> String {
    format!("指数 {role} · {venue}")
}

pub(in crate::panels::modules::opportunities) fn history_section(symbol: &str) -> String {
    format!("历史 · {symbol}")
}
