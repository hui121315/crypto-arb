use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{
    OnchainComparisonDirection, OnchainComparisonQuality, OnchainComparisonSnapshot,
};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};

use super::super::format::percent_label;

const CHART_WIDTH: f64 = 640.0;
const CHART_HEIGHT: f64 = 126.0;
const CHART_PAD_X: f64 = 5.0;
const CHART_PAD_Y: f64 = 10.0;
const MAX_SAMPLES: usize = 96;
const MIN_SAMPLE_INTERVAL_MS: i64 = 500;

#[derive(Clone, Copy, PartialEq)]
struct SpreadSample {
    observed_at_ms: i64,
    buy_onchain_sell_cex_bps: Option<f64>,
    buy_cex_sell_onchain_bps: Option<f64>,
    raw: bool,
}

#[derive(Clone, PartialEq)]
struct ChartProjection {
    buy_onchain_sell_cex_area: String,
    buy_cex_sell_onchain_area: String,
    buy_onchain_sell_cex_path: String,
    buy_cex_sell_onchain_path: String,
    buy_onchain_sell_cex_marker: Option<ChartPoint>,
    buy_cex_sell_onchain_marker: Option<ChartPoint>,
    zero_y: f64,
    top_label: String,
    bottom_label: String,
}

#[derive(Clone, Copy, PartialEq)]
struct ChartPoint {
    x: f64,
    y: f64,
}

