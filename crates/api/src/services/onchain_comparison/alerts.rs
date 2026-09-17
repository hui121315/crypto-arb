use crate::services::webhook;
use crate::state::AppState;
use shared_types::{
    onchain_cex_base_token, onchain_cex_pair_matches, onchain_cex_quote_token,
    OnchainBatchItemSnapshot, OnchainCexComparison, OnchainComparisonConfig,
    OnchainComparisonDirection, OnchainComparisonQuality, OnchainComparisonSnapshot,
    OnchainCrossChainQuality, OnchainCrossChainSnapshot, OnchainDexComparisonQuality,
    OnchainDexRouteComparison, OnchainSpreadAlertMode, WebhookEventKind,
};

const EVENT_ID_KEY: &[u8] = b"crossline-onchain-spread-v1";
const DEX_EVENT_ID_KEY: &[u8] = b"crossline-onchain-dex-cross-v1";
const CROSS_CHAIN_EVENT_ID_KEY: &[u8] = b"crossline-onchain-cross-chain-v1";

pub(super) fn crossed_alert_threshold(
    previous: &OnchainComparisonSnapshot,
    next: &OnchainComparisonSnapshot,
) -> bool {
    let Some(next_row) = alert_candidate(next) else {
        return false;
    };
    let Some(previous_row) = alert_candidate(previous) else {
        return true;
    };
    !same_alert_identity(
        &previous.config,
        previous_row.direction,
        &next.config,
        next_row.direction,
    )
}

pub(super) fn batch_crossed_alert_threshold(
    previous: Option<&OnchainBatchItemSnapshot>,
    next: &OnchainComparisonSnapshot,
) -> bool {
    let Some(next_row) = alert_candidate(next) else {
        return false;
    };
    let Some(previous) = previous else {
        return true;
    };
    let Some(previous_direction) = batch_alert_direction(previous) else {
        return true;
    };
    !same_alert_identity(
        &previous.config,
        previous_direction,
        &next.config,
        next_row.direction,
    )
}

fn batch_alert_direction(item: &OnchainBatchItemSnapshot) -> Option<OnchainComparisonDirection> {
    if !item.config.spread_alert.enabled {
        return None;
    }
    let threshold_met = match item.config.spread_alert.mode {
        OnchainSpreadAlertMode::VerifiedNet if item.quality == OnchainComparisonQuality::Fresh => {
            item.best_net_spread_bps.is_some_and(|spread| {
                spread > 0.0 && spread >= item.config.spread_alert.min_net_spread_bps.max(0.0)
            })
        }
        OnchainSpreadAlertMode::RawObservation
            if raw_observation_sources_are_fresh(item.quality) =>
        {
            item.best_gross_spread_bps
                .is_some_and(|spread| spread >= item.config.spread_alert.min_raw_spread_bps)
        }
        _ => false,
    };
    threshold_met.then_some(item.best_direction).flatten()
}

fn same_alert_identity(
    previous: &OnchainComparisonConfig,
    previous_direction: OnchainComparisonDirection,
    next: &OnchainComparisonConfig,
    next_direction: OnchainComparisonDirection,
) -> bool {
    previous.chain == next.chain
        && previous.base_mint == next.base_mint
        && previous.quote_mint == next.quote_mint
        && previous.cex_venue == next.cex_venue
        && previous.cex_symbol == next.cex_symbol
        && previous.spread_alert.mode == next.spread_alert.mode
        && previous_direction == next_direction
}

pub(super) async fn emit_if_due(
    state: &AppState,
    snapshot: &OnchainComparisonSnapshot,
    now_ms: i64,
) {
    let Some(row) = alert_candidate(snapshot) else {
        return;
    };
    let status = webhook::status(state).await;
    if !status.config.enabled
        || !status.config.url_configured
        || (status.config.provider == shared_types::WebhookProvider::Generic
            && !status.config.secret_configured)
        || !status
            .config
            .event_kinds
            .contains(&WebhookEventKind::OnchainSpread)
    {
        return;
    }
    let event_id = event_id(snapshot, row, now_ms);
    let payload = alert_payload(snapshot, row);
    if let Err(error) =
        webhook::emit_idempotent(state, WebhookEventKind::OnchainSpread, event_id, payload).await
    {
        tracing::warn!(error = %error, "failed to queue on-chain spread webhook");
    }
}

pub(super) async fn emit_dex_if_due(
    state: &AppState,
    snapshot: &OnchainComparisonSnapshot,
    now_ms: i64,
) {
    let Some(route) = dex_alert_candidate(snapshot) else {
        return;
    };
    let status = webhook::status(state).await;
    if !status.config.enabled
        || !status.config.url_configured
        || (status.config.provider == shared_types::WebhookProvider::Generic
            && !status.config.secret_configured)
        || !status
            .config
            .event_kinds
            .contains(&WebhookEventKind::OnchainSpread)
    {
        return;
    }
    let event_id = dex_event_id(snapshot, route, now_ms);
    let payload = dex_alert_payload(snapshot, route);
    if let Err(error) =
        webhook::emit_idempotent(state, WebhookEventKind::OnchainSpread, event_id, payload).await
    {
        tracing::warn!(error = %error, "failed to queue DEX cross spread webhook");
    }
}

