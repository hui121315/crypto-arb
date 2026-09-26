use super::*;
use crate::panels::modules::cost_copy::{fee_evidence_label, one_cycle_verdict};

pub(super) fn notional_pair(preview: &ExecutionPreview) -> String {
    format!(
        "{} / {}",
        money(preview.long_notional_usd),
        money(preview.short_notional_usd)
    )
}

pub(super) fn cost_breakdown_text(preview: &ExecutionPreview) -> String {
    if !preview.is_ready() {
        return "待成本".into();
    }
    if preview.one_cycle_cost.is_none() {
        return "缺成本数据依据".into();
    }
    format!(
        "开 {} / 平 {} / 滑点 {}",
        money(preview.open_cost_usd),
        money(preview.close_cost_usd),
        money(preview.slippage_cost_usd)
    )
}

pub(super) fn one_cycle_cost_summary(preview: &ExecutionPreview) -> String {
    if !preview.is_ready() {
        return "待成本".into();
    }
    preview
        .one_cycle_cost
        .as_ref()
        .map_or_else(|| "缺成本数据依据".into(), one_cycle_cost_line)
}

pub(super) fn one_cycle_cost_line(cost: &PreviewOneCycleCost) -> String {
    let verdict = one_cycle_verdict(cost.covers_round_trip_cost);
    format!("单次 {} · {verdict}", pct_from_bps(cost.net_bps))
}

pub(super) fn one_cycle_cost_detail(preview: &ExecutionPreview) -> String {
    if !preview.is_ready() {
        return "等待成本数据依据".into();
    }
    let Some(cost) = preview.one_cycle_cost.as_ref() else {
        return "缺 one-cycle 成本数据依据".into();
    };
    let funding_evidence = funding_window_evidence_line(cost.funding_window_mismatch_evidence);
    let profitability = profitability_evidence_line(cost);
    let shortfall = if cost.covers_round_trip_cost {
        ""
    } else {
        " / 净利不足，阻断执行"
    };
    format!(
        "毛 {} / 成 {} / 开 {} / 平 {} / 开滑 {} / 平滑 {} / {funding_evidence} / {profitability}{shortfall}",
        pct_from_bps(cost.gross_edge_bps),
        pct_from_bps(cost.total_cost_bps),
        pct_from_bps(cost.open_fee_bps),
        pct_from_bps(cost.close_fee_bps),
        pct_from_bps(cost.open_slippage_bps),
        pct_from_bps(cost.close_slippage_bps),
    )
}

fn profitability_evidence_line(cost: &PreviewOneCycleCost) -> String {
    let status = match cost.profitability_status {
        shared_types::ProfitabilityEvidenceStatus::Verified => "完整",
        shared_types::ProfitabilityEvidenceStatus::Partial => "部分",
        shared_types::ProfitabilityEvidenceStatus::Stale => "过期",
        shared_types::ProfitabilityEvidenceStatus::Missing => "缺失",
    };
    let history = cost.funding_history_health.map_or_else(
        || "历史无样本".to_owned(),
        |health| {
            format!(
                "历史 {} / {} 样本",
                funding_history_health_label(health),
                cost.funding_history_sample_count
            )
        },
    );
    format!("盈利数据依据 {status} / {history}")
}

fn funding_history_health_label(health: shared_types::FundingDiffSampleHealth) -> &'static str {
    match health {
        shared_types::FundingDiffSampleHealth::Ok => "健康",
        shared_types::FundingDiffSampleHealth::Thin => "样本偏少",
        shared_types::FundingDiffSampleHealth::Empty => "空",
        shared_types::FundingDiffSampleHealth::Stale => "过期",
        shared_types::FundingDiffSampleHealth::Unknown => "未知",
    }
}

fn funding_window_evidence_line(evidence: Option<PreviewFundingWindowEvidence>) -> String {
    let Some(evidence) = evidence else {
        return "缺窗口数据依据".into();
    };
    format!(
        "口径 {} / 缓冲 {} / 多腿 {}ms / 空腿 {}ms",
        evidence.yield_basis.label(),
        pct_from_bps(evidence.buffer_bps),
        evidence.long_next_settlement_ms,
        evidence.short_next_settlement_ms
    )
}

