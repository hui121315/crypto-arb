use shared_types::{OnchainComparisonQuality, OnchainComparisonSnapshot};

use super::super::format::retry_after_label;

pub(super) struct DecisionGuidance {
    pub status: &'static str,
    pub title: String,
    pub detail: String,
    pub next_step: String,
    pub technical: Option<String>,
    pub tone: &'static str,
}

pub(super) fn empty_decision_guidance(snapshot: &OnchainComparisonSnapshot) -> DecisionGuidance {
    match snapshot.quality {
        OnchainComparisonQuality::ValuationPending => guidance(
            "待估值",
            "美元计价尚未就绪",
            "缺少计价币兑美元的汇率，暂不显示预计美元收益，也不会发出盈利机会提醒。",
            "确认当前交易所或 Kraken 的现货行情订阅已开启，等待最新美元交易对报价。",
            technical_problem(snapshot),
            "is-warning",
        ),
        OnchainComparisonQuality::MappingInvalid => guidance(
            "资产待核对",
            "尚未确认链上和交易所是同一种资产",
            "当前不会计算可执行收益，也不会构建交易计划。",
            "核对代币合约识别结果，以及交易所选中的买卖币种。",
            technical_problem(snapshot),
            "is-danger",
        ),
        OnchainComparisonQuality::UpstreamUnavailable
            if !has_complete_onchain_quote(snapshot) && snapshot.cex_freshness_ms.is_none() =>
        {
            guidance(
                "两边报价未就绪",
                "链上和交易所都还没有报价",
                "当前没有可比较的双边价格，不会计算收益或构建交易计划。",
                "先检查本机代理与网络连通性；保持监控开启，两边报价恢复后会自动继续。",
                technical_problem(snapshot),
                "is-danger",
            )
        }
        OnchainComparisonQuality::UpstreamUnavailable if snapshot.cex_freshness_ms.is_none() => {
            guidance(
                "行情中断",
                format!(
                    "{} {} 实时报价暂不可用",
                    snapshot.config.cex_venue.to_uppercase(),
                    snapshot.config.cex_symbol
                ),
                "链上报价会保留；交易所最佳买卖价格恢复后自动继续比较。",
                "保持监控开启；若持续失败，再检查本机代理与交易所连通性。",
                technical_problem(snapshot),
                "is-danger",
            )
        }
        OnchainComparisonQuality::UpstreamUnavailable => {
            let next_step = snapshot.provider_retry_after_ms.map_or_else(
                || "检查报价服务的 API Key、请求额度及链上节点连接。".to_owned(),
                |delay| format!("系统将在 {}；无需重新应用配置。", retry_after_label(delay)),
            );
            guidance(
                "报价中断",
                "链上报价服务暂不可用",
                "交易所报价会保留；链上报价恢复后自动继续比较。",
                next_step,
                technical_problem(snapshot),
                "is-danger",
            )
        }
        OnchainComparisonQuality::Pending if snapshot.cex_freshness_ms.is_none() => guidance(
            "连接中",
            format!(
                "正在等待 {} {} 的首批报价",
                snapshot.config.cex_venue.to_uppercase(),
                snapshot.config.cex_symbol
            ),
            "链上报价已独立运行；收到交易所最新买卖价格后立即开始比较。",
            "保持监控开启，连接恢复后无需重新应用配置。",
            technical_problem(snapshot),
            "is-warning",
        ),
        OnchainComparisonQuality::Pending if snapshot.provider_problem.is_some() => guidance(
            "连接中",
            "正在等待首次链上报价",
            "交易所实时报价会继续保留，链上报价恢复后自动比较。",
            "保持监控开启；系统会等待一段时间后重试。",
            technical_problem(snapshot),
            "is-warning",
        ),
        OnchainComparisonQuality::Stale => guidance(
            "报价过期",
            "两边报价尚未同时更新",
            "旧价格不会进入收益判断或交易计划。",
            "等待链上和交易所的最新报价。",
            technical_problem(snapshot),
            "is-warning",
        ),
        _ => guidance(
            "准备中",
            "等待首个可比较报价",
            "链上和所选交易所买卖币种的价格都需要是最新的。",
            "保持监控开启，收到两边报价后自动显示两个方向的对比结果。",
            technical_problem(snapshot),
            "is-neutral",
        ),
    }
}

fn has_complete_onchain_quote(snapshot: &OnchainComparisonSnapshot) -> bool {
    snapshot.quote_observed_at_ms.is_some() && snapshot.quote_evidence.len() >= 2
}

