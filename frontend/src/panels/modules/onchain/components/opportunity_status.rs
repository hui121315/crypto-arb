use shared_types::{
    OnchainCexComparison, OnchainComparisonQuality, OnchainComparisonSnapshot,
    OnchainDirectionReadiness,
};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct OpportunityStatus {
    pub(super) label: &'static str,
    pub(super) short_label: &'static str,
    pub(super) detail: String,
    pub(super) tone: &'static str,
}

pub(super) fn opportunity_status(snapshot: &OnchainComparisonSnapshot) -> OpportunityStatus {
    match snapshot.quality {
        OnchainComparisonQuality::Disabled => status(
            "监控已暂停",
            "已暂停",
            "当前配置已保存，但两边报价与机会判断没有运行。",
            "is-neutral",
        ),
        OnchainComparisonQuality::Pending => status(
            "等待两边报价",
            "报价读取中",
            "正在等待链上和交易所的最新买卖价格。",
            "is-neutral",
        ),
        OnchainComparisonQuality::Fresh => fresh_status(snapshot),
        OnchainComparisonQuality::ValuationPending => status(
            "等待美元估值",
            "待估值",
            snapshot
                .degradation_reasons
                .first()
                .map(String::as_str)
                .unwrap_or("计价币兑美元的实时汇率尚未取得，暂不能计算预计净收益。"),
            "is-warning",
        ),
        OnchainComparisonQuality::RawCrossQuote => status(
            "计价币不同，仅供对比",
            "计价币不同",
            "两边计价币尚未按实时汇率换算；只展示原始价格，不判断收益。",
            "is-warning",
        ),
        OnchainComparisonQuality::RawCustomPair => status(
            "自定义资产观察",
            "自定义观察",
            "链上和交易所选的不是同一种资产；只展示原始价格，不判断收益。",
            "is-warning",
        ),
        OnchainComparisonQuality::Stale if snapshot.onchain_freshness_ms.is_none()
            || snapshot.cex_freshness_ms.is_none() => status(
            "更新时间待确认",
            "更新时间待确认",
            stale_detail(snapshot),
            "is-warning",
        ),
        OnchainComparisonQuality::Stale => status(
            "报价已过期",
            "报价过期",
            stale_detail(snapshot),
            "is-warning",
        ),
        OnchainComparisonQuality::LowLiquidity => status(
            "可成交金额待检查",
            "金额待检查",
            "当前最佳报价下可成交金额不足；创建交易计划时会检查更多买卖挂单。",
            "is-warning",
        ),
        OnchainComparisonQuality::MappingInvalid => status(
            "尚未确认是同一资产",
            "资产待核对",
            "尚未确认链上和交易所选的是同一种资产，暂不能创建交易计划。",
            "is-danger",
        ),
        OnchainComparisonQuality::UpstreamUnavailable => status(
            "行情源中断",
            "行情中断",
            unavailable_detail(snapshot),
            "is-danger",
        ),
        OnchainComparisonQuality::NoNetProfit => status(
            "暂无符合条件的机会",
            "暂无机会",
            "按当前两边价格测算，扣除手续费、成交价偏差和网络费后，收益尚未达到设定条件。",
            "is-neutral",
        ),
    }
}

fn stale_detail(snapshot: &OnchainComparisonSnapshot) -> String {
    if let Some(reason) = snapshot.degradation_reasons.first() { return reason.clone(); }
    match (
        snapshot.provider_problem.as_deref(),
        snapshot.cex_problem.as_deref(),
    ) {
        (Some(_), Some(_)) => "链上和交易所报价都没有更新；当前只显示上次结果，系统正在分别重连。若持续超过 1 分钟，优先检查项目代理或海外网络。".to_owned(),
        (Some(_), None) => "链上报价没有更新；当前只显示上次结果，系统会等待一段时间后重试。".to_owned(),
        (None, Some(_)) => format!(
            "{} 实时报价没有更新；当前只显示上次结果，系统正在自动重连。",
            snapshot.config.cex_venue.to_uppercase(),
        ),
        (None, None) => "至少一边报价已超过有效时间；当前只显示上次结果，等待自动刷新后再判断。".to_owned(),
    }
}

fn unavailable_detail(snapshot: &OnchainComparisonSnapshot) -> String {
    match (
        snapshot.provider_problem.as_deref(),
        snapshot.cex_problem.as_deref(),
    ) {
        (Some(_), Some(_)) => "链上和交易所都没有可用报价，系统正在分别重连。若持续超过 1 分钟，优先检查项目代理或海外网络。".to_owned(),
        (Some(_), None) => "链上暂时没有可用报价，系统会等待一段时间后重试。".to_owned(),
        (None, Some(_)) => format!(
            "{} 暂时没有可用实时报价，系统正在自动重连。",
            snapshot.config.cex_venue.to_uppercase(),
        ),
        (None, None) => "链上或交易所行情暂不可用；系统正在自动恢复。".to_owned(),
    }
}