pub(super) async fn emit_cross_chain_if_due(
    state: &AppState,
    snapshot: &OnchainComparisonSnapshot,
    now_ms: i64,
) {
    if !cross_chain_alert_candidate(snapshot, now_ms) {
        return;
    }
    let status = webhook::status(state).await;
    if !status.config.enabled
        || !status.config.url_configured
        || (status.config.provider == shared_types::WebhookProvider::Generic
            && !status.config.secret_configured)
        || !status
            .config
            .event_kinds
            .contains(&WebhookEventKind::OnchainSpread)
    {
        return;
    }
    let event_id = cross_chain_event_id(snapshot, now_ms);
    let payload = cross_chain_alert_payload(snapshot);
    if let Err(error) =
        webhook::emit_idempotent(state, WebhookEventKind::OnchainSpread, event_id, payload).await
    {
        tracing::warn!(error = %error, "failed to queue cross-chain spread webhook");
    }
}

fn cross_chain_alert_candidate(snapshot: &OnchainComparisonSnapshot, now_ms: i64) -> bool {
    snapshot.config.spread_alert.enabled
        && snapshot.config.cross_chain.enabled
        && snapshot.config.spread_alert.mode == OnchainSpreadAlertMode::VerifiedNet
        && snapshot.cross_chain.quality == OnchainCrossChainQuality::Fresh
        && super::usd_valuation::rate(
            snapshot.cross_chain.quote_usd_valuation.as_ref(),
            &snapshot.config.quote_token,
            snapshot.config.max_age_ms,
            now_ms,
        )
        .is_some()
        && snapshot.cross_chain.net_return_bps.is_some_and(|net| {
            net > 0.0 && net >= snapshot.config.spread_alert.min_net_spread_bps.max(0.0)
        })
}

fn cross_chain_alert_payload(snapshot: &OnchainComparisonSnapshot) -> serde_json::Value {
    let route: &OnchainCrossChainSnapshot = &snapshot.cross_chain;
    let net = route.net_return_bps.unwrap_or_default();
    serde_json::json!({
        "summary": format!(
            "{} → {} → {} {}/{} 跨链闭环费后回报 {:.4}%，非原子，仅监控",
            snapshot.config.chain,
            route.peer_chain.as_deref().unwrap_or("未知目标链"),
            snapshot.config.chain,
            snapshot.config.base_token,
            snapshot.config.quote_token,
            net / 100.0,
        ),
        "marketType": "cross_chain_closed_cycle",
        "profitabilityVerified": true,
        "netCostCalculated": true,
        "bridgeEvidenceVerified": route.legs.len() == 4,
        "atomic": false,
        "executable": false,
        "monitorOnly": true,
        "sourceChain": snapshot.config.chain,
        "peerChain": route.peer_chain,
        "provider": route.provider,
        "initialQuoteAmountRaw": route.initial_quote_amount_raw,
        "finalQuoteAmountRaw": route.final_quote_amount_raw,
        "netReturnBps": route.net_return_bps,
        "totalCostBps": route.total_cost_bps,
        "bridgeFeeUsd": route.bridge_fee_usd,
        "gasUsd": route.gas_usd,
        "usdValuation": route.quote_usd_valuation,
        "currencyRiskBps": route.stablecoin_risk_bps,
        "estimatedDurationSeconds": route.estimated_duration_seconds,
        "legs": route.legs,
        "inventory": route.inventory,
        "executionBlockers": [
            "路径包含两次跨链终局等待，不能作为原子即时套利自动提交",
            "构建前必须重新核验源链 Quote、两条链 Gas 与每条桥的最新最小到账"
        ],
        "snapshotObservedAtMs": route.observed_at_ms,
    })
}

fn cross_chain_event_id(snapshot: &OnchainComparisonSnapshot, now_ms: i64) -> String {
    let cooldown = snapshot.config.spread_alert.cooldown_ms.max(1);
    let bucket = now_ms.div_euclid(cooldown);
    let canonical = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        snapshot.config.chain,
        snapshot.config.base_mint,
        snapshot.config.quote_mint,
        snapshot.config.cross_chain.peer_item_id,
        snapshot.config.cross_chain.provider,
        bucket,
    );
    let digest = common::signing::hmac_sha256_hex(CROSS_CHAIN_EVENT_ID_KEY, canonical.as_bytes());
    format!("onchain-cross-chain-{}", &digest[..24])
}

