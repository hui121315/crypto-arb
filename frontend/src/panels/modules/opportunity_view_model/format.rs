//! 机会列表视图模型共享的派生/格式化助手：执行 blocker 收敛、行情证据新鲜度、
//! 来源/倒计时/金额格式化。视图模型结构与构造见 `model.rs`，展示文案见 `labels.rs`。

use crate::panels::modules::timestamp::now_ms;
use shared_types::{MarketDataQuality, MarketDataSourceKind, OpportunityLegMarketEvidence};

pub(in crate::panels::modules::opportunity_view_model) fn execution_blockers(
    mut blockers: Vec<String>,
    p0_scope: bool,
    market_evidence_ready: bool,
    cost_verified: bool,
) -> Vec<String> {
    if !p0_scope && !blockers.iter().any(|value| value.contains("非 P0 策略")) {
        blockers.insert(0, "非 P0 策略未开放，仅观察。".into());
    }
    if !market_evidence_ready && !blockers.iter().any(|value| value.contains("行情证据") || value.contains("行情数据依据")) {
        blockers.push("行情数据依据未验证，仅观察：等待双腿交易所实时行情。".into());
    }
    if !cost_verified && !blockers.iter().any(|value| value.contains("成本未验证")) {
        blockers.push("成本未验证，仅观察：等待后端 execution_cost 数据依据。".into());
    }
    blockers
}

pub(in crate::panels::modules::opportunity_view_model) fn has_fresh_market_evidence(
    long: Option<&OpportunityLegMarketEvidence>,
    short: Option<&OpportunityLegMarketEvidence>,
) -> bool {
    leg_has_fresh_market_evidence(long) && leg_has_fresh_market_evidence(short)
}

fn leg_has_fresh_market_evidence(evidence: Option<&OpportunityLegMarketEvidence>) -> bool {
    evidence.is_some_and(|evidence| {
        evidence.health.quality == MarketDataQuality::Fresh
            && evidence.health.source == MarketDataSourceKind::WsPush
    })
}

pub(in crate::panels::modules::opportunity_view_model) fn source_label(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        "未知".into()
    } else {
        trimmed.into()
    }
}

pub(in crate::panels::modules::opportunity_view_model) fn row_freshness_line(
    countdown: &str,
    source: &str,
    updated_at_ms: i64,
) -> String {
    format!(
        "{countdown} · 来源 {} · 更新 {}",
        source_label(source),
        updated_age_label(updated_at_ms)
    )
}

fn updated_age_label(updated_at_ms: i64) -> String {
    if updated_at_ms <= 0 {
        return "未知".into();
    }
    let age_ms = now_ms().saturating_sub(updated_at_ms).max(0);
    if age_ms < 1_000 {
        "刚刚".into()
    } else if age_ms < 60_000 {
        format!("{}s前", age_ms / 1_000)
    } else if age_ms < 3_600_000 {
        format!("{:.1}m前", age_ms as f64 / 60_000.0)
    } else {
        format!("{:.1}h前", age_ms as f64 / 3_600_000.0)
    }
}

pub(in crate::panels::modules::opportunity_view_model) fn countdown(
    seconds: Option<i64>,
    ms: i64,
) -> String {
    let Some(secs) = seconds.or_else(|| (ms > 0).then_some(ms / 1_000)) else {
        return "结算时间数据待确认".into();
    };
    let secs = secs.max(0);
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, secs % 3600 / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

pub(in crate::panels::modules::opportunity_view_model) fn pct(value: f64) -> String {
    format!("{value:.3}%")
}

pub(in crate::panels::modules::opportunity_view_model) fn money(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("${:.0}K", value / 1_000.0)
    } else {
        format!("${value:.0}")
    }
}
