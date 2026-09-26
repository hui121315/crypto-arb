use crate::panels::modules::timestamp::utc_hms;
use shared_types::{
    onchain_chain_preset, onchain_quote_provider, OnchainComparisonDirection,
    OnchainComparisonQuality,
};

pub(super) const fn direction_label(direction: OnchainComparisonDirection) -> &'static str {
    match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => "链上买入 → 交易所卖出",
        OnchainComparisonDirection::BuyCexSellOnchain => "交易所买入 → 链上卖出",
    }
}

pub(super) const fn quality_label(quality: OnchainComparisonQuality) -> &'static str {
    match quality {
        OnchainComparisonQuality::Disabled => "监控未启用",
        OnchainComparisonQuality::Pending => "首轮读取中",
        OnchainComparisonQuality::ValuationPending => "美元估值待核实",
        OnchainComparisonQuality::Fresh => "报价已更新",
        OnchainComparisonQuality::RawCrossQuote => "计价币不同，仅供对比",
        OnchainComparisonQuality::RawCustomPair => "自选币种，仅供对比",
        OnchainComparisonQuality::Stale => "报价已过期",
        OnchainComparisonQuality::LowLiquidity => "待确认可成交金额",
        OnchainComparisonQuality::MappingInvalid => "尚未确认是同一资产",
        OnchainComparisonQuality::UpstreamUnavailable => "报价服务暂不可用",
        OnchainComparisonQuality::NoNetProfit => "未达收益门槛",
    }
}

pub(super) const fn quality_tone(quality: OnchainComparisonQuality) -> &'static str {
    match quality {
        OnchainComparisonQuality::Fresh => "is-positive",
        OnchainComparisonQuality::Disabled
        | OnchainComparisonQuality::Pending
        | OnchainComparisonQuality::NoNetProfit => "is-neutral",
        OnchainComparisonQuality::RawCrossQuote
        | OnchainComparisonQuality::ValuationPending
        | OnchainComparisonQuality::RawCustomPair
        | OnchainComparisonQuality::Stale
        | OnchainComparisonQuality::LowLiquidity => "is-warning",
        OnchainComparisonQuality::MappingInvalid
        | OnchainComparisonQuality::UpstreamUnavailable => "is-danger",
    }
}

pub(super) const fn quality_reason_label(quality: OnchainComparisonQuality) -> &'static str {
    match quality {
        OnchainComparisonQuality::Disabled => "尚未开始比较链上与交易所的价格。",
        OnchainComparisonQuality::Pending => "正在等待链上和交易所两边的首批报价。",
        OnchainComparisonQuality::ValuationPending => {
            "计价币兑美元的实时汇率缺失或过期，暂不能显示净利润和美元金额。"
        }
        OnchainComparisonQuality::Fresh => "两边报价时效、资产身份和收益门槛已通过检查；创建计划时仍要检查完整盘口。",
        OnchainComparisonQuality::RawCrossQuote => {
            "两边计价币不同；只展示未换算的原始价格差，不把它当成净利润或净亏损。"
        }
        OnchainComparisonQuality::RawCustomPair => {
            "链上与交易所买卖的是不同资产；只对比两个市场的原始价格，不判断套利利润。"
        }
        OnchainComparisonQuality::Stale => "链上或交易所报价已过期，需要等待更新。",
        OnchainComparisonQuality::LowLiquidity => {
            "交易所当前最优价的挂单金额不足；这只是预览，创建计划时读取完整盘口确认可成交金额。"
        }
        OnchainComparisonQuality::MappingInvalid => "尚不能确认链上代币与所选交易对是同一资产。",
        OnchainComparisonQuality::UpstreamUnavailable => {
            "链上报价服务或交易所行情暂不可用。"
        }
        OnchainComparisonQuality::NoNetProfit => "当前双向费后净差均未达到配置的最低收益门槛。",
    }
}

pub(super) fn percent_label(bps: f64) -> String {
    format!("{:+.3}%", bps / 100.0)
}

pub(super) fn cost_percent_label(bps: f64) -> String {
    format!("{:.3}%", bps.abs() / 100.0)
}

pub(super) fn price_label(value: f64) -> String {
    if !value.is_finite() {
        return "--".to_owned();
    }
    let absolute = value.abs();
    if absolute >= 1_000.0 {
        format!("{value:.2}")
    } else if absolute >= 1.0 {
        format!("{value:.4}")
    } else if absolute >= 0.01 {
        format!("{value:.6}")
    } else {
        format!("{value:.8}")
    }
}

pub(super) fn usd(value: f64) -> String {
    format!("${value:.2}")
}

pub(super) fn provider_label(provider: &str) -> String {
    onchain_quote_provider(provider)
        .map_or_else(|| provider.to_owned(), |option| option.label.to_owned())
}

pub(super) fn chain_label(chain: &str) -> String {
    onchain_chain_preset(chain).map_or_else(|| chain.to_owned(), |preset| preset.label.to_owned())
}

