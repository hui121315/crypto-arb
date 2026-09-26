use crate::api::ws::{WsChannelState, WsStatus};
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{onchain_chain_preset, OnchainComparisonQuality, OnchainComparisonSnapshot};

use super::super::format::{
    cex_source_label, chain_label, freshness_label, provider_label, retry_after_label,
};

pub(in crate::panels::modules::onchain) fn source_telemetry(
    state: RwSignal<LoadState<OnchainComparisonSnapshot>>,
    transport: RwSignal<WsChannelState>,
) -> impl IntoView {
    view! {
        <section class="onchain-source-telemetry" aria-label="报价与节点实时状态">
            <header class="onchain-source-header">
                <div>
                    <strong>"运行来源"</strong>
                    <span>"链上报价、交易所 最优价、节点与换汇"</span>
                </div>
                <div class="onchain-source-columns" aria-hidden="true">
                    <span>"来源"</span>
                    <span>"时效 / 节奏"</span>
                    <span>"状态"</span>
                </div>
            </header>
            <div class="onchain-source-list" role="table">
                {move || telemetry_state(&state.get(), &transport.get())}
            </div>
        </section>
    }
}

fn telemetry_state(
    state: &LoadState<OnchainComparisonSnapshot>,
    transport: &WsChannelState,
) -> AnyView {
    let Some(snapshot) = state.value() else {
        return view! {
            {source_cell("HTTP QUOTE", "链上报价", "等待配置", ("待启动", "待启动"), source_presentation("正在读取来源契约", "is-neutral"))}
            {source_cell("CHAIN RPC", "按需节点", "等待合约", ("待请求", "按需"), source_presentation("公共 RPC 自动读取", "is-neutral"))}
            {source_cell("WS BBO", "交易所 最优价", "等待订阅", ("待订阅", "100ms"), source_presentation("AppWS 正在连接", "is-neutral"))}
        }
        .into_any();
    };
    let onchain = onchain_quote_telemetry(snapshot);
    let cex = cex_quote_telemetry(snapshot, transport);
    let rpc_cell = rpc_source_cell(snapshot);
    let conversion_cell = conversion_source_cell(snapshot);
    view! {
        {source_cell(
            "HTTP QUOTE",
            &provider_label(&snapshot.config.provider),
            &chain_label(&snapshot.config.chain),
            (&freshness_label(snapshot.onchain_freshness_ms), &format!("{}ms", snapshot.quote_interval_ms)),
            source_presentation(onchain.state, onchain.tone),
        )}
        {rpc_cell}
        {source_cell(
            "WS BBO",
            &format!("{} · {}", snapshot.config.cex_venue.to_uppercase(), snapshot.config.cex_symbol),
            &cex_source_label(&snapshot.cex_source),
            (&freshness_label(snapshot.cex_freshness_ms), &format!("{}ms", snapshot.projection_interval_ms)),
            source_presentation(cex.state, cex.tone),
        )}
        {conversion_cell}
        {usd_source_cell(snapshot)}
    }
    .into_any()
}

fn cex_quote_telemetry(
    snapshot: &OnchainComparisonSnapshot,
    transport: &WsChannelState,
) -> QuoteTelemetry {
    let waiting_for_first_snapshot = snapshot.quality == OnchainComparisonQuality::Pending;
    let ws_label = match transport.status {
        WsStatus::Connected if transport.subscribed => "AppWS 已订阅",
        WsStatus::Connected => "AppWS 已连接",
        WsStatus::Connecting => "AppWS 连接中",
        WsStatus::Disconnected => "AppWS 已断开",
    };
    let tone = if !snapshot.config.enabled {
        "is-neutral"
    } else if snapshot.cex_freshness_ms.is_none() && snapshot.cex_problem.is_some() {
        if snapshot.quality == OnchainComparisonQuality::UpstreamUnavailable {
            "is-danger"
        } else {
            "is-warning"
        }
    } else if waiting_for_first_snapshot && snapshot.cex_freshness_ms.is_none() {
        "is-neutral"
    } else if snapshot.cex_freshness_ms.is_none() {
        "is-danger"
    } else if snapshot
        .cex_freshness_ms
        .is_some_and(|age| age > snapshot.config.max_age_ms)
    {
        "is-warning"
    } else {
        "is-positive"
    };
    let state = if snapshot.cex_freshness_ms.is_none() && snapshot.cex_problem.is_some() {
        let retry = snapshot
            .cex_retry_after_ms
            .map(retry_after_label)
            .unwrap_or_else(|| "自动重试".to_owned());
        let state = if snapshot.quality == OnchainComparisonQuality::UpstreamUnavailable {
            "交易所 WS 暂不可用"
        } else {
            "交易所 WS 重连中"
        };
        format!("{state} · {retry} · {ws_label}")
    } else if waiting_for_first_snapshot && snapshot.cex_freshness_ms.is_none() {
        format!("等待首个最优价 · {ws_label}")
    } else if snapshot
        .cex_freshness_ms
        .is_some_and(|age| age > snapshot.config.max_age_ms)
    {
        format!("最后 WS 最优价已过期 · 自动重连 · {ws_label}")
    } else {
        format!("{ws_label} · {} 帧", transport.message_count)
    };
    QuoteTelemetry { state, tone }
}