#[derive(Clone, PartialEq)]
struct ChartSessionStats {
    buy_onchain_range: String,
    buy_cex_range: String,
    duration: String,
    cadence: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChartFeedStatus {
    Waiting,
    Live,
    Paused,
    Stale,
    Blocked,
    Interrupted,
}

#[derive(Clone, Copy)]
pub(super) struct SpreadHistory {
    samples: RwSignal<Vec<SpreadSample>>,
    status: RwSignal<ChartFeedStatus>,
    expanded: RwSignal<bool>,
}

impl SpreadHistory {
    pub(super) fn new(state: RwSignal<LoadState<OnchainComparisonSnapshot>>) -> Self {
        let samples = RwSignal::new(Vec::<SpreadSample>::with_capacity(MAX_SAMPLES));
        let status = RwSignal::new(ChartFeedStatus::Waiting);
        let expanded = RwSignal::new(false);
        let active_market = RwSignal::new(None::<u64>);
        Effect::new(move |_| {
            let (next_status, market, sample) = state.with(chart_feed_update);
            status.set(next_status);
            if market != active_market.get_untracked() {
                active_market.set(market);
                samples.update(Vec::clear);
            }
            let Some(sample) = sample else { return };
            samples.update(|rows| push_sample(rows, sample));
        });
        Self {
            samples,
            status,
            expanded,
        }
    }
}

pub(super) fn spread_chart(
    history: SpreadHistory,
    active_direction: RwSignal<OnchainComparisonDirection>,
) -> impl IntoView {
    let projection = Memo::new(move |_| chart_projection(&history.samples.get()));
    let session_stats = Memo::new(move |_| chart_session_stats(&history.samples.get()));

    view! {
        <section
            class="onchain-spread-chart"
            class:is-expanded=move || history.expanded.get()
            aria-label="双向套利价差走势"
        >
            <header>
                <div class="onchain-chart-heading">
                    <div>
                        <strong>{move || chart_title(&history.samples.get())}</strong>
                        <span>{move || chart_sample_label(&history.samples.get())}</span>
                    </div>
                    <span
                        class=move || chart_feed_class(history.status.get())
                        title=move || chart_feed_detail(history.status.get())
                    >{move || chart_feed_label(history.status.get())}</span>
                </div>
                <div class="onchain-spread-legend" role="group" aria-label="价差与执行方向">
                    <button
                        type="button"
                        class="is-positive"
                        class:is-selected=move || active_direction.get() == OnchainComparisonDirection::BuyOnchainSellCex
                        aria-pressed=move || (active_direction.get() == OnchainComparisonDirection::BuyOnchainSellCex).to_string()
                        title="查看链上买入、CEX 卖出的报价与执行条件"
                        on:click=move |_| active_direction.set(OnchainComparisonDirection::BuyOnchainSellCex)
                    >
                        "链买 → CEX 卖"
                        <strong class="num">{move || latest_label(&history.samples.get(), true)}</strong>
                    </button>
                    <button
                        type="button"
                        class="is-negative"
                        class:is-selected=move || active_direction.get() == OnchainComparisonDirection::BuyCexSellOnchain
                        aria-pressed=move || (active_direction.get() == OnchainComparisonDirection::BuyCexSellOnchain).to_string()
                        title="查看 CEX 买入、链上卖出的报价与执行条件"
                        on:click=move |_| active_direction.set(OnchainComparisonDirection::BuyCexSellOnchain)
                    >
                        "CEX 买 → 链卖"
                        <strong class="num">{move || latest_label(&history.samples.get(), false)}</strong>
                    </button>
                </div>
                <button
                    type="button"
                    class="onchain-chart-toggle"
                    aria-controls="onchain-spread-detail"
                    aria-expanded=move || history.expanded.get().to_string()
                    on:click=move |_| history.expanded.update(|expanded| *expanded = !*expanded)
                >
                    <span>{move || if history.expanded.get() { "收起" } else { "走势" }}</span>
                    <span class="onchain-chart-toggle-caret" aria-hidden="true">"⌄"</span>
                </button>
            </header>
            <div
                id="onchain-spread-detail"
                class="onchain-spread-detail"
                hidden=move || !history.expanded.get()
            >
                <div class="onchain-spread-canvas">
                    <span class="onchain-chart-bound is-top">{move || projection.get().top_label}</span>
                    <svg
                        viewBox=format!("0 0 {CHART_WIDTH} {CHART_HEIGHT}")
                        preserveAspectRatio="none"
                        role="img"
                        aria-label="本页会话双向价差折线"
                    >
                        <path
                            class="onchain-chart-grid"
                            d="M 0 32 H 640 M 0 63 H 640 M 0 94 H 640 M 128 0 V 126 M 256 0 V 126 M 384 0 V 126 M 512 0 V 126"
                        />
                        <path
                            class="onchain-chart-zero"
                            d=move || format!("M 0 {:.2} H {CHART_WIDTH}", projection.get().zero_y)
                        />
                        <path
                            class="onchain-chart-area is-positive"
                            d=move || projection.get().buy_onchain_sell_cex_area
                        />
                        <path
                            class="onchain-chart-area is-negative"
                            d=move || projection.get().buy_cex_sell_onchain_area
                        />
                        <path
                            class="onchain-chart-line is-positive"
                            d=move || projection.get().buy_onchain_sell_cex_path
                        />
                        <path
                            class="onchain-chart-line is-negative"
                            d=move || projection.get().buy_cex_sell_onchain_path
                        />
                        {move || projection.get().buy_onchain_sell_cex_marker.map(|point| view! {
                            <circle
                                class="onchain-chart-marker is-positive"
                                cx=format!("{:.2}", point.x)
                                cy=format!("{:.2}", point.y)
                                r="3.5"
                            />
                        })}
                        {move || projection.get().buy_cex_sell_onchain_marker.map(|point| view! {
                            <circle
                                class="onchain-chart-marker is-negative"
                                cx=format!("{:.2}", point.x)
                                cy=format!("{:.2}", point.y)
                                r="3.5"
                            />
                        })}
                    </svg>
                    <span class="onchain-chart-bound is-bottom">{move || projection.get().bottom_label}</span>
                    <span
                        class="onchain-chart-zero-label"
                        style=move || format!(
                            "--onchain-chart-zero-y: {:.2}%",
                            projection.get().zero_y / CHART_HEIGHT * 100.0,
                        )
                    >{move || zero_line_label(&history.samples.get())}</span>
                    <div class="onchain-chart-timeline" aria-hidden="true">
                        <span>"会话起点"</span>
                        <span>"实时"</span>
                    </div>
                    {move || (history.samples.get().len() < 2).then(|| view! {
                        <span class="onchain-chart-waiting">"等待连续快照"</span>
                    })}
                    {move || (history.status.get() != ChartFeedStatus::Live
                        && !history.samples.get().is_empty()).then(|| view! {
                        <span class="onchain-chart-retained">"显示上次实时会话"</span>
                    })}
                </div>
                <dl class="onchain-chart-session" aria-label="当前会话价差统计">
                    <div>
                        <dt>"链买区间"</dt>
                        <dd class="num is-positive">{move || session_stats.get().buy_onchain_range}</dd>
                    </div>
                    <div>
                        <dt>"CEX 买区间"</dt>
                        <dd class="num is-negative">{move || session_stats.get().buy_cex_range}</dd>
                    </div>
                    <div>
                        <dt>"会话跨度"</dt>
                        <dd class="num">{move || session_stats.get().duration}</dd>
                    </div>
                    <div>
                        <dt>"平均节奏"</dt>
                        <dd class="num">{move || session_stats.get().cadence}</dd>
                    </div>
                </dl>
            </div>
        </section>
    }
}

fn chart_feed_update(
    state: &LoadState<OnchainComparisonSnapshot>,
) -> (ChartFeedStatus, Option<u64>, Option<SpreadSample>) {
    let market = state.value().map(market_fingerprint);
    match state {
        LoadState::Loading => (ChartFeedStatus::Waiting, market, None),
        LoadState::Error(_) => (ChartFeedStatus::Interrupted, market, None),
        LoadState::Stale { .. } => (ChartFeedStatus::Stale, market, None),
        LoadState::Ready(snapshot) => {
            let status = chart_feed_status(snapshot.quality);
            let sample = (status == ChartFeedStatus::Live)
                .then(|| sample_from_snapshot(snapshot))
                .flatten();
            (status, market, sample)
        }
    }
}

const fn chart_feed_status(quality: OnchainComparisonQuality) -> ChartFeedStatus {
    match quality {
        OnchainComparisonQuality::Disabled => ChartFeedStatus::Paused,
        OnchainComparisonQuality::Pending | OnchainComparisonQuality::ValuationPending => {
            ChartFeedStatus::Waiting
        }
        OnchainComparisonQuality::Stale => ChartFeedStatus::Stale,
        OnchainComparisonQuality::MappingInvalid => ChartFeedStatus::Blocked,
        OnchainComparisonQuality::UpstreamUnavailable => ChartFeedStatus::Interrupted,
        OnchainComparisonQuality::Fresh
        | OnchainComparisonQuality::RawCrossQuote
        | OnchainComparisonQuality::RawCustomPair
        | OnchainComparisonQuality::LowLiquidity
        | OnchainComparisonQuality::NoNetProfit => ChartFeedStatus::Live,
    }
}

fn market_fingerprint(snapshot: &OnchainComparisonSnapshot) -> u64 {
    let config = &snapshot.config;
    let mut hasher = DefaultHasher::new();
    for value in [
        config.chain.as_str(),
        config.provider.as_str(),
        config.base_mint.as_str(),
        config.quote_mint.as_str(),
        config.cex_venue.as_str(),
        config.cex_symbol.as_str(),
    ] {
        value.trim().hash(&mut hasher);
    }
    hasher.finish()
}

const fn chart_feed_label(status: ChartFeedStatus) -> &'static str {
    match status {
        ChartFeedStatus::Waiting => "等待",
        ChartFeedStatus::Live => "实时",
        ChartFeedStatus::Paused => "暂停",
        ChartFeedStatus::Stale => "过期",
        ChartFeedStatus::Blocked => "阻断",
        ChartFeedStatus::Interrupted => "中断",
    }
}

