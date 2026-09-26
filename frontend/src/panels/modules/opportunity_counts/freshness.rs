//! 机会快照新鲜度/覆盖/状态文案：把 `OpportunityCountMeta` 的扫描覆盖、降级 venue、
//! envelope 状态与 retry/error 收敛成单行诊断标签，过 60s 的 Fresh 视为 Stale。
//! 计数选择器见 `count_meta.rs`，纯文案助手见 `format.rs`。

use shared_types::{OpportunityEnvelopeScope, OpportunityEnvelopeStatus, OpportunityScanOutcome};

use super::count_meta::OpportunityCountMeta;
use super::format::{duration_label, problem_text};

impl OpportunityCountMeta {
    pub(crate) fn aged_at(&self, clock: (i64, i64)) -> Self {
        let mut aged = self.clone();
        let base_age = self.freshness_ms.or_else(|| {
            self.cached_at.map(|cached| {
                self.observed_at_ms
                    .saturating_sub(cached.timestamp_millis())
                    .max(0)
            })
        });
        // Browser elapsed time is separate from the server clock. Keep aging through sleep
        // and do not make an old quote younger when the system clock moves backwards.
        let elapsed = self.received_clock.map_or(0, |received| {
            clock
                .0
                .saturating_sub(received.0)
                .max(clock.1.saturating_sub(received.1))
                .max(0)
        });
        aged.freshness_ms = base_age.map(|age| age.max(0).saturating_add(elapsed));
        aged.received_clock = Some(clock);
        aged
    }

    pub(crate) fn preview_age_expired(&self) -> bool {
        self.freshness_ms
            .is_some_and(|age| age > shared_types::HEDGE_PREVIEW_MARKET_MAX_AGE_MS)
    }

    pub(crate) fn freshness_label(&self) -> String {
        let Some((age, age_secs)) = self.snapshot_age_label() else {
            return "等待机会数据".into();
        };
        let source = if self.source.is_empty() {
            "snapshot"
        } else {
            self.source.as_str()
        };
        format!(
            "{} · {age} · 来源 {source} · {} · 查询 {} · {}{}",
            self.status_label(age_secs),
            self.scope_label(),
            self.query_key,
            self.coverage_label(),
            self.retry_label()
        )
    }

    fn snapshot_age_label(&self) -> Option<(String, u64)> {
        if let Some(freshness_ms) = self.freshness_ms {
            let age_secs = (freshness_ms.max(0) / 1_000) as u64;
            return Some((
                format!("距更新 {}", duration_label(freshness_ms)),
                age_secs,
            ));
        }
        let cached_at = self.cached_at?;
        let age_ms = (js_sys::Date::now() as i64 - cached_at.timestamp_millis()).max(0);
        Some((format!("距更新 {}", duration_label(age_ms)), (age_ms / 1_000) as u64))
    }

    fn coverage_label(&self) -> String {
        let coverage = &self.scan.coverage;
        let history = match self.scan.history_append_ok {
            Some(true) => "记录已保存",
            Some(false) => "记录保存失败",
            None => "记录保存状态未知",
        };
        format!(
            "扫描耗时 {}ms · {} · 筛选前 {} / 筛选后 {} · 资金费 {} 条 / {} 家交易所 · 永续行情 {} 条 · 现货行情 {} 条 · 行情问题 {} 项{} · {history}",
            self.scan.scan_ms,
            scan_outcome_label(self.scan.scan_outcome),
            self.scan.candidate_count,
            self.scan.emitted_count,
            coverage.funding_rows,
            coverage.funding_venues,
            coverage.perp_tickers,
            coverage.spot_ticks,
            self.scan.market_data_problem_count,
            degraded_venues_label(&self.scan.degraded_venues)
        )
    }

