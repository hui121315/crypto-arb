use leptos::prelude::*;
use shared_types::{MissReason, MissedOpportunity};

use super::format::{reason_label, signed_money};

#[derive(Clone, Copy, PartialEq)]
struct ReasonSlice {
    reason: MissReason,
    count: usize,
    pnl_usd: f64,
}

const REASONS: &[MissReason] = &[
    MissReason::RiskBlocked,
    MissReason::DepthInsufficient,
    MissReason::LatencyExceeded,
    MissReason::PriceMoved,
    MissReason::ManualSkip,
    MissReason::SignalDecayed,
];

pub(in crate::panels::modules::review) fn attribution_chart(
    rows: Memo<Vec<MissedOpportunity>>,
) -> impl IntoView {
    let slices = Memo::new(move |_| reason_slices(&rows.get()));
    view! {
        <section class="review-attribution">
            <div class="review-viz-head">
                <span>"错失归因 · 机会快照"</span>
                <strong>{move || format!("{} 条观察", rows.get().len())}</strong>
            </div>
            <div class="attribution-bars">
                {move || {
                    let slices = slices.get();
                    if slices.is_empty() {
                        return view! { <div class="empty-cell">"暂无归因数据"</div> }.into_any();
                    }
                    let total = slices.iter().map(|slice| slice.count).sum::<usize>().max(1);
                    slices.into_iter().map(|slice| view! {
                        <div class="attribution-row">
                            <span>{reason_label(slice.reason)}</span>
                            <div><i style=format!("width: {:.1}%;", pct(slice.count, total))></i></div>
                            <strong>{format!("{} 条 · 预期 {}", slice.count, signed_money(slice.pnl_usd))}</strong>
                        </div>
                    }).collect_view().into_any()
                }}
            </div>
        </section>
    }
}

fn reason_slices(rows: &[MissedOpportunity]) -> Vec<ReasonSlice> {
    REASONS
        .iter()
        .copied()
        .filter_map(|reason| {
            let mut count = 0;
            let mut pnl_usd = 0.0;
            for row in rows.iter().filter(|row| row.reason == reason) {
                count += 1;
                pnl_usd += row.expected_pnl_usd;
            }
            (count > 0).then_some(ReasonSlice {
                reason,
                count,
                pnl_usd,
            })
        })
        .collect()
}

fn pct(count: usize, total: usize) -> f64 {
    count as f64 / total as f64 * 100.0
}