const fn chart_feed_class(status: ChartFeedStatus) -> &'static str {
    match status {
        ChartFeedStatus::Live => "onchain-chart-live is-live",
        ChartFeedStatus::Stale => "onchain-chart-live is-warning",
        ChartFeedStatus::Blocked | ChartFeedStatus::Interrupted => "onchain-chart-live is-danger",
        ChartFeedStatus::Waiting | ChartFeedStatus::Paused => "onchain-chart-live is-neutral",
    }
}

const fn chart_feed_detail(status: ChartFeedStatus) -> &'static str {
    match status {
        ChartFeedStatus::Waiting => "等待第一组可比较的双源实时报价",
        ChartFeedStatus::Live => "折线正在写入双源实时快照",
        ChartFeedStatus::Paused => "监控已暂停，折线停止写入",
        ChartFeedStatus::Stale => "报价已过期，保留上次实时会话但不再追加数据点",
        ChartFeedStatus::Blocked => "资产映射未通过，折线停止写入",
        ChartFeedStatus::Interrupted => "行情来源中断，保留上次实时会话但不再追加数据点",
    }
}

fn sample_from_snapshot(snapshot: &OnchainComparisonSnapshot) -> Option<SpreadSample> {
    if !snapshot.config.enabled || snapshot.comparisons.is_empty() {
        return None;
    }
    let raw = matches!(
        snapshot.quality,
        OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
    );
    let value = |direction| {
        snapshot
            .comparisons
            .iter()
            .find(|row| row.direction == direction)
            .map(|row| {
                if raw {
                    row.gross_spread_bps
                } else {
                    row.net_spread_bps
                }
            })
            .filter(|value| value.is_finite())
    };
    Some(SpreadSample {
        observed_at_ms: snapshot.observed_at_ms,
        buy_onchain_sell_cex_bps: value(OnchainComparisonDirection::BuyOnchainSellCex),
        buy_cex_sell_onchain_bps: value(OnchainComparisonDirection::BuyCexSellOnchain),
        raw,
    })
}

