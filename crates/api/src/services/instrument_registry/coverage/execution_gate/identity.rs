use shared_types::instrument_registry::{InstrumentAssetClass, VenueInstrument};
use shared_types::{
    ArbitrageOpportunityDto, IndexCompositionEvidence, IndexCompositionQuality,
    IndexCompositionStatus, StrategyKind,
};

const ECONOMIC_IDENTITY_BLOCKER_PREFIX: &str = "经济标的身份未通过：";
const MAX_EXECUTABLE_PRICE_SCALE_RATIO: f64 = 4.0;

pub(super) fn economic_identity_blocker(
    opportunity: &ArbitrageOpportunityDto,
    long: &VenueInstrument,
    short: &VenueInstrument,
) -> Option<String> {
    if long.asset_class == InstrumentAssetClass::Unknown
        || short.asset_class == InstrumentAssetClass::Unknown
    {
        return Some(format!(
            "{ECONOMIC_IDENTITY_BLOCKER_PREFIX}两腿资产类别未完成官方核验，仅观察"
        ));
    }
    if long.asset_class != short.asset_class {
        return Some(format!(
            "{ECONOMIC_IDENTITY_BLOCKER_PREFIX}{}={}、{}={}，同名代码不代表同一经济标的，仅观察",
            long.venue.to_ascii_uppercase(),
            asset_class_label(long.asset_class),
            short.venue.to_ascii_uppercase(),
            asset_class_label(short.asset_class),
        ));
    }
    if opportunity.strategy_kind == Some(StrategyKind::PerpCross) {
        if let Some(blocker) = perp_cross_quote_blocker(long, short) {
            return Some(blocker);
        }
    }
    if let Some(ratio) = price_scale_ratio(opportunity.long_price, opportunity.short_price)
        .filter(|ratio| *ratio > MAX_EXECUTABLE_PRICE_SCALE_RATIO)
    {
        return Some(format!(
            "{ECONOMIC_IDENTITY_BLOCKER_PREFIX}两腿可执行价格尺度相差 {ratio:.1} 倍，可能为同名异资产或合约单位不一致，仅观察"
        ));
    }
    if long.asset_class != InstrumentAssetClass::Crypto
        && !has_verified_underlying_identity(opportunity)
    {
        return Some(
            opportunity
                .index_composition
                .as_ref()
                .and_then(|profile| profile.blocker.clone())
                .unwrap_or_else(|| {
                    format!(
                        "{ECONOMIC_IDENTITY_BLOCKER_PREFIX}非加密资产缺少双边官方底层成分证据，仅观察"
                    )
                }),
        );
    }
    None
}

fn perp_cross_quote_blocker(long: &VenueInstrument, short: &VenueInstrument) -> Option<String> {
    let long_quote = match instrument_quote(long) {
        Ok(quote) => quote,
        Err(reason) => return Some(format!("{ECONOMIC_IDENTITY_BLOCKER_PREFIX}{reason}")),
    };
    let short_quote = match instrument_quote(short) {
        Ok(quote) => quote,
        Err(reason) => return Some(format!("{ECONOMIC_IDENTITY_BLOCKER_PREFIX}{reason}")),
    };
    if long_quote.eq_ignore_ascii_case(short_quote) {
        return None;
    }
    Some(format!(
        "{ECONOMIC_IDENTITY_BLOCKER_PREFIX}{}={}、{}={}，未配置报价币风险对冲，仅观察",
        long.venue.to_ascii_uppercase(),
        long_quote.to_ascii_uppercase(),
        short.venue.to_ascii_uppercase(),
        short_quote.to_ascii_uppercase(),
    ))
}

fn instrument_quote(instrument: &VenueInstrument) -> Result<&str, String> {
    let quote = instrument
        .quote_asset
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let settle = instrument
        .settle_asset
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (quote, settle) {
        (Some(quote), Some(settle)) if quote.eq_ignore_ascii_case(settle) => Ok(quote),
        (Some(quote), Some(settle)) => Err(format!(
            "{} 报价={}、结算={}，非线性结算尚未支持，仅观察",
            instrument.venue.to_ascii_uppercase(),
            quote.to_ascii_uppercase(),
            settle.to_ascii_uppercase(),
        )),
        _ => Err(format!(
            "{} 报价/结算资产未完成官方核验，仅观察",
            instrument.venue.to_ascii_uppercase(),
        )),
    }
}

fn has_verified_underlying_identity(opportunity: &ArbitrageOpportunityDto) -> bool {
    let Some(profile) = opportunity.index_composition.as_ref() else {
        return false;
    };
    profile.status == IndexCompositionStatus::Verified
        && profile.long_quality == IndexCompositionQuality::Verified
        && profile.short_quality == IndexCompositionQuality::Verified
        && profile.overlap_score >= 0.75
        && valid_composition_evidence(profile.long_evidence.as_ref())
        && valid_composition_evidence(profile.short_evidence.as_ref())
}

fn valid_composition_evidence(evidence: Option<&IndexCompositionEvidence>) -> bool {
    evidence
        .is_some_and(|evidence| !evidence.source.trim().is_empty() && evidence.received_at_ms > 0)
}

const fn asset_class_label(asset_class: InstrumentAssetClass) -> &'static str {
    match asset_class {
        InstrumentAssetClass::Crypto => "加密资产",
        InstrumentAssetClass::Equity => "股票",
        InstrumentAssetClass::Index => "指数",
        InstrumentAssetClass::Metal => "金属",
        InstrumentAssetClass::Energy => "能源",
        InstrumentAssetClass::Forex => "外汇",
        InstrumentAssetClass::Unknown => "未知",
    }
}

fn price_scale_ratio(long_price: Option<f64>, short_price: Option<f64>) -> Option<f64> {
    let long_price = long_price.filter(|value| value.is_finite() && *value > 0.0)?;
    let short_price = short_price.filter(|value| value.is_finite() && *value > 0.0)?;
    Some(long_price.max(short_price) / long_price.min(short_price))
}