fn rpc_source_cell(snapshot: &OnchainComparisonSnapshot) -> Option<AnyView> {
    let rpc = &snapshot.rpc_status;
    let rpc_title = rpc
        .endpoint_label
        .clone()
        .unwrap_or_else(|| "系统公共 RPC".to_owned());
    let solana = snapshot.config.chain.eq_ignore_ascii_case("solana");
    let provider_managed = rpc.mode == shared_types::OnchainRpcMode::ProviderManaged;
    let rpc_detail = if provider_managed {
        format!("{} · 按需连接", chain_label(&snapshot.config.chain))
    } else if solana {
        rpc.block_number.map_or_else(
            || "Solana mainnet · Slot 待核对".to_owned(),
            |slot| format!("Solana mainnet · Slot {slot}"),
        )
    } else {
        rpc.expected_chain_id.map_or_else(
            || "EVM Chain 待核对".to_owned(),
            |chain_id| {
                rpc.block_number.map_or_else(
                    || format!("Chain {chain_id}"),
                    |block| format!("Chain {chain_id} · Block #{block}"),
                )
            },
        )
    };
    let rpc_age = rpc
        .observed_at_ms
        .map(|observed| snapshot.observed_at_ms.saturating_sub(observed));
    let rpc_metrics = if provider_managed {
        ("按需".to_owned(), "请求时".to_owned())
    } else {
        (
            freshness_label(rpc_age),
            rpc.latency_ms
                .map_or_else(|| "待请求".to_owned(), |latency| format!("{latency}ms")),
        )
    };
    let rpc_state = if provider_managed {
        "输入合约或读取余额时连接".to_owned()
    } else {
        rpc.problem
            .clone()
            .unwrap_or_else(|| "主网身份与最新高度已核对".to_owned())
    };
    let rpc_tone = if provider_managed {
        "is-neutral"
    } else if rpc.ready {
        "is-positive"
    } else {
        "is-danger"
    };
    onchain_chain_preset(&snapshot.config.chain).map(|_| {
        source_cell(
            if solana { "SOLANA RPC" } else { "EVM RPC" },
            &rpc_title,
            &rpc_detail,
            (&rpc_metrics.0, &rpc_metrics.1),
            source_presentation(rpc_state, rpc_tone),
        )
        .into_any()
    })
}

fn conversion_source_cell(snapshot: &OnchainComparisonSnapshot) -> Option<AnyView> {
    snapshot.quote_conversion.as_ref().map(|conversion| {
        source_cell(
            "WS FX",
            &format!(
                "{} · {}",
                conversion.venue.to_uppercase(),
                conversion.symbol
            ),
            &format!("{} → {}", conversion.cex_quote, conversion.onchain_quote),
            (&freshness_label(Some(conversion.freshness_ms)), "100ms"),
            source_presentation("实时 WS 报价换算已取得", "is-positive"),
        )
        .into_any()
    })
}

fn usd_source_cell(snapshot: &OnchainComparisonSnapshot) -> impl IntoView {
    let (title, detail, age, state, tone) = snapshot.quote_usd_valuation.as_ref().map_or_else(
        || {
            (
                format!("{}/USD", snapshot.config.quote_token),
                "汇率未就绪".to_owned(),
                "待取证".to_owned(),
                "美元金额与净收益待估值".to_owned(),
                "is-warning",
            )
        },
        |row| {
            let age = snapshot.observed_at_ms.saturating_sub(row.observed_at_ms);
            let fresh = age >= 0 && age <= snapshot.config.max_age_ms;
            (
                if row.venue.is_empty() {
                    row.symbol.clone()
                } else {
                    format!("{} · {}", row.venue.to_uppercase(), row.symbol)
                },
                format!("1 {} = ${:.6}", row.asset, row.usd_bid),
                freshness_label(Some(age)),
                if fresh {
                    "美元估值".to_owned()
                } else {
                    "估值已过期".to_owned()
                },
                if fresh { "is-positive" } else { "is-warning" },
            )
        },
    );
    let pace = if snapshot
        .quote_usd_valuation
        .as_ref()
        .is_some_and(|row| row.source == "same_currency")
    {
        "同币"
    } else {
        "WS"
    };
    source_cell(
        "USD VALUE",
        &title,
        &detail,
        (&age, pace),
        source_presentation(state, tone),
    )
}