pub(super) fn cex_source_label(source: &str) -> String {
    match source {
        "ws_push" => "交易所实时推送的最优价".to_owned(),
        "ws_pending" => "等待交易所实时报价".to_owned(),
        "rest_baseline" => "交易所定期查询报价".to_owned(),
        "rest_cold_start" => "交易所首次查询报价".to_owned(),
        "local_cache" => "交易所上次报价".to_owned(),
        "not_started" => "交易所行情尚未启动".to_owned(),
        value => format!("交易所 来源 {value}"),
    }
}

pub(super) fn freshness_label(value: Option<i64>) -> String {
    value.map_or_else(
        || "未知".to_owned(),
        |ms| {
            if ms < 1_000 {
                format!("{ms}ms")
            } else {
                format!("{:.1}s", ms as f64 / 1_000.0)
            }
        },
    )
}

pub(super) fn retry_after_label(delay_ms: i64) -> String {
    if delay_ms < 1_000 {
        format!("{delay_ms}ms 后重试")
    } else {
        format!("{:.1}s 后重试", delay_ms as f64 / 1_000.0)
    }
}

pub(super) fn time_label(ms: i64) -> String {
    utc_hms(ms).unwrap_or_else(|| "未知".to_owned())
}

pub(super) fn raw_amount_label(raw: &str, decimals: u8, token: &str) -> String {
    let token = token.trim();
    let raw = raw.trim();
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return format!("{raw} {token}");
    }
    let decimals = usize::from(decimals);
    let value = if decimals == 0 {
        raw.to_owned()
    } else if raw.len() <= decimals {
        format!("0.{}{}", "0".repeat(decimals - raw.len()), raw)
    } else {
        let split = raw.len() - decimals;
        format!("{}.{}", &raw[..split], &raw[split..])
    };
    let value = if decimals == 0 {
        value.as_str()
    } else {
        value.trim_end_matches('0').trim_end_matches('.')
    };
    format!("{} {token}", if value.is_empty() { "0" } else { value })
}

pub(super) fn compact_identity(value: &str) -> String {
    if value.chars().count() <= 18 {
        return value.to_owned();
    }
    let prefix = value.chars().take(8).collect::<String>();
    let suffix = value
        .chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}…{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_amount_keeps_decimal_precision_without_float_conversion() {
        assert_eq!(raw_amount_label("1000000000", 9, "SOL"), "1 SOL");
        assert_eq!(raw_amount_label("1250000", 6, "USDC"), "1.25 USDC");
        assert_eq!(raw_amount_label("1000", 0, "TOKEN"), "1000 TOKEN");
        assert_eq!(raw_amount_label("100000000", 6, "USDC"), "100 USDC");
        assert_eq!(raw_amount_label("1", 18, "ETH"), "0.000000000000000001 ETH");
    }

    #[test]
    fn price_and_cost_labels_match_trading_density() {
        assert_eq!(cost_percent_label(21.25), "0.212%");
        assert_eq!(price_label(63_084.125), "63084.12");
        assert_eq!(price_label(0.002_345_678), "0.00234568");
    }

    #[test]
    fn provider_labels_follow_the_shared_catalog() {
        assert_eq!(chain_label("solana"), "Solana");
        assert!(provider_label("okx_dex_v6").ends_with("Aggregator V6"));
        assert_eq!(provider_label("zeroex_swap_v2"), "0x Swap API V2");
        assert_eq!(provider_label("cow_protocol"), "CoW Protocol Fast Quote");
    }

    #[test]
    fn cex_source_labels_keep_ws_and_rest_distinct() {
        assert_eq!(cex_source_label("ws_push"), "交易所实时推送的最优价");
        assert_eq!(cex_source_label("ws_pending"), "等待交易所实时报价");
        assert_eq!(cex_source_label("rest_baseline"), "交易所定期查询报价");
        assert_eq!(cex_source_label("local_cache"), "交易所上次报价");
        assert_eq!(cex_source_label("not_started"), "交易所行情尚未启动");
    }

    #[test]
    fn quality_reasons_follow_the_shared_quality_contract() {
        assert_eq!(
            quality_reason_label(OnchainComparisonQuality::NoNetProfit),
            "当前双向费后净差均未达到配置的最低收益门槛。"
        );
        assert!(
            quality_reason_label(OnchainComparisonQuality::RawCrossQuote)
                .contains("不把它当成净利润")
        );
        assert!(
            quality_reason_label(OnchainComparisonQuality::RawCustomPair)
                .contains("不同资产")
        );
        assert!(
            quality_reason_label(OnchainComparisonQuality::MappingInvalid).contains("尚不能确认")
        );
        assert!(
            quality_reason_label(OnchainComparisonQuality::UpstreamUnavailable)
                .contains("暂不可用")
        );
        assert_eq!(
            quality_label(OnchainComparisonQuality::LowLiquidity),
            "待确认可成交金额"
        );
        assert!(quality_reason_label(OnchainComparisonQuality::LowLiquidity)
            .contains("创建计划时读取完整盘口"));
    }
}