fn technical_problem(snapshot: &OnchainComparisonSnapshot) -> Option<String> {
    let mut problems = Vec::new();
    for problem in snapshot
        .degradation_reasons
        .iter()
        .chain(snapshot.provider_problem.iter())
        .chain(snapshot.cex_problem.iter())
    {
        let stale_disabled_reason = problem == "on-chain comparison is disabled"
            && snapshot.quality != OnchainComparisonQuality::Disabled;
        if !problem.trim().is_empty()
            && !stale_disabled_reason
            && !problems.iter().any(|row| row == problem)
        {
            problems.push(problem.clone());
        }
    }
    (!problems.is_empty()).then(|| problems.join("；"))
}

fn guidance(
    status: &'static str,
    title: impl Into<String>,
    detail: impl Into<String>,
    next_step: impl Into<String>,
    technical: Option<String>,
    tone: &'static str,
) -> DecisionGuidance {
    DecisionGuidance {
        status,
        title: title.into(),
        detail: detail.into(),
        next_step: next_step.into(),
        technical,
        tone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_cex_state_has_one_operational_next_step() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.cex_venue = "kraken".to_owned();
        snapshot.config.cex_symbol = "PUPS/USD".to_owned();
        snapshot.quality = OnchainComparisonQuality::Pending;
        snapshot.cex_problem = Some("inbound idle timeout".to_owned());

        let result = empty_decision_guidance(&snapshot);

        assert_eq!(result.status, "连接中");
        assert!(result.title.contains("KRAKEN PUPS/USD"));
        assert!(result.next_step.contains("无需重新应用配置"));
        assert_eq!(result.technical.as_deref(), Some("inbound idle timeout"));
    }

    #[test]
    fn identity_failure_never_looks_like_a_loading_state() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quality = OnchainComparisonQuality::MappingInvalid;
        snapshot.degradation_reasons = vec!["base identity mismatch".to_owned()];

        let result = empty_decision_guidance(&snapshot);

        assert_eq!(result.status, "资产待核对");
        assert_eq!(result.tone, "is-danger");
        assert!(result.next_step.contains("代币合约"));
    }

    #[test]
    fn simultaneous_provider_and_cex_failure_is_not_misreported_as_cex_only() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quality = OnchainComparisonQuality::UpstreamUnavailable;
        snapshot.degradation_reasons = vec!["Jupiter 报价无法连接".to_owned()];
        snapshot.cex_problem = Some("Kraken WS 正在重连".to_owned());

        let result = empty_decision_guidance(&snapshot);

        assert_eq!(result.status, "两边报价未就绪");
        assert!(result.title.contains("链上报价"));
        let technical = result.technical.unwrap_or_default();
        assert!(technical.contains("Jupiter"));
        assert!(technical.contains("Kraken"));
    }

    #[test]
    fn cex_only_failure_keeps_the_existing_onchain_quote_statement() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quality = OnchainComparisonQuality::UpstreamUnavailable;
        snapshot.quote_observed_at_ms = Some(1_000);
        snapshot.quote_evidence = vec![quote_evidence(), quote_evidence()];
        snapshot.cex_problem = Some("Kraken WS 正在重连".to_owned());

        let result = empty_decision_guidance(&snapshot);

        assert_eq!(result.status, "行情中断");
        assert!(result.detail.contains("链上报价会保留"));
    }

    #[test]
    fn provider_backoff_is_reported_as_a_wait_not_a_user_action() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.quality = OnchainComparisonQuality::UpstreamUnavailable;
        snapshot.cex_freshness_ms = Some(10);
        snapshot.provider_retry_after_ms = Some(8_500);

        let result = empty_decision_guidance(&snapshot);

        assert_eq!(result.status, "报价中断");
        assert!(result.next_step.contains("8.5s 后重试"));
        assert!(result.next_step.contains("无需重新应用配置"));
    }

    fn quote_evidence() -> shared_types::OnchainQuoteEvidence {
        shared_types::OnchainQuoteEvidence {
            provider: "provider".to_owned(),
            endpoint: "https://example.test/quote".to_owned(),
            official_docs_url: "https://example.test/docs".to_owned(),
            input_mint: "base".to_owned(),
            output_mint: "quote".to_owned(),
            input_amount_raw: "1".to_owned(),
            output_amount_raw: "1".to_owned(),
            router: None,
            transaction_requested: false,
            observed_at_ms: 1_000,
        }
    }
}
