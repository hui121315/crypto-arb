use super::labels::status_label;
use super::labels::{payload_evidence_label, quality_label, snapshot_health};
use leptos::prelude::*;
use shared_types::IndexCompositionRiskProfile;
use shared_types::{IndexComponent, IndexCompositionSnapshot, IndexCompositionStatus};

const INDEX_COMPONENT_PREVIEW_COUNT: usize = 8;

#[derive(Clone, PartialEq)]
pub(crate) struct IndexCompositionView {
    pub label: String,
    pub detail: String,
    pub overlap_pct: Option<u8>,
    pub status: Option<IndexCompositionStatus>,
}

impl Default for IndexCompositionView {
    fn default() -> Self {
        Self {
            label: "未提供".into(),
            detail: "后端未提供指数成分风险。".into(),
            overlap_pct: None,
            status: None,
        }
    }
}

impl IndexCompositionView {
    pub(crate) fn from_profile(profile: Option<&IndexCompositionRiskProfile>) -> Self {
        let Some(profile) = profile else {
            return Self::default();
        };
        let overlap_pct = (profile.overlap_score.clamp(0.0, 1.0) * 100.0).round() as u8;
        let mut detail = format!(
            "{} · 重合度 {}% · 多腿 {} / 空腿 {}",
            status_label(profile.status),
            overlap_pct,
            quality_label(profile.long_quality),
            quality_label(profile.short_quality)
        );
        if let Some(blocker) = profile.blocker.as_deref() {
            detail.push_str(" · ");
            detail.push_str(blocker);
        }
        Self {
            label: status_label(profile.status).into(),
            detail,
            overlap_pct: Some(overlap_pct),
            status: Some(profile.status),
        }
    }

    pub(crate) fn compact_text(&self) -> String {
        self.overlap_pct
            .map(|pct| format!("{} {}%", self.label, pct))
            .unwrap_or_else(|| self.label.clone())
    }
}

