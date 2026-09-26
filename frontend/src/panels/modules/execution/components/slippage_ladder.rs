use leptos::prelude::*;

use crate::panels::modules::rate_format::{depth_bps_percent_label, signed_bps_percent};

use super::super::data::PreviewDepth;
use super::super::draft::ExecutionDraft;

#[derive(Clone, Copy)]
struct LadderTier {
    id: u8,
    limit_offset_bps: f64,
}

const TIERS: &[LadderTier] = &[
    LadderTier {
        id: 5,
        limit_offset_bps: 5.0,
    },
    LadderTier {
        id: 10,
        limit_offset_bps: 10.0,
    },
    LadderTier {
        id: 20,
        limit_offset_bps: 20.0,
    },
];

pub(in crate::panels::modules::execution) fn slippage_ladder(
    draft: ExecutionDraft,
    expired: Memo<bool>,
) -> impl IntoView {
    view! {
        <section class="execution-section slippage-section">
            <div class="execution-section-head">
                <div>
                    <span>{move || if expired.get() { "上次盘口 · 已过期" } else { "滑点阶梯" }}</span>
                    <strong>{move || offset_text(draft)}</strong>
                </div>
                <em>{move || draft.order_type.get()}</em>
            </div>
            <div class="slippage-ladder">
                {TIERS.iter().map(|tier| view! {
                    <button
                        class=move || tier_class(tier, draft)
                        on:click=move |_| apply_tier(*tier, draft)
                    >
                        <span>{depth_bps_percent_label(tier.limit_offset_bps)}</span>
                        <strong>{move || tier_depth_text(*tier, draft)}</strong>
                        <i style=move || tier_width_style(*tier, draft)></i>
                    </button>
                }).collect_view()}
            </div>
        </section>
    }
}

fn tier_class(tier: &LadderTier, draft: ExecutionDraft) -> &'static str {
    let selected = draft
        .limit_offset_bps
        .get()
        .parse::<f64>()
        .unwrap_or_default();
    if (selected - tier.limit_offset_bps).abs() < f64::EPSILON {
        "active"
    } else {
        ""
    }
}

fn offset_text(draft: ExecutionDraft) -> String {
    let bps = draft
        .limit_offset_bps
        .get()
        .parse::<f64>()
        .unwrap_or_default();
    format!("限价偏移 {}", signed_bps_percent(bps))
}

fn apply_tier(tier: LadderTier, draft: ExecutionDraft) {
    draft
        .limit_offset_bps
        .set(format!("{:.0}", tier.limit_offset_bps));
}

fn tier_depth_text(tier: LadderTier, draft: ExecutionDraft) -> String {
    let preview = draft.preview.get();
    tier_depth_text_for(tier, &preview.depth)
}

fn tier_depth_text_for(tier: LadderTier, depth: &PreviewDepth) -> String {
    tier_depth(tier, depth)
        .map(money)
        .or_else(|| depth.executable_reason.clone())
        .or_else(|| depth.long_reason.clone())
        .or_else(|| depth.short_reason.clone())
        .unwrap_or_else(|| "等待 fresh 盘口".into())
}

fn tier_width_style(tier: LadderTier, draft: ExecutionDraft) -> String {
    let preview = draft.preview.get();
    let target = preview
        .long_notional_usd
        .max(preview.short_notional_usd)
        .max(1.0);
    tier_width_style_for(tier, &preview.depth, target)
}

fn tier_width_style_for(tier: LadderTier, depth: &PreviewDepth, target: f64) -> String {
    let Some(depth) = tier_depth(tier, depth) else {
        return "width: 0%;".into();
    };
    let width = ((depth / target) * 100.0).clamp(8.0, 100.0);
    format!("width: {width:.0}%;")
}

fn tier_depth(tier: LadderTier, depth: &PreviewDepth) -> Option<f64> {
    match tier.id {
        5 => min_depth(depth.long_5bps, depth.short_5bps),
        10 => min_depth(depth.long_10bps, depth.short_10bps),
        _ => min_depth(depth.long_20bps, depth.short_20bps),
    }
}

fn min_depth(long: Option<f64>, short: Option<f64>) -> Option<f64> {
    Some(long?.min(short?))
}

fn money(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("${:.0}K", value / 1_000.0)
    } else {
        format!("${value:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::HedgeDepthStatus;

    #[test]
    fn tier_text_prefers_structured_depth_reason() {
        let depth = PreviewDepth {
            long_5bps: None,
            long_10bps: None,
            long_20bps: None,
            short_5bps: Some(1_000.0),
            short_10bps: Some(1_000.0),
            short_20bps: Some(1_000.0),
            executable_status: HedgeDepthStatus::Unknown,
            executable_amount_usd: None,
            executable_reason: Some("hyperliquid MU orderbook 触发限频退避，2000ms 后重试".into()),
            long_reason: None,
            short_reason: None,
            long_depth_health: None,
            short_depth_health: None,
        };

        let text = tier_depth_text_for(TIERS[0], &depth);

        assert!(text.contains("限频退避"));
    }

    #[test]
    fn tier_width_keeps_missing_depth_empty() {
        let depth = PreviewDepth {
            long_5bps: None,
            long_10bps: None,
            long_20bps: None,
            short_5bps: Some(1_000.0),
            short_10bps: Some(1_000.0),
            short_20bps: Some(1_000.0),
            executable_status: HedgeDepthStatus::Unknown,
            executable_amount_usd: None,
            executable_reason: Some("等待 fresh 盘口".into()),
            long_reason: None,
            short_reason: None,
            long_depth_health: None,
            short_depth_health: None,
        };

        assert_eq!(
            tier_width_style_for(TIERS[0], &depth, 1_000.0),
            "width: 0%;"
        );
    }
}
