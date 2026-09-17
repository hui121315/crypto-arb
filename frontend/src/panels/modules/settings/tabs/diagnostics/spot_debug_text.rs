use leptos::prelude::*;
use shared_types::{InstrumentMetadataSource, SpotTick, VenueCoverageEntry, VenueListingState};

pub(super) fn listing_state_label(state: VenueListingState) -> &'static str {
    match state {
        VenueListingState::Listed => "已挂牌（仍须 P0 双腿门禁）",
        VenueListingState::Unlisted => "未挂牌",
        VenueListingState::Failed => "探测失败",
        VenueListingState::Stale => "证据过期",
        VenueListingState::Unsupported => "不支持",
        VenueListingState::Unknown => "待探测",
    }
}

pub(super) fn listing_evidence_label(entry: &VenueCoverageEntry) -> String {
    let source = match entry.source {
        InstrumentMetadataSource::OfficialEndpoint => "官方端点",
        InstrumentMetadataSource::CachedSnapshot => "缓存快照",
        InstrumentMetadataSource::Manual => "人工",
        InstrumentMetadataSource::Unverified => "未核验",
    };
    let stale = entry
        .stale_after_ms
        .map(|value| format!(" · 截止 {value}"))
        .unwrap_or_default();
    let problem = entry
        .problem
        .as_ref()
        .map(|value| format!(" · {}", value.code))
        .unwrap_or_default();
    format!(
        "{source} · {} · 核验 {}{stale}{problem}",
        entry.native_symbol, entry.checked_at_ms
    )
}

pub(super) fn spot_tick_row(tick: SpotTick) -> impl IntoView {
    let sizes = format!(
        "{} / {}",
        decimal_or_missing(tick.bid_size),
        decimal_or_missing(tick.ask_size)
    );
    let timestamp = tick_timestamp_label(&tick);
    view! {
        <tr>
            <td>{tick.venue}</td>
            <td>{tick.symbol}</td>
            <td>{format!("{} / {}", tick.bid, tick.ask)}</td>
            <td>{sizes}</td>
            <td><em>{timestamp}</em></td>
        </tr>
    }
}

fn decimal_or_missing<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "缺证据".to_owned(), |size| size.to_string())
}

/// 交易所官方时间戳优先，缺失时明示为本地落地时间，不混同两者。
fn tick_timestamp_label(tick: &SpotTick) -> String {
    tick.exchange_ts_ms.map_or_else(
        || format!("本地落地 {}", tick.received_at_ms),
        |timestamp| format!("交易所 {timestamp}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(exchange_ts_ms: Option<i64>) -> SpotTick {
        SpotTick {
            venue: "binance".into(),
            symbol: "MUUSDT".into(),
            bid: 1.into(),
            ask: 2.into(),
            last: 1.into(),
            bid_size: None,
            ask_size: Some(3.into()),
            volume_24h: 0.into(),
            exchange_ts_ms,
            received_at_ms: 42,
        }
    }

    #[test]
    fn timestamp_label_prefers_exchange_ts_and_flags_local_fallback() {
        assert_eq!(tick_timestamp_label(&tick(Some(7))), "交易所 7");
        assert_eq!(tick_timestamp_label(&tick(None)), "本地落地 42");
    }

    #[test]
    fn missing_sizes_render_as_missing_evidence_not_zero() {
        let tick = tick(None);
        assert_eq!(decimal_or_missing(tick.bid_size), "缺证据");
        assert_eq!(decimal_or_missing(tick.ask_size), "3");
    }

    #[test]
    fn listing_labels_stay_observation_only() {
        assert_eq!(
            listing_state_label(VenueListingState::Listed),
            "已挂牌（仍须 P0 双腿门禁）"
        );
        assert_eq!(listing_state_label(VenueListingState::Unknown), "待探测");
    }
}
