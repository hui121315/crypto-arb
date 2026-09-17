use leptos::prelude::*;
use shared_types::{VenueQuality, VenueQualitySampleStatus};

use super::VenueQualityChartMeta;

pub(in crate::panels::modules::review) fn venue_radar(
    rows: Memo<Vec<VenueQuality>>,
    meta: Memo<VenueQualityChartMeta>,
) -> impl IntoView {
    let radar = Memo::new(move |_| radar_model(&rows.get()));
    view! {
        <div class="venue-radar">
            {move || {
                let meta = meta.get();
                let meta_label = meta.label();
                let Some(radar) = radar.get() else {
                    return view! {
                        <div class="empty-cell">{format!("缺少雷达样本 · {meta_label}")}</div>
                    }.into_any();
                };
                let aria = format!("交易所质量雷达，样本 {}，{}", radar.sample_count, meta_label);
                let meta_title = meta_label.clone();
                view! {
                    <svg viewBox="0 0 220 180" role="img" aria-label=aria>
                        <polygon class="radar-grid" points="110,18 190,62 170,150 50,150 30,62"/>
                        <polygon class="radar-grid inner" points="110,54 154,78 143,126 77,126 66,78"/>
                        <polygon class="radar-fill" points=radar.points.clone()/>
                        <polyline class="radar-line" points=radar.points/>
                    </svg>
                    <div class="radar-legend">
                        <span>"延迟"</span>
                        <span>"抖动"</span>
                        <span>"成交"</span>
                        <span>"滑点"</span>
                        <span>"可用"</span>
                        <span>{format!("样本 {}", radar.sample_count)}</span>
                        <span class="radar-meta" title=meta_title>{meta_label}</span>
                    </div>
                }.into_any()
            }}
        </div>
    }
}

#[derive(Clone, PartialEq)]
struct RadarModel {
    points: String,
    sample_count: u32,
}

fn radar_model(rows: &[VenueQuality]) -> Option<RadarModel> {
    let rows = valid_rows(rows);
    if rows.is_empty() {
        return None;
    }
    let latency = 1.0 - avg(&rows, |row| row.avg_rest_latency_ms as f64) / 220.0;
    let jitter = 1.0 - avg(&rows, |row| row.ws_jitter_p99_ms as f64) / 360.0;
    let fill = avg(&rows, |row| row.fill_rate_pct) / 100.0;
    let slippage = 1.0 - avg(&rows, |row| row.avg_slippage_bps) / 6.0;
    let uptime = avg(&rows, |row| row.uptime_window_pct) / 100.0;
    let sample_count = rows
        .iter()
        .map(|row| row.uptime_window_samples)
        .sum::<u32>();
    Some(RadarModel {
        points: radar_points(latency, jitter, fill, slippage, uptime),
        sample_count,
    })
}

fn valid_rows(rows: &[VenueQuality]) -> Vec<&VenueQuality> {
    rows.iter().filter(|row| valid_row(row)).collect()
}

fn valid_row(row: &VenueQuality) -> bool {
    row.sample_status != VenueQualitySampleStatus::NoSample
        && row.uptime_window_samples > 0
        && row.fill_window_samples > 0
        && row.fill_rate_pct.is_finite()
        && row.avg_slippage_bps.is_finite()
        && row.uptime_window_pct.is_finite()
}

fn radar_points(latency: f64, jitter: f64, fill: f64, slippage: f64, uptime: f64) -> String {
    [
        point(110.0, 90.0, 0.0, latency.clamp(0.05, 1.0)),
        point(110.0, 90.0, 72.0, jitter.clamp(0.05, 1.0)),
        point(110.0, 90.0, 144.0, fill.clamp(0.05, 1.0)),
        point(110.0, 90.0, 216.0, slippage.clamp(0.05, 1.0)),
        point(110.0, 90.0, 288.0, uptime.clamp(0.05, 1.0)),
    ]
    .into_iter()
    .map(|(x, y)| format!("{x:.1},{y:.1}"))
    .collect::<Vec<_>>()
    .join(" ")
}

fn avg(rows: &[&VenueQuality], value: impl Fn(&VenueQuality) -> f64) -> f64 {
    rows.iter().map(|row| value(row)).sum::<f64>() / rows.len() as f64
}

fn point(cx: f64, cy: f64, deg: f64, score: f64) -> (f64, f64) {
    let rad = (deg - 90.0).to_radians();
    let radius = 72.0 * score;
    (cx + radius * rad.cos(), cy + radius * rad.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radar_ignores_bad_or_unsampled_rows() {
        let rows = vec![
            row("bad", f64::NAN, 1),
            row("empty", 90.0, 0),
            row("ok", 90.0, 7),
        ];

        let model = radar_model(&rows).unwrap_or(RadarModel {
            points: String::new(),
            sample_count: 0,
        });

        assert_eq!(model.sample_count, 7);
        assert!(!model.points.contains("NaN"));
    }

    #[test]
    fn radar_returns_none_without_valid_samples() {
        let rows = vec![row("empty", 90.0, 0)];

        assert!(radar_model(&rows).is_none());
    }

    fn row(venue: &str, fill_rate_pct: f64, samples: u32) -> VenueQuality {
        VenueQuality {
            venue: venue.into(),
            source: "test".into(),
            sample_status: if samples == 0 {
                VenueQualitySampleStatus::NoSample
            } else {
                VenueQualitySampleStatus::Ready
            },
            avg_rest_latency_ms: 80,
            rest_latency_samples: samples,
            ws_jitter_p99_ms: 120,
            ws_jitter_samples: samples,
            fill_rate_pct,
            fill_window_samples: samples,
            avg_slippage_bps: 2.0,
            slippage_samples: samples,
            uptime_window_pct: 99.0,
            uptime_window_samples: samples,
            sample_window: shared_types::VenueQualitySampleWindow::default(),
            operation_health: Vec::new(),
            retry_after_ms: None,
            last_problem: None,
        }
    }
}