fn dex_alert_candidate(snapshot: &OnchainComparisonSnapshot) -> Option<&OnchainDexRouteComparison> {
    if !snapshot.config.spread_alert.enabled
        || !snapshot.config.dex_comparison.enabled
        || snapshot.config.spread_alert.mode != OnchainSpreadAlertMode::VerifiedNet
        || !matches!(
            snapshot.dex_comparison.quality,
            OnchainDexComparisonQuality::Fresh | OnchainDexComparisonQuality::EvidencePending
        )
    {
        return None;
    }
    snapshot
        .dex_comparison
        .routes
        .iter()
        .filter(|route| route.net_return_bps.is_some_and(f64::is_finite))
        .max_by(|left, right| {
            left.net_return_bps
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&right.net_return_bps.unwrap_or(f64::NEG_INFINITY))
        })
        .filter(|route| {
            route.net_return_bps.is_some_and(|net| {
                net > 0.0 && net >= snapshot.config.spread_alert.min_net_spread_bps.max(0.0)
            })
        })
}

fn dex_alert_payload(
    snapshot: &OnchainComparisonSnapshot,
    route: &OnchainDexRouteComparison,
) -> serde_json::Value {
    let net = route.net_return_bps.unwrap_or_default();
    let route_proven =
        route.route_identity == shared_types::OnchainDexRouteIdentity::ProvenDistinct;
    let blockers = route
        .problem
        .clone()
        .into_iter()
        .chain(std::iter::once(
            "两笔链上交易尚未生成原子执行计划，当前只监控，不自动提交".to_owned(),
        ))
        .collect::<Vec<_>>();
    serde_json::json!({
        "summary": format!(
            "{} {}/{} 同链 DEX 费后回报 {:.4}%，{}",
            snapshot.config.chain,
            snapshot.config.base_token,
            snapshot.config.quote_token,
            net / 100.0,
            if route_proven { "路由独立，保持监控" } else { "路由证据待核验，仅监控" },
        ),
        "marketType": "dex_cross",
        "profitabilityVerified": route.net_return_bps.is_some() && route_proven,
        "netCostCalculated": route.net_return_bps.is_some(),
        "routeIdentityVerified": route_proven,
        "executable": false,
        "monitorOnly": true,
        "chain": snapshot.config.chain,
        "base": {
            "symbol": snapshot.config.base_token,
            "address": snapshot.config.base_mint,
        },
        "quote": {
            "symbol": snapshot.config.quote_token,
            "address": snapshot.config.quote_mint,
        },
        "route": route,
        "executionBlockers": blockers,
        "snapshotObservedAtMs": snapshot.dex_comparison.observed_at_ms,
    })
}

fn dex_event_id(
    snapshot: &OnchainComparisonSnapshot,
    route: &OnchainDexRouteComparison,
    now_ms: i64,
) -> String {
    let cooldown = snapshot.config.spread_alert.cooldown_ms.max(1);
    let bucket = now_ms.div_euclid(cooldown);
    let canonical = format!(
        "{}\n{}\n{}\n{}\n{}\n{:?}\n{}",
        snapshot.config.chain,
        snapshot.config.base_mint,
        snapshot.config.quote_mint,
        route.buy_provider,
        route.sell_provider,
        route.direction,
        bucket,
    );
    let digest = common::signing::hmac_sha256_hex(DEX_EVENT_ID_KEY, canonical.as_bytes());
    format!("onchain-dex-cross-{}", &digest[..24])
}

