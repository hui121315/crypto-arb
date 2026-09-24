use shared_types::ApiProblem;

pub(crate) fn normalized_query(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

pub(crate) fn symbol_search_query(value: &str) -> Option<String> {
    let query = normalized_query(value);
    if query.len() < 2 || query.len() > 24 || is_venue_query(&query) {
        return None;
    }
    query
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | ':'))
        .then_some(query)
}

pub(crate) fn is_venue_query(query: &str) -> bool {
    matches!(
        query,
        "BINANCE"
            | "OKX"
            | "BYBIT"
            | "GATE"
            | "KUCOIN"
            | "BITGET"
            | "HYPERLIQUID"
            | "KRAKEN"
            | "BACKPACK"
            | "GATE_CROSSEX"
    ) || query.starts_with("HYPERLIQUID:")
}

pub(crate) fn symbol_search_problem(
    mut problem: ApiProblem,
    query: &str,
    cursor: Option<&str>,
) -> ApiProblem {
    let query = normalized_query(query);
    let cursor = cursor
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    problem.message = symbol_search_problem_message(&problem.message, &query, cursor.as_deref());
    problem.details = Some(symbol_search_problem_details(
        &query,
        cursor.as_deref(),
        problem.details.take(),
    ));
    if problem.source.as_deref().is_none_or(str::is_empty) {
        problem.source = Some("symbol-search".to_owned());
    }
    problem
}

fn symbol_search_problem_message(message: &str, query: &str, cursor: Option<&str>) -> String {
    let mut parts = vec![message.to_owned(), format!("symbol {query}")];
    if let Some(cursor) = cursor {
        parts.push(format!("cursor {cursor}"));
    }
    parts.join(" · ")
}

fn symbol_search_problem_details(
    query: &str,
    cursor: Option<&str>,
    backend_details: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut details = serde_json::json!({
        "symbolSearch": {
            "query": query,
            "cursor": cursor,
        },
        "backendDetails": null,
    });
    if let Some(backend_details) = backend_details {
        details["backendDetails"] = backend_details;
    }
    details
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_query_skips_venue_terms() {
        assert_eq!(symbol_search_query("mu"), Some("MU".into()));
        assert_eq!(symbol_search_query("MU-USDT"), Some("MU-USDT".into()));
        assert_eq!(symbol_search_query("binance"), None);
        assert_eq!(symbol_search_query("hyperliquid:xyz"), None);
        assert_eq!(symbol_search_query("kraken"), None);
        assert_eq!(symbol_search_query("backpack"), None);
        assert_eq!(symbol_search_query("gate_crossex"), None);
    }

    #[test]
    fn symbol_search_problem_keeps_local_request_context_visible() {
        let problem = ApiProblem::new("RATE_LIMITED", "rate limited")
            .with_request_id(Some("req-symbol".to_owned()))
            .with_retry_after_ms(Some(1_500));

        let enriched = symbol_search_problem(problem, " mu-usdt ", Some(" cursor-2 "));

        assert_eq!(enriched.code, "RATE_LIMITED");
        assert_eq!(enriched.request_id.as_deref(), Some("req-symbol"));
        assert_eq!(enriched.retry_after_ms, Some(1_500));
        assert_eq!(enriched.source.as_deref(), Some("symbol-search"));
        assert!(enriched.message.contains("symbol MU-USDT"));
        assert!(enriched.message.contains("cursor cursor-2"));
        assert_eq!(
            enriched
                .details
                .as_ref()
                .and_then(|details| details.get("symbolSearch"))
                .and_then(|details| details.get("query"))
                .and_then(serde_json::Value::as_str),
            Some("MU-USDT")
        );
        assert_eq!(
            enriched
                .details
                .as_ref()
                .and_then(|details| details.get("symbolSearch"))
                .and_then(|details| details.get("cursor"))
                .and_then(serde_json::Value::as_str),
            Some("cursor-2")
        );
    }

    #[test]
    fn symbol_search_problem_nests_backend_details() {
        let mut problem = ApiProblem::new("UPSTREAM", "upstream failed");
        problem.details = Some(serde_json::json!({ "venue": "okx" }));

        let enriched = symbol_search_problem(problem, "BTC", None);

        assert_eq!(
            enriched
                .details
                .as_ref()
                .and_then(|details| details.get("backendDetails"))
                .and_then(|details| details.get("venue"))
                .and_then(serde_json::Value::as_str),
            Some("okx")
        );
        assert_eq!(
            enriched
                .details
                .as_ref()
                .and_then(|details| details.get("symbolSearch"))
                .and_then(|details| details.get("cursor")),
            Some(&serde_json::Value::Null)
        );
    }
}