pub(super) fn profit_evidence_summary(preview: &ExecutionPreview) -> String {
    let evidence = &preview.profit_evidence;
    if !evidence.has_evidence() {
        return if preview.is_ready() {
            "缺收益数据依据".into()
        } else {
            "待机会数据依据".into()
        };
    }
    let fee_text = fee_evidence_label(
        evidence.fee_evidence_ids.len(),
        evidence.fee_evidence_complete,
    );
    format!(
        "{fee_text} · 单次费后 {}",
        pct_from_bps(evidence.one_cycle_net_bps)
    )
}

pub(super) fn profit_evidence_detail(preview: &ExecutionPreview) -> String {
    let evidence = &preview.profit_evidence;
    if !evidence.has_evidence() {
        return "等待机会列表携带费后收益与双腿费率数据依据".into();
    }
    let mut parts = vec![format!(
        "列表单次费后净利 {}",
        pct_from_bps(evidence.one_cycle_net_bps)
    )];
    if evidence.fee_evidence_ids.is_empty() {
        parts.push("费率 evidence id 未进入列表行".into());
    } else {
        parts.push(format!(
            "fee evidence {}",
            evidence.fee_evidence_ids.join(", ")
        ));
    }
    parts.push(fee_evidence_label(
        evidence.fee_evidence_ids.len(),
        evidence.fee_evidence_complete,
    ));
    if !evidence.fee_evidence_complete {
        parts.push("数据依据未完整，仅观察或阻断执行".into());
    }
    if evidence.one_cycle_net_bps <= f64::EPSILON {
        parts.push("单次费后净利下限非正，阻断执行".into());
    }
    parts.join(" / ")
}

pub(super) fn depth_summary(preview: &ExecutionPreview) -> String {
    let base = match preview.depth.executable_status {
        HedgeDepthStatus::Available => preview
            .depth
            .executable_amount_usd
            .map(money)
            .unwrap_or_else(|| "可执行".into()),
        HedgeDepthStatus::Insufficient => preview
            .depth
            .executable_amount_usd
            .map(|amount| format!("不足 {}", money(amount)))
            .unwrap_or_else(|| "深度不足".into()),
        HedgeDepthStatus::Unknown => preview
            .depth
            .executable_reason
            .clone()
            .or_else(|| preview.depth.long_reason.clone())
            .or_else(|| preview.depth.short_reason.clone())
            .unwrap_or_else(|| "等待 fresh 盘口".into()),
    };
    match depth_health_badge(preview) {
        Some(badge) => format!("{base} · {badge}"),
        _ => base,
    }
}

pub(super) fn depth_detail(preview: &ExecutionPreview) -> String {
    let mut parts = [
        preview.depth.executable_reason.clone(),
        preview.depth.long_reason.clone(),
        preview.depth.short_reason.clone(),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>();
    parts.extend(depth_health_lines(preview));
    if parts.is_empty() {
        "等待后端 HedgeTicket 返回深度数据依据".into()
    } else {
        parts.join(" / ")
    }
}

pub(super) fn depth_health_badge(preview: &ExecutionPreview) -> Option<String> {
    depth_health_badge_from_pair(
        preview.depth.long_depth_health.as_ref(),
        preview.depth.short_depth_health.as_ref(),
    )
}

pub(super) fn depth_health_badge_from_pair(
    long: Option<&MarketDataHealth>,
    short: Option<&MarketDataHealth>,
) -> Option<String> {
    let health = [long, short];
    let total = health.iter().filter(|item| item.is_some()).count();
    if total == 0 {
        return None;
    }
    let fresh = health
        .iter()
        .flatten()
        .filter(|item| item.quality == MarketDataQuality::Fresh)
        .count();
    Some(format!("盘口 {fresh}/{total}"))
}
