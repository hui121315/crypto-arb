use shared_types::{
    ApiProblem, MarketDataHealth, MarketDataQuality, MarketDataSourceKind,
    OpportunityLegMarketEvidence,
};

pub(crate) fn leg_evidence_label(
    evidence: Option<&OpportunityLegMarketEvidence>,
) -> Option<String> {
    evidence.map(|item| format!("证据 {}", market_health_label(&item.health)))
}

pub(crate) fn compact_leg_evidence_label(
    evidence: Option<&OpportunityLegMarketEvidence>,
) -> Option<(String, &'static str)> {
    evidence.map(|item| {
        let health = &item.health;
        let incomplete_coverage = health
            .coverage
            .as_ref()
            .filter(|coverage| coverage.requested > 0 && coverage.received < coverage.requested);
        let coverage_incomplete = incomplete_coverage.is_some();
        let needs_attention = coverage_incomplete
            || health.problem.is_some()
            || health.last_error.is_some()
            || health.retry_after_ms.is_some();
        let state = if needs_attention {
            "is-problem"
        } else {
            match health.quality {
                MarketDataQuality::Fresh => "is-fresh",
                MarketDataQuality::StaleAllowed => "is-cached",
                _ => "is-problem",
            }
        };
        let mut parts = Vec::with_capacity(5);
        if health.quality != MarketDataQuality::Fresh {
            parts.push(market_quality_label(health.quality).to_owned());
        }
        parts.push(market_source_label(health.source).to_owned());
        if let Some(freshness_ms) = health.freshness_ms {
            parts.push(freshness_label(freshness_ms));
        }
        if let Some(coverage) = incomplete_coverage {
            parts.push(format!("覆盖 {}/{}", coverage.received, coverage.requested));
        }
        if needs_attention && !coverage_incomplete {
            parts.push("需查看".to_owned());
        }
        (parts.join(" · "), state)
    })
}

pub(crate) fn market_health_label(health: &MarketDataHealth) -> String {
    let base = format!(
        "{} · {}",
        market_quality_label(health.quality),
        market_source_label(health.source)
    );
    let with_freshness = health.freshness_ms.map_or(base.clone(), |freshness| {
        format!("{base} · {}", freshness_label(freshness))
    });
    let with_coverage = coverage_label(&with_freshness, health);
    let with_problem = problem_label(&with_coverage, health);
    retry_label(&with_problem, health.retry_after_ms)
}

fn freshness_label(freshness_ms: i64) -> String {
    let freshness_ms = freshness_ms.max(0);
    if freshness_ms < 1_000 {
        return format!("{freshness_ms}ms");
    }
    if freshness_ms < 60_000 {
        return format!("{:.1}s", freshness_ms as f64 / 1_000.0);
    }
    if freshness_ms < 3_600_000 {
        return format!("{}m", freshness_ms / 60_000);
    }
    format!("{}h", freshness_ms / 3_600_000)
}

fn coverage_label(base: &str, health: &MarketDataHealth) -> String {
    health.coverage.as_ref().map_or_else(
        || base.to_owned(),
        |coverage| {
            format!(
                "{base} · 覆盖 {}/{} ({:.0}%)",
                coverage.received,
                coverage.requested,
                coverage.coverage_pct * 100.0
            )
        },
    )
}

fn problem_label(base: &str, health: &MarketDataHealth) -> String {
    health.problem.as_ref().map_or_else(
        || {
            health
                .last_error
                .as_ref()
                .map_or_else(|| base.to_owned(), |error| format!("{base} · {error}"))
        },
        |problem| {
            let message = format!("{base} · {}", problem.message);
            problem_context_label(problem)
                .map_or(message.clone(), |context| format!("{message} · {context}"))
        },
    )
}

fn problem_context_label(problem: &ApiProblem) -> Option<String> {
    let mut context = structured_problem_context(problem);
    if let Some(request_id) = problem.request_id.as_deref() {
        context.push(format!("请求 {request_id}"));
    }
    (!context.is_empty()).then(|| context.join(" · "))
}

pub(crate) fn structured_problem_context_label(problem: &ApiProblem) -> Option<String> {
    let context = structured_problem_context(problem);
    (!context.is_empty()).then(|| context.join(" · "))
}

fn structured_problem_context(problem: &ApiProblem) -> Vec<String> {
    let mut context = Vec::with_capacity(4);
    if let Some(details) = problem.details.as_ref() {
        push_problem_detail(&mut context, details, "operation");
        push_problem_detail(&mut context, details, "symbol");
        push_problem_detail(&mut context, details, "path");
        if let Some(latency_ms) = details
            .get("latencyMs")
            .or_else(|| details.get("lastLatencyMs"))
            .and_then(serde_json::Value::as_u64)
        {
            context.push(format!("HTTP耗时 {latency_ms}ms"));
        }
    }
    context
}

