use super::*;
use shared_types::instrument_registry::InstrumentAssetClass;

pub(super) fn normalize_canonical_symbol(symbol: &str) -> String {
    exchange::strip_common_suffixes(symbol.trim()).to_ascii_uppercase()
}

pub(super) fn executable_venue(
    coverage: &InstrumentCoverageEntry,
    venue: &str,
    now_ms: i64,
) -> bool {
    coverage
        .venues
        .iter()
        .any(|entry| entry.venue.eq_ignore_ascii_case(venue) && entry.is_executable_leg(now_ms))
}

pub(super) fn listing_blocker(
    coverage: &InstrumentCoverageEntry,
    long_venue: &str,
    short_venue: &str,
    now_ms: i64,
) -> String {
    let blocked_legs = [long_venue, short_venue]
        .into_iter()
        .filter_map(|venue| blocked_leg_label(coverage, venue, now_ms))
        .collect::<Vec<_>>();
    format!(
        "{LISTING_BLOCKER_PREFIX}{}，仅观察。",
        blocked_legs.join("；")
    )
}

fn blocked_leg_label(
    coverage: &InstrumentCoverageEntry,
    venue: &str,
    now_ms: i64,
) -> Option<String> {
    let entry = coverage
        .venues
        .iter()
        .find(|entry| entry.venue.eq_ignore_ascii_case(venue));
    if entry.is_some_and(|entry| entry.is_executable_leg(now_ms)) {
        return None;
    }
    let reason = match entry.map(|entry| entry.state) {
        Some(VenueListingState::Listed) => entry
            .and_then(|entry| entry.problem.as_ref())
            .map_or("官方已挂牌但执行规格未通过", |problem| {
                problem.message.as_str()
            }),
        Some(VenueListingState::Unlisted) => "官方 metadata 当前未挂牌",
        Some(VenueListingState::Failed) => "instrument registry 刷新失败",
        Some(VenueListingState::Stale) => "instrument registry 证据已过期",
        Some(VenueListingState::Unsupported) => "官方 instrument metadata 未接入",
        Some(VenueListingState::Unknown) | None => "instrument registry 尚未刷新",
    };
    Some(format!("{} {reason}", venue.to_ascii_uppercase()))
}

pub(super) fn instrument_execution_problem(instrument: &VenueInstrument) -> Option<ApiProblem> {
    if instrument.venue.eq_ignore_ascii_case("binance")
        && instrument.listing_status == InstrumentListingStatus::Trading
        && !instrument.execution_supported
        && instrument.asset_class != InstrumentAssetClass::Crypto
    {
        return Some(
            ApiProblem::new(
                "BINANCE_TRADIFI_EXECUTION_PENDING",
                "TradFi 永续已在官方 exchangeInfo 挂牌；专用下单、撤单与私有终态语义尚未获得官方证据",
            )
            .with_source("instrument-registry"),
        );
    }
    if !instrument.venue.eq_ignore_ascii_case("gate")
        || instrument.listing_status != InstrumentListingStatus::Trading
        || instrument.execution_supported
    {
        return None;
    }
    if !instrument.native_symbol.is_ascii() {
        return Some(
            ApiProblem::new(
                "GATE_NATIVE_SYMBOL_EXECUTION_PENDING",
                "GATE 非 ASCII 原生合约公共挂牌与盘口已核验；下单及私有订单/仓位的符号编码协议尚未验收",
            )
            .with_source("instrument-registry"),
        );
    }
    if instrument.qty_step.is_none() {
        return Some(
            ApiProblem::new(
                "GATE_DECIMAL_EXECUTION_PENDING",
                "GATE 小数合约公共规格与盘口已核验；下单及私有订单/仓位小数协议尚未验收",
            )
            .with_source("instrument-registry"),
        );
    }
    Some(
        ApiProblem::new(
            "GATE_EXECUTION_SPEC_PENDING",
            "GATE 公共挂牌与盘口已核验；交易写路径完整执行规格尚未验收",
        )
        .with_source("instrument-registry"),
    )
}

pub(super) fn coverage_state_label(state: VenueListingState) -> &'static str {
    match state {
        VenueListingState::Listed => "已挂牌",
        VenueListingState::Unlisted => "未挂牌",
        VenueListingState::Failed => "探测失败",
        VenueListingState::Stale => "证据过期",
        VenueListingState::Unsupported => "不支持",
        VenueListingState::Unknown => "待探测",
    }
}

pub(super) fn metadata_source_label(source: InstrumentMetadataSource) -> &'static str {
    match source {
        InstrumentMetadataSource::OfficialEndpoint => "官方端点",
        InstrumentMetadataSource::CachedSnapshot => "缓存快照",
        InstrumentMetadataSource::Manual => "人工",
        InstrumentMetadataSource::Unverified => "未核验",
    }
}

pub(super) struct CoverageEvidence {
    pub(super) source: InstrumentMetadataSource,
    pub(super) execution_ready: bool,
    pub(super) stale_after_ms: Option<i64>,
    pub(super) problem: Option<ApiProblem>,
}

pub(super) fn coverage_entry(
    venue: &str,
    native_symbol: &str,
    state: VenueListingState,
    checked_at_ms: i64,
    evidence: CoverageEvidence,
) -> VenueCoverageEntry {
    VenueCoverageEntry {
        venue: venue.to_owned(),
        native_symbol: native_symbol.to_owned(),
        state,
        source: evidence.source,
        execution_ready: evidence.execution_ready,
        checked_at_ms: checked_at_ms.max(1),
        stale_after_ms: evidence.stale_after_ms,
        problem: evidence.problem,
    }
}