    fn status_label(&self, age_secs: u64) -> &'static str {
        match self.status {
            OpportunityEnvelopeStatus::Fresh if age_secs > 60 => "数据已过期",
            OpportunityEnvelopeStatus::Fresh => "数据已更新",
            OpportunityEnvelopeStatus::Warming => "正在读取首批数据",
            OpportunityEnvelopeStatus::Stale => "数据已过期",
            OpportunityEnvelopeStatus::Degraded => "部分数据异常",
            OpportunityEnvelopeStatus::Error => "读取失败",
        }
    }

    fn scope_label(&self) -> &'static str {
        match self.scope {
            OpportunityEnvelopeScope::MainP0 => "主要候选",
            OpportunityEnvelopeScope::RegistrySnapshot => "已收录候选",
            OpportunityEnvelopeScope::Custom => "自定义筛选",
        }
    }

    fn retry_label(&self) -> String {
        if let Some(problem) = self
            .error
            .as_ref()
            .or_else(|| self.partial_failures.first())
        {
            return format!(
                " · {} · {}",
                problem.code,
                problem_text(problem, self.retry_after_ms)
            );
        }
        self.retry_after_ms
            .map(|ms| format!(" · {ms}ms 后重试"))
            .unwrap_or_default()
    }
}

pub(crate) fn snapshot_clock() -> (i64, i64) {
    #[cfg(target_arch = "wasm32")]
    {
        let wall = js_sys::Date::now() as i64;
        let monotonic = web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now() as i64)
            .unwrap_or(wall);
        (wall, monotonic)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = chrono::Utc::now().timestamp_millis();
        (now, now)
    }
}

fn scan_outcome_label(outcome: OpportunityScanOutcome) -> &'static str {
    match outcome {
        OpportunityScanOutcome::Found => "发现候选机会",
        OpportunityScanOutcome::TrueEmpty => "本次扫描未发现机会",
        OpportunityScanOutcome::FilteredEmpty => "没有符合筛选条件的机会",
        OpportunityScanOutcome::Warming => "等待首批行情",
        OpportunityScanOutcome::PartialUpstream => "部分行情未取得",
    }
}

fn degraded_venues_label(venues: &[String]) -> String {
    if venues.is_empty() {
        String::new()
    } else {
        format!(" ({})", venues.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::ApiProblem;

    #[test]
    fn coverage_label_includes_scan_coverage_and_market_issues() {
        let mut meta = OpportunityCountMeta {
            total_count: 4,
            filtered_count: 3,
            executable_count: 2,
            scan: shared_types::OpportunityScanMeta {
                candidate_count: 5,
                emitted_count: 4,
                scan_outcome: shared_types::OpportunityScanOutcome::Found,
                scan_ms: 12,
                history_append_ok: Some(false),
                market_data_problem_count: 2,
                degraded_venues: vec!["bybit".to_owned(), "all".to_owned()],
                coverage: shared_types::OpportunityDataCoverage {
                    funding_rows: 9,
                    funding_venues: 3,
                    perp_tickers: 7,
                    spot_ticks: 6,
                    ..Default::default()
                },
                ..Default::default()
            },
            source: "snapshot".to_owned(),
            cached_at: None,
            ..Default::default()
        };

        let label = meta.coverage_label();

        assert!(label.contains("扫描耗时 12ms"));
        assert!(label.contains("发现候选机会"));
        assert!(label.contains("筛选前 5 / 筛选后 4"));
        assert!(label.contains("资金费 9 条 / 3 家交易所"));
        assert!(label.contains("行情问题 2 项"));
        assert!(label.contains("记录保存失败"));

        meta.scan.degraded_venues.clear();
        assert!(!meta.coverage_label().contains("bybit"));
    }

    #[test]
    fn freshness_label_surfaces_envelope_error_message_and_source() {
        let meta = OpportunityCountMeta {
            status: OpportunityEnvelopeStatus::Error,
            source: "snapshot".into(),
            cached_at: Some(chrono::Utc::now()),
            error: Some(
                ApiProblem::new("SNAPSHOT_FAILED", "scan failed").with_source("arbitrage-snapshot"),
            ),
            ..Default::default()
        };

        let label = meta.retry_label();

        assert!(label.contains("SNAPSHOT_FAILED"));
        assert!(label.contains("scan failed"));
        assert!(label.contains("arbitrage-snapshot"));
    }

    #[test]
    fn freshness_label_prefers_envelope_freshness_ms() {
        let meta = OpportunityCountMeta {
            source: "ws".into(),
            cached_at: Some(chrono::Utc::now()),
            observed_at_ms: 10_000,
            freshness_ms: Some(2_500),
            ..Default::default()
        };

        let label = meta.freshness_label();

        assert!(label.contains("距更新 2.5s"));
        assert!(label.contains("ws"));
    }
}