#[derive(Clone, Default, PartialEq)]
pub(crate) struct IndexCompositionDetailView {
    pub long: Option<IndexCompositionSnapshotView>,
    pub short: Option<IndexCompositionSnapshotView>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct IndexCompositionSnapshotView {
    pub title: String,
    pub quality: String,
    pub health: String,
    pub evidence: Option<String>,
    pub error: Option<String>,
    pub rows: Vec<IndexComponentView>,
    pub remaining_rows: Vec<IndexComponentView>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct IndexComponentView {
    pub symbol: String,
    pub name: String,
    pub weight: String,
    pub price: String,
}

impl IndexCompositionDetailView {
    pub(crate) fn from_views(
        long: Option<IndexCompositionSnapshotView>,
        short: Option<IndexCompositionSnapshotView>,
    ) -> Self {
        Self { long, short }
    }

    fn is_empty(&self) -> bool {
        self.long.is_none() && self.short.is_none()
    }
}

impl IndexCompositionSnapshotView {
    pub(crate) fn from_snapshot(role: &str, snapshot: IndexCompositionSnapshot) -> Self {
        let total = snapshot.components.len();
        let mut rows: Vec<IndexComponentView> = snapshot
            .components
            .iter()
            .cloned()
            .map(IndexComponentView::from_component)
            .collect();
        let remaining_rows = rows.split_off(rows.len().min(INDEX_COMPONENT_PREVIEW_COUNT));
        Self {
            title: format!("{role} {} {}", snapshot.venue, snapshot.index_id),
            quality: quality_label(snapshot.quality).into(),
            health: snapshot_health(&snapshot, total, rows.len()),
            evidence: payload_evidence_label(&snapshot),
            error: snapshot.error,
            rows,
            remaining_rows,
        }
    }

    pub(crate) fn unavailable(
        role: &str,
        venue: &str,
        symbol: &str,
        health: String,
        error: String,
    ) -> Self {
        Self {
            title: format!("{role} {venue} {symbol}"),
            quality: "错误".into(),
            health,
            evidence: None,
            error: Some(error),
            rows: Vec::new(),
            remaining_rows: Vec::new(),
        }
    }
}

impl IndexComponentView {
    fn from_component(component: IndexComponent) -> Self {
        Self {
            symbol: component.symbol,
            name: component.name,
            weight: format!("{:.1}%", component.weight.max(0.0) * 100.0),
            price: component
                .price
                .filter(|price| price.is_finite())
                .map(|price| format!("{price:.4}"))
                .unwrap_or_else(|| "隐藏".into()),
        }
    }
}

pub(in crate::panels::modules) fn index_composition_detail(
    detail: IndexCompositionDetailView,
) -> impl IntoView {
    let visible = !detail.is_empty();
    let long = detail.long;
    let short = detail.short;
    view! {
        <Show
            when=move || visible
            fallback=move || view! { <p class="index-composition-empty">"指数成分明细等待缓存或官方接口返回。"</p> }
        >
            <div class="index-composition-detail">
                <SnapshotPanel snapshot=long.clone()/>
                <SnapshotPanel snapshot=short.clone()/>
            </div>
        </Show>
    }
}

#[component]
fn SnapshotPanel(snapshot: Option<IndexCompositionSnapshotView>) -> impl IntoView {
    view! {
        {snapshot.map(|snapshot| {
            let rows_view = component_rows_view(snapshot.rows);
            let remaining_count = snapshot.remaining_rows.len();
            let remaining_view = (!snapshot.remaining_rows.is_empty()).then(|| view! {
                <details class="index-component-more">
                    <summary>{format!("其余 {remaining_count} 项")}</summary>
                    {component_rows_view(snapshot.remaining_rows)}
                </details>
            });
            view! {
                <div class="index-composition-panel">
                    <div>
                        <strong>{snapshot.title}</strong>
                        <span>{snapshot.quality}" · "{snapshot.health}</span>
                        {snapshot.evidence.map(|evidence| view! { <span class="index-composition-evidence">{evidence}</span> })}
                        {snapshot.error.map(|error| view! { <em>{error}</em> })}
                    </div>
                    {rows_view}
                    {remaining_view}
                </div>
            }
        })}
    }
}

fn component_rows_view(rows: Vec<IndexComponentView>) -> AnyView {
    if rows.is_empty() {
        return view! {
            <p class="index-composition-empty">"无可展示成分；请查看上方 source / freshness / problem。"</p>
        }
        .into_any();
    }
    view! {
        <div class="index-component-list">
            {rows.into_iter().map(|row| view! {
                <div>
                    <strong>{row.symbol}</strong>
                    <span>{row.name}</span>
                    <em>{row.weight}" · "{row.price}</em>
                </div>
            }).collect_view()}
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::IndexCompositionQuality;

    #[test]
    fn snapshot_preserves_all_components_for_expand() {
        let snapshot = IndexCompositionSnapshot {
            venue: "hyperliquid:xyz".into(),
            symbol: "MU".into(),
            index_id: "MU-INDEX".into(),
            components: (0..11)
                .map(|index| IndexComponent {
                    symbol: format!("C{index}"),
                    name: format!("Component {index}"),
                    weight: 1.0 / 11.0,
                    price: Some(index as f64 + 1.0),
                })
                .collect(),
            quality: IndexCompositionQuality::Verified,
            source: "official-rest".into(),
            received_at_ms: 1_000,
            freshness_ms: Some(10),
            error: None,
            retry_after_ms: None,
            source_url: Some("https://example.test/index".into()),
            payload_sha256: Some("0123456789abcdef".into()),
            schema_version: Some("index-v1".into()),
        };

        let view = IndexCompositionSnapshotView::from_snapshot("多腿", snapshot);

        assert_eq!(view.rows.len(), 8);
        assert_eq!(view.remaining_rows.len(), 3);
        assert!(view.health.contains("成分 11 项 · 首屏 8 项"));
        assert!(view
            .evidence
            .as_deref()
            .is_some_and(|value| value.contains("schema index-v1")));
    }
}