fn alert_payload(
    snapshot: &OnchainComparisonSnapshot,
    row: &OnchainCexComparison,
) -> serde_json::Value {
    let raw_observation =
        snapshot.config.spread_alert.mode == OnchainSpreadAlertMode::RawObservation;
    let cex_base = onchain_cex_base_token(&snapshot.config.cex_symbol).unwrap_or("未知");
    let cex_quote = onchain_cex_quote_token(&snapshot.config.cex_symbol).unwrap_or("未知");
    let base_matches = cex_base.eq_ignore_ascii_case(snapshot.config.base_token.trim());
    let quotes_match = cex_quote.eq_ignore_ascii_case(snapshot.config.quote_token.trim());
    let identity_unresolved =
        !snapshot.config.base_identity_resolved || !snapshot.config.quote_identity_resolved;
    let pair_matches = onchain_cex_pair_matches(&snapshot.config);
    let quote_comparison_verified =
        pair_matches || (base_matches && snapshot.quote_conversion.is_some());
    let readiness = (!raw_observation)
        .then(|| {
            snapshot
                .execution_readiness
                .directions
                .iter()
                .find(|candidate| candidate.direction == row.direction)
        })
        .flatten();
    let blockers = if raw_observation && identity_unresolved {
        vec![
            "链上合约与精度已读取，但币种符号身份尚未核验；只能观察原始价格，不能据此执行"
                .to_owned(),
        ]
    } else if raw_observation && !base_matches {
        vec!["链上与 CEX 是不同 Base 资产，只能比较两个独立市场的原始价格，不能据此执行".to_owned()]
    } else if raw_observation && quotes_match {
        vec!["原始价格尚未扣除费用、滑点和 Gas，只能观察，不能据此执行".to_owned()]
    } else if raw_observation {
        vec!["原始价格尚未完成 Quote 汇率、费用和滑点换算，只能观察，不能据此执行".to_owned()]
    } else {
        readiness.map_or_else(
            || vec!["该监控项尚未生成链上钱包与 CEX 现货账户执行资格证据".to_owned()],
            |readiness| readiness.blockers.clone(),
        )
    };
    let build_ready = readiness.is_some_and(|readiness| readiness.build_ready);
    let submit_ready = readiness.is_some_and(|readiness| readiness.submit_ready);
    let summary = alert_summary(snapshot, row, raw_observation, submit_ready);
    serde_json::json!({
        "summary": summary,
        "alertMode": snapshot.config.spread_alert.mode,
        "profitabilityVerified": !raw_observation && quote_comparison_verified,
        "notice": if raw_observation {
            Some("原始价差只表示两边名义价格不同，不代表可实现的套利利润")
        } else if snapshot.quote_conversion.is_some() {
            Some("净差已包含 Quote 换算盘口与手续费；自动执行仍等待第三条换算腿接入")
        } else {
            None
        },
        "chain": snapshot.config.chain,
        "provider": snapshot.config.provider,
        "base": {
            "symbol": snapshot.config.base_token,
            "identityResolved": snapshot.config.base_identity_resolved,
            "address": snapshot.config.base_mint,
        },
        "quote": {
            "symbol": snapshot.config.quote_token,
            "identityResolved": snapshot.config.quote_identity_resolved,
            "address": snapshot.config.quote_mint,
        },
        "cex": {
            "venue": snapshot.config.cex_venue,
            "symbol": snapshot.config.cex_symbol,
            "source": snapshot.cex_source,
            "base": cex_base,
        },
        "quoteConversion": snapshot.quote_conversion,
        "usdValuation": snapshot.quote_usd_valuation,
        "direction": row.direction,
        "grossSpreadBps": row.gross_spread_bps,
        "netSpreadBps": (!raw_observation).then_some(row.net_spread_bps),
        "minNetSpreadBps": (!raw_observation)
            .then_some(snapshot.config.spread_alert.min_net_spread_bps),
        "rawSpreadBps": raw_observation.then_some(row.gross_spread_bps),
        "minRawSpreadBps": raw_observation
            .then_some(snapshot.config.spread_alert.min_raw_spread_bps),
        "onchainQuote": snapshot.config.quote_token,
        "cexQuote": cex_quote,
        "totalCostBps": (!raw_observation).then_some(row.total_cost_bps),
        "observableNotionalUsd": (!raw_observation).then_some(row.observable_notional_usd),
        "quoteObservedAtMs": snapshot.quote_observed_at_ms,
        "cexObservedAtMs": snapshot.cex_observed_at_ms,
        "snapshotObservedAtMs": snapshot.observed_at_ms,
        "quality": snapshot.quality,
        "readOnly": raw_observation || snapshot.read_only,
        "executable": !raw_observation && row.executable,
        "buildReady": build_ready,
        "submitReady": submit_ready,
        "executionBlockers": blockers,
        "executionReadiness": readiness,
    })
}

fn alert_summary(
    snapshot: &OnchainComparisonSnapshot,
    row: &OnchainCexComparison,
    raw_observation: bool,
    submit_ready: bool,
) -> String {
    let cex_base = onchain_cex_base_token(&snapshot.config.cex_symbol).unwrap_or("未知");
    let cex_quote = onchain_cex_quote_token(&snapshot.config.cex_symbol).unwrap_or("未知");
    let base_matches = cex_base.eq_ignore_ascii_case(snapshot.config.base_token.trim());
    let quotes_match = cex_quote.eq_ignore_ascii_case(snapshot.config.quote_token.trim());
    if raw_observation
        && (!snapshot.config.base_identity_resolved || !snapshot.config.quote_identity_resolved)
    {
        format!(
            "链上 {}/{} 对 {} {} 原始价差 {:.4}%，币种身份待核验，仅观察",
            snapshot.config.base_token,
            snapshot.config.quote_token,
            snapshot.config.cex_venue.to_ascii_uppercase(),
            snapshot.config.cex_symbol,
            row.gross_spread_bps / 100.0,
        )
    } else if raw_observation && !base_matches {
        format!(
            "链上 {} 与 {} {} 是不同 Base 资产，原始价格差 {:.4}%，仅观察",
            snapshot.config.base_token,
            snapshot.config.cex_venue.to_ascii_uppercase(),
            cex_base,
            row.gross_spread_bps / 100.0,
        )
    } else if raw_observation && quotes_match {
        format!(
            "{}/{} 对 {} {} 原始价差 {:.4}%，尚未扣除费用与滑点，仅观察",
            snapshot.config.base_token,
            snapshot.config.quote_token,
            snapshot.config.cex_venue.to_ascii_uppercase(),
            snapshot.config.cex_symbol,
            row.gross_spread_bps / 100.0,
        )
    } else if raw_observation {
        format!(
            "{}/{} 对 {} {} 原始价差 {:.4}%，未做 {}/{} 汇率换算，仅观察",
            snapshot.config.base_token,
            snapshot.config.quote_token,
            snapshot.config.cex_venue.to_ascii_uppercase(),
            snapshot.config.cex_symbol,
            row.gross_spread_bps / 100.0,
            cex_quote,
            snapshot.config.quote_token,
        )
    } else {
        format!(
            "{}/{} 费后价差 {:.4}%，执行{}",
            snapshot.config.base_token,
            snapshot.config.quote_token,
            row.net_spread_bps / 100.0,
            if submit_ready {
                "已就绪"
            } else {
                "被证据阻断"
            },
        )
    }
}