fn push_sample(rows: &mut Vec<SpreadSample>, sample: SpreadSample) {
    if rows.last().is_some_and(|last| {
        last.raw == sample.raw
            && sample.observed_at_ms.saturating_sub(last.observed_at_ms) < MIN_SAMPLE_INTERVAL_MS
    }) {
        return;
    }
    if rows.last().is_some_and(|last| last.raw != sample.raw) {
        rows.clear();
    }
    rows.push(sample);
    if rows.len() > MAX_SAMPLES {
        rows.drain(..rows.len() - MAX_SAMPLES);
    }
}

fn chart_projection(rows: &[SpreadSample]) -> ChartProjection {
    let values = rows
        .iter()
        .flat_map(|row| [row.buy_onchain_sell_cex_bps, row.buy_cex_sell_onchain_bps])
        .flatten()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let (lower, upper) = chart_bounds(&values);
    let zero_y = project_y(0.0, lower, upper);
    ChartProjection {
        buy_onchain_sell_cex_area: series_area_path(
            rows,
            |row| row.buy_onchain_sell_cex_bps,
            lower,
            upper,
            zero_y,
        ),
        buy_cex_sell_onchain_area: series_area_path(
            rows,
            |row| row.buy_cex_sell_onchain_bps,
            lower,
            upper,
            zero_y,
        ),
        buy_onchain_sell_cex_path: series_path(
            rows,
            |row| row.buy_onchain_sell_cex_bps,
            lower,
            upper,
        ),
        buy_cex_sell_onchain_path: series_path(
            rows,
            |row| row.buy_cex_sell_onchain_bps,
            lower,
            upper,
        ),
        buy_onchain_sell_cex_marker: latest_point(
            rows,
            |row| row.buy_onchain_sell_cex_bps,
            lower,
            upper,
        ),
        buy_cex_sell_onchain_marker: latest_point(
            rows,
            |row| row.buy_cex_sell_onchain_bps,
            lower,
            upper,
        ),
        zero_y,
        top_label: percent_label(upper),
        bottom_label: percent_label(lower),
    }
}

fn chart_bounds(values: &[f64]) -> (f64, f64) {
    let (mut lower, mut upper) = values
        .iter()
        .copied()
        .fold((0.0_f64, 0.0_f64), |(lower, upper), value| {
            (lower.min(value), upper.max(value))
        });
    let range = (upper - lower).max(10.0);
    let padding = range * 0.12;
    lower -= padding;
    upper += padding;
    (lower, upper)
}

fn series_path(
    rows: &[SpreadSample],
    value: impl Fn(&SpreadSample) -> Option<f64>,
    lower: f64,
    upper: f64,
) -> String {
    let denominator = rows.len().saturating_sub(1).max(1) as f64;
    let mut path = String::with_capacity(rows.len() * 24);
    let mut first = true;
    for (index, row) in rows.iter().enumerate() {
        let Some(value) = value(row) else { continue };
        let command = if first { 'M' } else { 'L' };
        first = false;
        let x = project_x(index, denominator);
        let y = project_y(value, lower, upper);
        let _ = write!(path, "{command} {x:.2} {y:.2} ");
    }
    path
}