fn push_problem_detail(context: &mut Vec<String>, details: &serde_json::Value, key: &str) {
    if let Some(value) = details.get(key).and_then(serde_json::Value::as_str) {
        context.push(value.to_owned());
    }
}

pub(crate) fn retry_label(base: &str, retry_after_ms: Option<u64>) -> String {
    retry_after_ms.map_or_else(
        || base.to_owned(),
        |retry_after_ms| format!("{base} · {retry_after_ms}ms 后重试"),
    )
}

pub(crate) fn market_quality_label(quality: MarketDataQuality) -> &'static str {
    match quality {
        MarketDataQuality::Fresh => "新鲜",
        MarketDataQuality::StaleAllowed => "短时缓存",
        MarketDataQuality::StaleBlocked => "过期",
        MarketDataQuality::Missing => "缺数据",
        MarketDataQuality::RateLimited => "限频",
        MarketDataQuality::CircuitOpen => "熔断",
        MarketDataQuality::Unsupported => "不支持",
        MarketDataQuality::Unverified => "未验证",
    }
}

pub(crate) fn market_source_label(source: MarketDataSourceKind) -> &'static str {
    match source {
        MarketDataSourceKind::WsPush => "WS",
        MarketDataSourceKind::RestColdStart => "REST 冷启动",
        MarketDataSourceKind::RestBaseline => "REST 基线",
        MarketDataSourceKind::RestFallback => "REST 兜底",
        MarketDataSourceKind::LocalCache => "本地缓存",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExchangeProblem, MarketDataCoverage};

    #[test]
    fn formats_fresh_leg_evidence_without_inference() {
        let evidence = OpportunityLegMarketEvidence {
            venue: "binance".into(),
            symbol: "MU".into(),
            price: Some(100.0),
            health: health(MarketDataQuality::Fresh),
        };

        assert_eq!(
            leg_evidence_label(Some(&evidence)).as_deref(),
            Some("证据 新鲜 · 本地缓存 · 10ms · 覆盖 1/1 (100%)")
        );
        assert_eq!(
            compact_leg_evidence_label(Some(&evidence)),
            Some(("本地缓存 · 10ms".to_owned(), "is-fresh"))
        );
    }

    #[test]
    fn formats_retry_and_problem_from_health() {
        let mut health = health(MarketDataQuality::RateLimited);
        health.retry_after_ms = Some(2_000);
        health.problem = Some(ApiProblem::new("RATE_LIMITED", "rate limited"));

        assert_eq!(
            market_health_label(&health),
            "限频 · 本地缓存 · 10ms · 覆盖 1/1 (100%) · rate limited · 2000ms 后重试"
        );
    }

    #[test]
    fn exposes_structured_exchange_problem_context_for_execution_evidence() {
        let mut health = health(MarketDataQuality::RateLimited);
        health.problem = Some(
            ExchangeProblem::new("binance", "rest_orderbooks", "rate limited")
                .with_path("/fapi/v1/depth")
                .with_symbol("BTCUSDT")
                .with_latency_ms(Some(35))
                .with_request_id(Some("req-ev-1".into()))
                .to_api_problem("UPSTREAM_HTTP"),
        );

        assert_eq!(
            market_health_label(&health),
            "限频 · 本地缓存 · 10ms · 覆盖 1/1 (100%) · rate limited · rest_orderbooks · BTCUSDT · /fapi/v1/depth · HTTP耗时 35ms · 请求 req-ev-1"
        );
    }

    #[test]
    fn exposes_shared_market_labels_for_diagnostics() {
        assert_eq!(market_quality_label(MarketDataQuality::RateLimited), "限频");
        assert_eq!(market_source_label(MarketDataSourceKind::WsPush), "WS");
    }

    #[test]
    fn freshness_uses_compact_units_without_losing_low_latency_precision() {
        assert_eq!(freshness_label(32), "32ms");
        assert_eq!(freshness_label(5_615), "5.6s");
        assert_eq!(freshness_label(184_131), "3m");
        assert_eq!(freshness_label(7_200_000), "2h");
        assert_eq!(freshness_label(-1), "0ms");
    }

    fn health(quality: MarketDataQuality) -> MarketDataHealth {
        MarketDataHealth {
            quality,
            source: MarketDataSourceKind::LocalCache,
            freshness_ms: Some(10),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: Some(MarketDataCoverage::new(1, 1)),
            problem: None,
        }
    }
}
