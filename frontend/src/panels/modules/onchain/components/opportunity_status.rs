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
            "当前配置已保存，但双源报价与机会判断没有运行。",
            "is-neutral",
        ),
        OnchainComparisonQuality::Pending => status(
            "等待双源",
            "双源读取中",
            "正在等待链上报价和 交易所 WS 最优价同时就绪。",
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
                .unwrap_or("Quote/USD 官方 WS 汇率未就绪，暂不计算净利润。"),
            "is-warning",
        ),
        OnchainComparisonQuality::RawCrossQuote => status(
            "跨 Quote 观察",
            "跨 Quote",
            "两边 Quote 尚未完成实时汇率换算；只展示原始价格，不判断净收益。",
            "is-warning",
        ),
        OnchainComparisonQuality::RawCustomPair => status(
            "自定义资产观察",
            "自定义观察",
            "链上 Base 与 交易所 Base 不是同一资产；只展示原始价格，不判断净收益。",
            "is-warning",
        ),
        OnchainComparisonQuality::Stale if snapshot.onchain_freshness_ms.is_none()
            || snapshot.cex_freshness_ms.is_none() => status(
            "时效待确认",
            "时效待确认",
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
            "深度待核对",
            "待核深度",
            "最优档预览没有覆盖目标金额；构建时读取完整深度并重新计算。",
            "is-warning",
        ),
        OnchainComparisonQuality::MappingInvalid => status(
            "身份映射阻断",
            "映射阻断",
            "链上资产与所选 交易所 市场身份未通过核对，不能判断或构建交易。",
            "is-danger",
        ),
        OnchainComparisonQuality::UpstreamUnavailable => status(
            "行情源中断",
            "行情中断",
            unavailable_detail(snapshot),
            "is-danger",
        ),
        OnchainComparisonQuality::NoNetProfit => status(
            "暂无费后利润",
            "未盈利",
            "当前双向价差无法覆盖手续费、滑点、Gas 与配置门槛。",
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
        (Some(_), Some(_)) => "链上报价和 交易所 WS 都没有新数据；当前只显示上次结果，系统正在分别重连。若持续超过 1 分钟，优先检查项目代理或海外网络。".to_owned(),
        (Some(_), None) => "链上 报价服务 没有刷新；当前只显示上次结果，系统正在按退避时间重试。".to_owned(),
        (None, Some(_)) => format!(
            "{} WS 没有刷新；当前只显示上次结果，系统正在自动重连。",
            snapshot.config.cex_venue.to_uppercase(),
        ),
        (None, None) => "至少一边报价超过新鲜度上限；当前只显示上次结果，等待自动刷新后再判断。".to_owned(),
    }
}

fn unavailable_detail(snapshot: &OnchainComparisonSnapshot) -> String {
    match (
        snapshot.provider_problem.as_deref(),
        snapshot.cex_problem.as_deref(),
    ) {
        (Some(_), Some(_)) => "链上 报价服务 与 交易所 WS 都没有可用报价，系统正在分别重连。若持续超过 1 分钟，优先检查项目代理或海外网络。".to_owned(),
        (Some(_), None) => "链上 报价服务 暂时没有可用报价，系统正在按退避时间重试。".to_owned(),
        (None, Some(_)) => format!(
            "{} WS 暂时没有可用报价，系统正在自动重连。",
            snapshot.config.cex_venue.to_uppercase(),
        ),
        (None, None) => "链上 报价服务 或 交易所 行情暂不可用；系统正在自动恢复。".to_owned(),
    }
}

fn fresh_status(snapshot: &OnchainComparisonSnapshot) -> OpportunityStatus {
    let Some(candidate) = best_profitable_direction(snapshot) else {
        return status(
            "结论待复核",
            "待复核",
            "快照标记为新鲜，但没有找到达到门槛的正收益方向。",
            "is-danger",
        );
    };
    let readiness = direction_readiness(snapshot, candidate);
    if readiness.is_some_and(|row| row.build_ready) {
        return status(
            "费后盈利 · 可构建",
            "可构建",
            "基础库存、规格与链上接入已就绪；构建时会读取完整深度并远程复核权限。",
            "is-positive",
        );
    }
    let blocker = readiness
        .and_then(|row| row.blockers.first())
        .or_else(|| snapshot.execution_readiness.global_blockers.first())
        .cloned()
        .unwrap_or_else(|| "执行准备数据依据尚未建立".to_owned());
    status(
        "费后盈利 · 待接入",
        "盈利待接入",
        format!("价格存在费后利润，但现在还不能构建：{blocker}"),
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

        assert_eq!(status.label, "费后盈利 · 待接入");
        assert_eq!(status.short_label, "盈利待接入");
        assert_eq!(status.tone, "is-warning");
        assert!(status.detail.contains("钱包地址"));
    }

    #[test]
    fn fresh_profit_becomes_buildable_only_with_direction_evidence() {
        let snapshot = profitable_snapshot(true);

        let status = opportunity_status(&snapshot);

        assert_eq!(status.label, "费后盈利 · 可构建");
        assert_eq!(status.short_label, "可构建");
        assert_eq!(status.tone, "is-positive");
    }

    #[test]
    fn raw_cross_quote_never_uses_profit_language() {
        let mut snapshot = profitable_snapshot(true);
        snapshot.quality = OnchainComparisonQuality::RawCrossQuote;

        let status = opportunity_status(&snapshot);

        assert_eq!(status.label, "跨 Quote 观察");
        assert_eq!(status.short_label, "跨 Quote");
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