fn series_area_path(
    rows: &[SpreadSample],
    value: impl Fn(&SpreadSample) -> Option<f64>,
    lower: f64,
    upper: f64,
    zero_y: f64,
) -> String {
    let denominator = rows.len().saturating_sub(1).max(1) as f64;
    let points = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            value(row)
                .filter(|value| value.is_finite())
                .map(|value| ChartPoint {
                    x: project_x(index, denominator),
                    y: project_y(value, lower, upper),
                })
        })
        .collect::<Vec<_>>();
    let Some((first, last)) = points.first().zip(points.last()) else {
        return String::new();
    };
    let mut path = String::with_capacity(points.len() * 24 + 48);
    let _ = write!(path, "M {:.2} {zero_y:.2} L ", first.x);
    for point in &points {
        let _ = write!(path, "{:.2} {:.2} ", point.x, point.y);
    }
    let _ = write!(path, "L {:.2} {zero_y:.2} Z", last.x);
    path
}

fn latest_point(
    rows: &[SpreadSample],
    value: impl Fn(&SpreadSample) -> Option<f64>,
    lower: f64,
    upper: f64,
) -> Option<ChartPoint> {
    let denominator = rows.len().saturating_sub(1).max(1) as f64;
    rows.iter().enumerate().rev().find_map(|(index, row)| {
        value(row)
            .filter(|value| value.is_finite())
            .map(|value| ChartPoint {
                x: project_x(index, denominator),
                y: project_y(value, lower, upper),
            })
    })
}

fn project_x(index: usize, denominator: f64) -> f64 {
    CHART_PAD_X + index as f64 / denominator * (CHART_WIDTH - CHART_PAD_X * 2.0)
}

fn project_y(value: f64, lower: f64, upper: f64) -> f64 {
    let usable_height = CHART_HEIGHT - CHART_PAD_Y * 2.0;
    CHART_PAD_Y + (upper - value) / (upper - lower) * usable_height
}

fn chart_title(rows: &[SpreadSample]) -> &'static str {
    if rows.last().is_some_and(|row| row.raw) {
        "原始价差走势"
    } else {
        "费后价差走势"
    }
}

fn chart_sample_label(rows: &[SpreadSample]) -> String {
    if rows.is_empty() {
        "本页会话 · 等待首个实时点".to_owned()
    } else {
        format!("本页会话 · {} 点", rows.len())
    }
}

fn zero_line_label(rows: &[SpreadSample]) -> &'static str {
    if rows.last().is_some_and(|row| row.raw) {
        "0% 原始基准线"
    } else {
        "0% 费后盈亏线"
    }
}

fn latest_label(rows: &[SpreadSample], buy_onchain: bool) -> String {
    rows.last()
        .and_then(|row| {
            if buy_onchain {
                row.buy_onchain_sell_cex_bps
            } else {
                row.buy_cex_sell_onchain_bps
            }
        })
        .map_or_else(|| "--".to_owned(), percent_label)
}

fn chart_session_stats(rows: &[SpreadSample]) -> ChartSessionStats {
    ChartSessionStats {
        buy_onchain_range: series_range_label(rows, |row| row.buy_onchain_sell_cex_bps),
        buy_cex_range: series_range_label(rows, |row| row.buy_cex_sell_onchain_bps),
        duration: sample_duration_label(rows),
        cadence: sample_cadence_label(rows),
    }
}

fn series_range_label(
    rows: &[SpreadSample],
    value: impl Fn(&SpreadSample) -> Option<f64>,
) -> String {
    let range = rows
        .iter()
        .filter_map(value)
        .filter(|value| value.is_finite())
        .fold(None, |range, value| {
            Some(range.map_or((value, value), |(lower, upper): (f64, f64)| {
                (lower.min(value), upper.max(value))
            }))
        });
    let Some((lower, upper)) = range else {
        return "--".to_owned();
    };
    if (upper - lower).abs() < f64::EPSILON {
        percent_label(lower)
    } else {
        format!("{} ~ {}", percent_label(lower), percent_label(upper))
    }
}

fn sample_duration_label(rows: &[SpreadSample]) -> String {
    let Some((first, last)) = rows.first().zip(rows.last()) else {
        return "--".to_owned();
    };
    duration_label(last.observed_at_ms.saturating_sub(first.observed_at_ms))
}