fn fresh_status(snapshot: &OnchainComparisonSnapshot) -> OpportunityStatus {
    let Some(candidate) = best_profitable_direction(snapshot) else {
        return status(
            "结论待复核",
            "待复核",
            "报价已更新，但还没有找到预计收益达到设定条件的方向。",
            "is-danger",
        );
    };
    let readiness = direction_readiness(snapshot, candidate);
    if readiness.is_some_and(|row| row.build_ready) {
        return status(
            "预计有收益 · 可创建计划",
            "可创建计划",
            "余额、交易规则与钱包连接已准备好；创建计划时仍会检查可成交金额和交易权限。",
            "is-positive",
        );
    }
    let blocker = readiness
        .and_then(|row| row.blockers.first())
        .or_else(|| snapshot.execution_readiness.global_blockers.first())
        .cloned()
        .unwrap_or_else(|| "交易前的必要信息尚未确认".to_owned());
    status(
        "预计有收益 · 尚不能交易",
        "交易条件未齐",
        format!("按当前价格预计有收益，但尚不能创建交易计划：{blocker}"),
        "is-warning",
    )
}

fn best_profitable_direction(
    snapshot: &OnchainComparisonSnapshot,
) -> Option<&OnchainCexComparison> {
    let minimum_bps = snapshot.config.spread_alert.min_net_spread_bps.max(0.0);
    snapshot
        .comparisons
        .iter()
        .filter(|row| row.net_spread_bps > 0.0 && row.net_spread_bps >= minimum_bps)
        .max_by(|left, right| left.net_spread_bps.total_cmp(&right.net_spread_bps))
}

fn direction_readiness<'a>(
    snapshot: &'a OnchainComparisonSnapshot,
    candidate: &OnchainCexComparison,
) -> Option<&'a OnchainDirectionReadiness> {
    snapshot
        .execution_readiness
        .directions
        .iter()
        .find(|row| row.direction == candidate.direction)
}

fn status(
    label: &'static str,
    short_label: &'static str,
    detail: impl Into<String>,
    tone: &'static str,
) -> OpportunityStatus {
    OpportunityStatus {
        label,
        short_label,
        detail: detail.into(),
        tone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        OnchainCexInstrumentEvidence, OnchainComparisonDirection, OnchainDirectionReadiness,
    };

    #[test]
    fn fresh_profit_is_not_called_actionable_without_readiness() {
        let snapshot = profitable_snapshot(false);

        let status = opportunity_status(&snapshot);

        assert_eq!(status.label, "预计有收益 · 尚不能交易");
        assert_eq!(status.short_label, "交易条件未齐");
        assert_eq!(status.tone, "is-warning");
        assert!(status.detail.contains("钱包地址"));
    }

    #[test]
    fn fresh_profit_becomes_buildable_only_with_direction_evidence() {
        let snapshot = profitable_snapshot(true);

        let status = opportunity_status(&snapshot);

        assert_eq!(status.label, "预计有收益 · 可创建计划");
        assert_eq!(status.short_label, "可创建计划");
        assert_eq!(status.tone, "is-positive");
    }

    #[test]
    fn raw_cross_quote_never_uses_profit_language() {
        let mut snapshot = profitable_snapshot(true);
        snapshot.quality = OnchainComparisonQuality::RawCrossQuote;

        let status = opportunity_status(&snapshot);

        assert_eq!(status.label, "计价币不同，仅供对比");
        assert_eq!(status.short_label, "计价币不同");
        assert!(!status.label.contains("盈利"));
    }

    #[test]
    fn stale_status_explains_dual_source_failure_without_treating_cache_as_live() {
        let mut snapshot = profitable_snapshot(false);
        snapshot.quality = OnchainComparisonQuality::Stale;
        snapshot.provider_problem = Some("provider timeout".to_owned());
        snapshot.cex_problem = Some("websocket disconnected".to_owned());

        let status = opportunity_status(&snapshot);

        assert_eq!(status.short_label, "报价过期");
        assert!(status.detail.contains("只显示上次结果"));
        assert!(status.detail.contains("项目代理或海外网络"));
    }

    fn profitable_snapshot(build_ready: bool) -> OnchainComparisonSnapshot {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quality = OnchainComparisonQuality::Fresh;
        snapshot.comparisons = vec![OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            onchain_price: 1.0,
            cex_price: 1.02,
            gross_spread_bps: 200.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 10.0,
            gas_usd: 0.01,
            gas_bps: 1.0,
            total_cost_bps: 21.0,
            net_spread_bps: 179.0,
            observable_notional_usd: 100.0,
            executable: false,
        }];
        snapshot.execution_readiness.global_blockers = vec!["链上钱包地址未配置".to_owned()];
        snapshot.execution_readiness.directions = vec![OnchainDirectionReadiness {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            path: Default::default(),
            inventory: Vec::new(),
            cex_instrument: OnchainCexInstrumentEvidence::default(),
            build_ready,
            submit_ready: false,
            blockers: (!build_ready)
                .then(|| "链上钱包地址未配置".to_owned())
                .into_iter()
                .collect(),
        }];
        snapshot
    }
}