struct QuoteTelemetry {
    state: String,
    tone: &'static str,
}

struct SourcePresentation {
    state: String,
    tone: &'static str,
}

fn source_presentation(state: impl Into<String>, tone: &'static str) -> SourcePresentation {
    SourcePresentation {
        state: state.into(),
        tone,
    }
}

fn onchain_quote_telemetry(snapshot: &OnchainComparisonSnapshot) -> QuoteTelemetry {
    if !snapshot.config.enabled {
        return QuoteTelemetry {
            state: snapshot
                .provider_problem
                .clone()
                .unwrap_or_else(|| "报价服务已完成交易检查，监控未启用".to_owned()),
            tone: if snapshot.provider_configured {
                "is-neutral"
            } else {
                "is-warning"
            },
        };
    }
    if !snapshot.provider_configured {
        return QuoteTelemetry {
            state: snapshot
                .provider_problem
                .clone()
                .unwrap_or_else(|| "报价服务配置未就绪".to_owned()),
            tone: "is-danger",
        };
    }
    let has_complete_quote = snapshot.quote_evidence.len() >= 2
        && snapshot.quote_observed_at_ms.is_some()
        && snapshot.onchain_freshness_ms.is_some();
    if let Some(problem) = snapshot.provider_problem.as_deref() {
        return QuoteTelemetry {
            state: provider_failure_state(
                problem,
                snapshot.provider_retry_after_ms,
                has_complete_quote,
            ),
            tone: if has_complete_quote || is_rate_limit_problem(problem) {
                "is-warning"
            } else {
                "is-danger"
            },
        };
    }
    if !has_complete_quote {
        let state = match snapshot.quality {
            OnchainComparisonQuality::Pending => "等待首轮双向只读报价".to_owned(),
            OnchainComparisonQuality::UpstreamUnavailable => {
                snapshot.provider_retry_after_ms.map_or_else(
                    || "链上报价失败 · 正在自动重试".to_owned(),
                    |delay| format!("链上报价失败 · {}", retry_after_label(delay)),
                )
            }
            OnchainComparisonQuality::MappingInvalid => "链上报价身份未通过".to_owned(),
            OnchainComparisonQuality::Stale => "链上报价已过期，正在刷新".to_owned(),
            _ => "尚未取得完整双向报价".to_owned(),
        };
        return QuoteTelemetry {
            state,
            tone: if snapshot.quality == OnchainComparisonQuality::Pending {
                "is-neutral"
            } else if snapshot.quality == OnchainComparisonQuality::UpstreamUnavailable {
                "is-danger"
            } else {
                "is-warning"
            },
        };
    }
    if snapshot
        .onchain_freshness_ms
        .is_some_and(|age| age > snapshot.config.max_age_ms)
    {
        return QuoteTelemetry {
            state: "链上报价已过期，正在刷新".to_owned(),
            tone: "is-warning",
        };
    }
    QuoteTelemetry {
        state: "官方双向只读报价已取得".to_owned(),
        tone: "is-positive",
    }
}

fn provider_failure_state(problem: &str, retry_after_ms: Option<i64>, cached: bool) -> String {
    let retry = retry_after_ms
        .map(retry_after_label)
        .unwrap_or_else(|| "自动重试".to_owned());
    let state = if is_rate_limit_problem(problem) {
        "报价服务配额退避"
    } else if is_timeout_problem(problem) {
        "报价服务响应超时"
    } else if is_connection_problem(problem) {
        "报价服务连接失败"
    } else {
        "报价服务刷新失败"
    };
    if cached {
        format!("{state} · 保留上次报价 · {retry}")
    } else {
        format!("{state} · {retry}")
    }
}

fn is_rate_limit_problem(problem: &str) -> bool {
    let problem = problem.to_ascii_lowercase();
    problem.contains("http 429") || problem.contains("rate limit") || problem.contains("已限速")
}

fn is_timeout_problem(problem: &str) -> bool {
    let problem = problem.to_ascii_lowercase();
    problem.contains("timeout") || problem.contains("timed out") || problem.contains("超时")
}

fn is_connection_problem(problem: &str) -> bool {
    let problem = problem.to_ascii_lowercase();
    problem.contains("request failed") || problem.contains("无法连接") || problem.contains("tls")
}