fn sample_cadence_label(rows: &[SpreadSample]) -> String {
    if rows.len() < 2 {
        return "--".to_owned();
    }
    let total = rows
        .last()
        .map(|sample| sample.observed_at_ms)
        .unwrap_or_default()
        .saturating_sub(
            rows.first()
                .map(|sample| sample.observed_at_ms)
                .unwrap_or_default(),
        );
    duration_label(total / rows.len().saturating_sub(1) as i64)
}

fn duration_label(milliseconds: i64) -> String {
    if milliseconds < 1_000 {
        format!("{milliseconds}ms")
    } else if milliseconds < 60_000 {
        format!("{:.1}s", milliseconds as f64 / 1_000.0)
    } else {
        format!("{:.1}m", milliseconds as f64 / 60_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_bounds_keep_zero_and_add_headroom() {
        let (lower, upper) = chart_bounds(&[-520.0, 180.0]);

        assert!(lower < -520.0);
        assert!(upper > 180.0);
        assert!(project_y(0.0, lower, upper).is_finite());
    }

    #[test]
    fn sample_buffer_is_bounded_and_deduplicated() {
        let mut rows = Vec::new();
        for index in 0..=MAX_SAMPLES {
            push_sample(
                &mut rows,
                SpreadSample {
                    observed_at_ms: index as i64 * MIN_SAMPLE_INTERVAL_MS,
                    buy_onchain_sell_cex_bps: Some(index as f64),
                    buy_cex_sell_onchain_bps: Some(-(index as f64)),
                    raw: false,
                },
            );
        }

        assert_eq!(rows.len(), MAX_SAMPLES);
        let latest = rows.last().map(|row| row.observed_at_ms);
        push_sample(
            &mut rows,
            SpreadSample {
                observed_at_ms: latest.unwrap_or_default() + 1,
                buy_onchain_sell_cex_bps: Some(1.0),
                buy_cex_sell_onchain_bps: Some(-1.0),
                raw: false,
            },
        );
        assert_eq!(rows.len(), MAX_SAMPLES);
        assert_eq!(rows.last().map(|row| row.observed_at_ms), latest);
    }

    #[test]
    fn single_snapshot_still_has_visible_latest_markers_and_stats() {
        let rows = [SpreadSample {
            observed_at_ms: 1_000,
            buy_onchain_sell_cex_bps: Some(25.0),
            buy_cex_sell_onchain_bps: Some(-40.0),
            raw: false,
        }];

        let projection = chart_projection(&rows);
        let stats = chart_session_stats(&rows);

        assert!(projection.buy_onchain_sell_cex_marker.is_some());
        assert!(projection.buy_cex_sell_onchain_marker.is_some());
        assert!(projection.buy_onchain_sell_cex_area.ends_with('Z'));
        assert!(projection.buy_cex_sell_onchain_area.ends_with('Z'));
        assert_eq!(
            projection.buy_onchain_sell_cex_marker.map(|point| point.x),
            Some(CHART_PAD_X)
        );
        assert_eq!(stats.buy_onchain_range, "+0.250%");
        assert_eq!(stats.buy_cex_range, "-0.400%");
        assert_eq!(stats.duration, "0ms");
        assert_eq!(stats.cadence, "--");
    }

    #[test]
    fn stale_and_interrupted_feeds_never_append_retained_quotes() {
        assert_eq!(
            chart_feed_status(OnchainComparisonQuality::NoNetProfit),
            ChartFeedStatus::Live
        );
        assert_eq!(
            chart_feed_status(OnchainComparisonQuality::Stale),
            ChartFeedStatus::Stale
        );
        assert_eq!(
            chart_feed_status(OnchainComparisonQuality::UpstreamUnavailable),
            ChartFeedStatus::Interrupted
        );
    }

    #[test]
    fn chart_sample_counter_does_not_expose_internal_buffer_capacity() {
        let rows = [SpreadSample {
            observed_at_ms: 1_000,
            buy_onchain_sell_cex_bps: Some(25.0),
            buy_cex_sell_onchain_bps: Some(-40.0),
            raw: false,
        }];

        assert_eq!(chart_sample_label(&rows), "本页会话 · 1 点");
    }
}