fn alert_candidate(snapshot: &OnchainComparisonSnapshot) -> Option<&OnchainCexComparison> {
    if !snapshot.config.spread_alert.enabled {
        return None;
    }
    match snapshot.config.spread_alert.mode {
        OnchainSpreadAlertMode::VerifiedNet
            if snapshot.quality == OnchainComparisonQuality::Fresh =>
        {
            snapshot
                .comparisons
                .iter()
                .filter(|row| row.net_spread_bps.is_finite())
                .max_by(|left, right| left.net_spread_bps.total_cmp(&right.net_spread_bps))
                .filter(|row| {
                    row.net_spread_bps > 0.0
                        && row.net_spread_bps
                            >= snapshot.config.spread_alert.min_net_spread_bps.max(0.0)
                })
        }
        OnchainSpreadAlertMode::RawObservation
            if raw_observation_sources_are_fresh(snapshot.quality) =>
        {
            snapshot
                .comparisons
                .iter()
                .filter(|row| row.gross_spread_bps.is_finite())
                .max_by(|left, right| left.gross_spread_bps.total_cmp(&right.gross_spread_bps))
                .filter(|row| {
                    row.gross_spread_bps >= snapshot.config.spread_alert.min_raw_spread_bps
                })
        }
        _ => None,
    }
}

const fn raw_observation_sources_are_fresh(quality: OnchainComparisonQuality) -> bool {
    matches!(
        quality,
        OnchainComparisonQuality::Fresh
            | OnchainComparisonQuality::LowLiquidity
            | OnchainComparisonQuality::NoNetProfit
            | OnchainComparisonQuality::RawCrossQuote
            | OnchainComparisonQuality::RawCustomPair
    )
}