fn source_cell(
    transport: &str,
    title: &str,
    detail: &str,
    metrics: (&str, &str),
    presentation: SourcePresentation,
) -> impl IntoView {
    let SourcePresentation { state, tone } = presentation;
    let state_title = state.clone();
    view! {
        <div class="onchain-source-cell" role="row">
            <div class="onchain-source-primary">
                <div class="onchain-source-title">
                    <span>{transport.to_owned()}</span>
                    <strong title=title.to_owned()>{title.to_owned()}</strong>
                </div>
                <p>{detail.to_owned()}</p>
            </div>
            <div class="onchain-source-metrics">
                <div><span>"年龄"</span><strong>{metrics.0.to_owned()}</strong></div>
                <div><span>"节奏"</span><strong>{metrics.1.to_owned()}</strong></div>
            </div>
            <div class=format!("onchain-source-state {tone}") title=state_title>{state}</div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::OnchainQuoteEvidence;

    #[test]
    fn usd_valuation_renders_the_actual_rate_and_missing_state() {
        Owner::new().with(|| {
            let mut snapshot = OnchainComparisonSnapshot::default();
            let missing = usd_source_cell(&snapshot).to_html();
            assert!(missing.contains("待估值"));
            assert!(!missing.contains("= $1.000000"));
            snapshot.quote_usd_valuation = Some(shared_types::OnchainUsdValuation {
                asset: "USDC".to_owned(),
                venue: "kraken".to_owned(),
                symbol: "USDC/USD".to_owned(),
                source: "ws_push".to_owned(),
                usd_bid: 0.9,
                usd_ask: 1.1,
                observed_at_ms: 0,
            });
            let html = usd_source_cell(&snapshot).to_html();
            assert!(html.contains("KRAKEN"));
            assert!(html.contains("1 USDC = $0.900000"));
            assert!(html.contains("role=\"row\""));
            if let Ok(path) = std::env::var("USD_VALUATION_RENDER_PATH") {
                let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/styles/.generated/input.css")).unwrap();
                std::fs::write(path, format!("<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>美元估值状态</title><style>{css}</style><body><section class=onchain-source-telemetry><div class=onchain-source-list role=table>{html}{missing}</div></section></body></html>")).unwrap();
            }
        });
    }

    #[test]
    fn provider_configuration_never_impersonates_quote_evidence() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.enabled = true;
        snapshot.provider_configured = true;
        snapshot.quality = OnchainComparisonQuality::UpstreamUnavailable;
        snapshot.provider_retry_after_ms = Some(12_000);

        let telemetry = onchain_quote_telemetry(&snapshot);

        assert_eq!(telemetry.tone, "is-danger");
        assert!(telemetry.state.contains("报价失败"));
        assert!(telemetry.state.contains("12.0s 后重试"));
        assert!(!telemetry.state.contains("已取得"));
    }

    #[test]
    fn complete_fresh_pair_is_reported_as_observed() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.enabled = true;
        snapshot.quality = OnchainComparisonQuality::Fresh;
        snapshot.quote_evidence = vec![quote_evidence(), quote_evidence()];
        snapshot.quote_observed_at_ms = Some(1_000);
        snapshot.onchain_freshness_ms = Some(25);

        let telemetry = onchain_quote_telemetry(&snapshot);

        assert_eq!(telemetry.tone, "is-positive");
        assert_eq!(telemetry.state, "官方双向只读报价已取得");
    }

    #[test]
    fn complete_but_old_pair_is_reported_as_stale() {
        let mut snapshot = OnchainComparisonSnapshot::default();
        snapshot.config.enabled = true;
        snapshot.quality = OnchainComparisonQuality::Stale;
        snapshot.quote_evidence = vec![quote_evidence(), quote_evidence()];
        snapshot.quote_observed_at_ms = Some(1_000);
        snapshot.onchain_freshness_ms = Some(snapshot.config.max_age_ms + 1);

        let telemetry = onchain_quote_telemetry(&snapshot);

        assert_eq!(telemetry.tone, "is-warning");
        assert!(telemetry.state.contains("已过期"));
    }

    #[test]
    fn rate_limit_is_reported_as_quota_backoff() {
        let state = provider_failure_state("Jupiter API 已限速（HTTP 429）", Some(10_000), false);

        assert_eq!(state, "报价服务配额退避 · 10.0s 后重试");
    }

    #[test]
    fn timeout_keeps_the_last_complete_quote_visible() {
        let state =
            provider_failure_state("Jupiter 报价超时 · transport=timeout", Some(5_000), true);

        assert_eq!(state, "报价服务响应超时 · 保留上次报价 · 5.0s 后重试");
    }

    fn quote_evidence() -> OnchainQuoteEvidence {
        OnchainQuoteEvidence {
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