fn event_id(
    snapshot: &OnchainComparisonSnapshot,
    row: &OnchainCexComparison,
    now_ms: i64,
) -> String {
    let cooldown = snapshot.config.spread_alert.cooldown_ms.max(1);
    let bucket = now_ms.div_euclid(cooldown);
    let canonical = format!(
        "{}\n{}\n{}\n{}\n{}\n{:?}\n{:?}\n{}",
        snapshot.config.chain,
        snapshot.config.base_mint,
        snapshot.config.quote_mint,
        snapshot.config.cex_venue,
        snapshot.config.cex_symbol,
        snapshot.config.spread_alert.mode,
        row.direction,
        bucket
    );
    let digest = common::signing::hmac_sha256_hex(EVENT_ID_KEY, canonical.as_bytes());
    format!("onchain-spread-{}", &digest[..24])
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::OnchainSpreadAlertConfig;

    fn profitable_snapshot() -> OnchainComparisonSnapshot {
        OnchainComparisonSnapshot {
            quality: OnchainComparisonQuality::Fresh,
            config: OnchainComparisonConfig {
                spread_alert: OnchainSpreadAlertConfig {
                    enabled: true,
                    mode: OnchainSpreadAlertMode::VerifiedNet,
                    min_net_spread_bps: 20.0,
                    min_raw_spread_bps: 20.0,
                    cooldown_ms: 60_000,
                },
                ..OnchainComparisonConfig::default()
            },
            comparisons: vec![OnchainCexComparison {
                direction: OnchainComparisonDirection::BuyOnchainSellCex,
                onchain_price: 100.0,
                cex_price: 101.0,
                gross_spread_bps: 100.0,
                cex_fee_bps: 10.0,
                quote_conversion_fee_bps: 0.0,
                slippage_bps: 5.0,
                gas_usd: 0.5,
                gas_bps: 5.0,
                total_cost_bps: 20.0,
                net_spread_bps: 80.0,
                observable_notional_usd: 1_000.0,
                executable: false,
            }],
            ..OnchainComparisonSnapshot::default()
        }
    }

    #[test]
    fn only_fresh_threshold_crossings_are_candidates() {
        let mut snapshot = profitable_snapshot();
        assert!(alert_candidate(&snapshot).is_some());
        snapshot.quality = OnchainComparisonQuality::Stale;
        assert!(alert_candidate(&snapshot).is_none());
        snapshot.quality = OnchainComparisonQuality::RawCrossQuote;
        assert!(alert_candidate(&snapshot).is_none());
        snapshot.quality = OnchainComparisonQuality::Fresh;
        snapshot.config.spread_alert.min_net_spread_bps = 81.0;
        assert!(alert_candidate(&snapshot).is_none());
    }

    #[test]
    fn ws_alert_only_fires_when_a_market_crosses_into_the_threshold() {
        let next = profitable_snapshot();
        let mut below_threshold = next.clone();
        below_threshold.config.spread_alert.min_net_spread_bps = 81.0;

        assert!(crossed_alert_threshold(&below_threshold, &next));

        let mut same_candidate = next.clone();
        same_candidate.comparisons[0].net_spread_bps = 90.0;
        assert!(!crossed_alert_threshold(&next, &same_candidate));

        let mut another_market = next.clone();
        another_market.config.cex_symbol = "SOL/USD".to_owned();
        assert!(crossed_alert_threshold(&next, &another_market));

        let mut stale = next.clone();
        stale.quality = OnchainComparisonQuality::Stale;
        assert!(!crossed_alert_threshold(&next, &stale));
    }

    #[test]
    fn batch_ws_alert_uses_the_previous_threshold_state() {
        let next = profitable_snapshot();
        let mut previous =
            super::super::projection::batch_item_snapshot("watch-1".to_owned(), &next);

        assert!(!batch_crossed_alert_threshold(Some(&previous), &next));

        previous.best_net_spread_bps = Some(10.0);
        assert!(batch_crossed_alert_threshold(Some(&previous), &next));
        assert!(batch_crossed_alert_threshold(None, &next));
    }

    #[test]
    fn raw_cross_quote_alerts_are_observations_without_profit_claims() {
        let mut snapshot = profitable_snapshot();
        snapshot.quality = OnchainComparisonQuality::RawCrossQuote;
        snapshot.config.cex_symbol = "SOL/USD".to_owned();
        snapshot.config.spread_alert.mode = OnchainSpreadAlertMode::RawObservation;
        snapshot.config.spread_alert.min_raw_spread_bps = 90.0;

        let Some(row) = alert_candidate(&snapshot) else {
            panic!("raw cross-quote threshold should emit an observation");
        };
        let payload = alert_payload(&snapshot, row);

        assert_eq!(payload["alertMode"], "raw_observation");
        assert_eq!(payload["profitabilityVerified"], false);
        assert!(payload["netSpreadBps"].is_null());
        assert!(payload["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("仅观察")));

        snapshot.quality = OnchainComparisonQuality::Fresh;
        snapshot.config.cex_symbol = "SOL/USDC".to_owned();
        let Some(row) = alert_candidate(&snapshot) else {
            panic!("same-quote raw threshold should emit an observation");
        };
        let payload = alert_payload(&snapshot, row);
        assert!(payload["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("尚未扣除费用与滑点")));

        snapshot.quality = OnchainComparisonQuality::NoNetProfit;
        assert!(
            alert_candidate(&snapshot).is_some(),
            "raw observation must not be blocked by the verified-net result"
        );
        snapshot.quality = OnchainComparisonQuality::LowLiquidity;
        assert!(
            alert_candidate(&snapshot).is_some(),
            "raw observation does not claim executable depth"
        );
        snapshot.quality = OnchainComparisonQuality::Stale;
        assert!(alert_candidate(&snapshot).is_none());
    }

    #[test]
    fn custom_base_alert_never_claims_profitability() {
        let mut snapshot = profitable_snapshot();
        snapshot.quality = OnchainComparisonQuality::RawCustomPair;
        snapshot.config.cex_symbol = "ETH/USDC".to_owned();
        snapshot.config.spread_alert.mode = OnchainSpreadAlertMode::RawObservation;

        let Some(row) = alert_candidate(&snapshot) else {
            panic!("custom-base raw threshold should emit an observation");
        };
        let payload = alert_payload(&snapshot, row);

        assert_eq!(payload["profitabilityVerified"], false);
        assert!(payload["netSpreadBps"].is_null());
        assert_eq!(payload["cex"]["base"], "ETH");
        assert!(payload["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("不同 Base 资产")));
    }

    #[test]
    fn provisional_identity_alert_says_identity_pending_instead_of_different_asset() {
        let mut snapshot = profitable_snapshot();
        snapshot.quality = OnchainComparisonQuality::RawCustomPair;
        snapshot.config.base_identity_resolved = false;
        snapshot.config.spread_alert.mode = OnchainSpreadAlertMode::RawObservation;

        let row = alert_candidate(&snapshot).expect("raw observation should remain monitorable");
        let payload = alert_payload(&snapshot, row);

        assert_eq!(payload["profitabilityVerified"], false);
        assert_eq!(payload["base"]["identityResolved"], false);
        assert!(payload["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("币种身份待核验")));
        assert!(!payload["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("不同 Base 资产")));
    }

    #[test]
    fn event_id_is_stable_within_cooldown_and_changes_afterward() {
        let snapshot = profitable_snapshot();
        let row = alert_candidate(&snapshot);
        assert!(row.is_some(), "profitable snapshot should emit an alert");
        let Some(row) = row else {
            return;
        };
        assert_eq!(
            event_id(&snapshot, row, 60_001),
            event_id(&snapshot, row, 119_999)
        );
        assert_ne!(
            event_id(&snapshot, row, 60_001),
            event_id(&snapshot, row, 120_000)
        );
        let mut another_market = snapshot.clone();
        another_market.config.cex_symbol = "SOL/USD".to_owned();
        assert_ne!(
            event_id(&snapshot, row, 60_001),
            event_id(&another_market, row, 60_001)
        );
    }

    #[test]
    fn payload_explains_why_a_profitable_row_cannot_submit() {
        let mut snapshot = profitable_snapshot();
        snapshot.execution_readiness.directions = vec![shared_types::OnchainDirectionReadiness {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            path: Default::default(),
            inventory: Vec::new(),
            cex_instrument: shared_types::OnchainCexInstrumentEvidence {
                venue: "kraken".to_owned(),
                requested_symbol: "SOL/USD".to_owned(),
                native_symbol: Some("SOL/USD".to_owned()),
                status: shared_types::OnchainCexInstrumentStatus::Ready,
                ready: true,
                source: "https://docs.kraken.com/api/docs/websocket-v2/instrument".to_owned(),
                observed_at_ms: Some(1),
                problem: None,
            },
            build_ready: false,
            submit_ready: false,
            blockers: vec!["KRAKEN 现货账户余额不足".to_owned()],
        }];
        let Some(row) = alert_candidate(&snapshot) else {
            panic!("profitable snapshot should emit an alert");
        };

        let payload = alert_payload(&snapshot, row);

        assert_eq!(payload["submitReady"], false);
        assert_eq!(payload["executionBlockers"][0], "KRAKEN 现货账户余额不足");
    }

    #[test]
    fn payload_carries_replenishment_network_evidence() {
        let mut snapshot = profitable_snapshot();
        snapshot.execution_readiness.directions = vec![shared_types::OnchainDirectionReadiness {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            path: shared_types::OnchainPathReadiness {
                replenishment: vec![shared_types::OnchainTransferEvidence {
                    direction: shared_types::OnchainTransferDirection::DepositToCex,
                    venue: "kraken".to_owned(),
                    asset: "SOL".to_owned(),
                    chain: "solana".to_owned(),
                    network: Some("SOL".to_owned()),
                    amount: 10.0,
                    amount_exact: Some("10".to_owned()),
                    status: shared_types::OnchainTransferStatus::Ready,
                    fee: Some(0.01),
                    fee_exact: Some("0.01".to_owned()),
                    minimum: Some(0.1),
                    minimum_exact: Some("0.1".to_owned()),
                    amount_step: None,
                    requires_tag: false,
                    contract_verified: true,
                    credit_confirmations: Some(12),
                    unlock_confirmations: Some(24),
                    network_status: Some("normal".to_owned()),
                    source: Some("exchange_transfer_networks".to_owned()),
                    observed_at_ms: Some(1),
                    problem: None,
                }],
                ..Default::default()
            },
            inventory: Vec::new(),
            cex_instrument: Default::default(),
            build_ready: false,
            submit_ready: false,
            blockers: vec!["需要先补充 KRAKEN 的 SOL 库存".to_owned()],
        }];
        let row = alert_candidate(&snapshot).expect("profitable snapshot should emit an alert");

        let payload = alert_payload(&snapshot, row);
        let evidence = &payload["executionReadiness"]["path"]["replenishment"][0];

        assert_eq!(evidence["status"], "ready");
        assert_eq!(evidence["creditConfirmations"], 12);
        assert_eq!(evidence["unlockConfirmations"], 24);
        assert_eq!(evidence["networkStatus"], "normal");
    }

    #[test]
    fn payload_never_reports_an_empty_blocker_list_when_readiness_is_missing() {
        let snapshot = profitable_snapshot();
        let Some(row) = alert_candidate(&snapshot) else {
            panic!("profitable snapshot should emit an alert");
        };

        let payload = alert_payload(&snapshot, row);

        assert_eq!(payload["submitReady"], false);
        assert_eq!(
            payload["executionBlockers"].as_array().map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn dex_alert_only_reports_profitable_non_duplicate_routes_as_monitoring() {
        let mut snapshot = profitable_snapshot();
        snapshot.config.dex_comparison = shared_types::OnchainDexComparisonConfig {
            enabled: true,
            peer_provider: "okx_dex_v6".to_owned(),
        };
        snapshot.dex_comparison = shared_types::OnchainDexComparisonSnapshot {
            primary_provider: "zeroex_swap_v2".to_owned(),
            peer_provider: "okx_dex_v6".to_owned(),
            quality: OnchainDexComparisonQuality::EvidencePending,
            routes: vec![OnchainDexRouteComparison {
                direction: shared_types::OnchainDexComparisonDirection::BuyPrimarySellPeer,
                buy_provider: "zeroex_swap_v2".to_owned(),
                sell_provider: "okx_dex_v6".to_owned(),
                input_quote_amount_raw: "100000000".to_owned(),
                acquired_base_amount_raw: "1000000000000000000".to_owned(),
                output_quote_amount_raw: "101000000".to_owned(),
                gross_return_bps: 100.0,
                execution_buffer_bps: 10.0,
                gas_usd: 0.2,
                gas_bps: Some(20.0),
                total_cost_bps: Some(30.0),
                net_return_bps: Some(70.0),
                buy_router: Some("0x sources".to_owned()),
                sell_router: Some("OKX router".to_owned()),
                route_identity: shared_types::OnchainDexRouteIdentity::Unknown,
                executable: false,
                problem: Some("底层池身份待核验".to_owned()),
                observed_at_ms: 1,
            }],
            observed_at_ms: 1,
            ..Default::default()
        };

        let route = dex_alert_candidate(&snapshot).expect("monitor-only edge should alert");
        let payload = dex_alert_payload(&snapshot, route);
        assert_eq!(payload["marketType"], "dex_cross");
        assert_eq!(payload["profitabilityVerified"], false);
        assert_eq!(payload["netCostCalculated"], true);
        assert_eq!(payload["monitorOnly"], true);
        assert_eq!(payload["executable"], false);

        snapshot.dex_comparison.quality = OnchainDexComparisonQuality::DuplicateRoute;
        assert!(dex_alert_candidate(&snapshot).is_none());
    }

    #[test]
    fn cross_chain_alert_requires_verified_net_and_remains_non_atomic() {
        let mut snapshot = profitable_snapshot();
        snapshot.config.cross_chain = shared_types::OnchainCrossChainConfig {
            enabled: true,
            peer_item_id: "watch-peer".to_owned(),
            provider: "lifi".to_owned(),
            stablecoin_risk_bps: 50,
        };
        snapshot.cross_chain = shared_types::OnchainCrossChainSnapshot {
            provider: "lifi".to_owned(),
            peer_item_id: "watch-peer".to_owned(),
            peer_chain: Some("arbitrum".to_owned()),
            quality: OnchainCrossChainQuality::Fresh,
            legs: vec![
                shared_types::OnchainCrossChainLeg {
                    position: 1,
                    kind: shared_types::OnchainCrossChainLegKind::SourceSwap,
                    provider: "zeroex_swap_v2".to_owned(),
                    from_chain: "base".to_owned(),
                    to_chain: "base".to_owned(),
                    from_asset: "USDC".to_owned(),
                    to_asset: "PUPS".to_owned(),
                    from_token: "source-usdc".to_owned(),
                    to_token: "source-pups".to_owned(),
                    input_amount_raw: "100000000".to_owned(),
                    expected_output_amount_raw: "1000000000000000000".to_owned(),
                    minimum_output_amount_raw: None,
                    input_decimals: 6,
                    output_decimals: 18,
                    fee_usd: None,
                    gas_usd: Some(0.1),
                    estimated_duration_seconds: None,
                    route_id: Some("route".to_owned()),
                    route_tools: Vec::new(),
                    official_docs_url: "https://docs.0x.org".to_owned(),
                    observed_at_ms: 1,
                };
                4
            ],
            initial_quote_amount_raw: Some("100000000".to_owned()),
            final_quote_amount_raw: Some("101000000".to_owned()),
            gross_return_bps: Some(100.0),
            execution_buffer_bps: Some(10.0),
            stablecoin_risk_bps: 50,
            bridge_fee_usd: Some(0.4),
            gas_usd: Some(0.3),
            quote_usd_valuation: Some(super::super::usd_valuation::fixture("USDC", 0.9, 1)),
            total_cost_bps: Some(30.0),
            net_return_bps: Some(70.0),
            estimated_duration_seconds: Some(20),
            inventory: Vec::new(),
            atomic: false,
            preview_ready: true,
            submit_ready: false,
            problem: Some("monitor only".to_owned()),
            quote_observed_at_ms: Some(1),
            quote_latency_ms: Some(1),
            observed_at_ms: 1,
        };

        assert!(cross_chain_alert_candidate(&snapshot, 1));
        let payload = cross_chain_alert_payload(&snapshot);
        assert_eq!(payload["marketType"], "cross_chain_closed_cycle");
        assert_eq!(payload["atomic"], false);
        assert_eq!(payload["executable"], false);
        assert_eq!(payload["monitorOnly"], true);
        assert_eq!(payload["usdValuation"]["usdBid"], 0.9);
        assert_eq!(payload["currencyRiskBps"], 50);
        assert!(!cross_chain_alert_candidate(
            &snapshot,
            2 + snapshot.config.max_age_ms
        ));
        let valuation = snapshot.cross_chain.quote_usd_valuation.take();
        assert!(!cross_chain_alert_candidate(&snapshot, 1));
        snapshot.cross_chain.quote_usd_valuation = valuation;

        snapshot.cross_chain.quality = OnchainCrossChainQuality::EvidencePending;
        assert!(!cross_chain_alert_candidate(&snapshot, 1));
    }
}
