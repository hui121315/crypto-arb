use shared_types::{
    OnchainComparisonQuality as Quality, OnchainComparisonSnapshot,
    OnchainCrossChainQuality as CrossQuality, OnchainDexComparisonQuality as DexQuality,
};

#[derive(Clone, Copy)]
pub(super) struct MarketClock {
    server_at: i64,
    received: (i64, i64),
}

fn elapsed(now: (i64, i64), then: (i64, i64)) -> i64 {
    now.0
        .saturating_sub(then.0)
        .max(now.1.saturating_sub(then.1))
        .max(0)
}

impl MarketClock {
    pub(super) fn advance(
        previous: Option<Self>,
        observed: i64,
        requested: (i64, i64),
        received: (i64, i64),
    ) -> Option<Self> {
        if observed <= 0 {
            return previous;
        }
        // Server and browser epochs may differ. Old frames and late HTTP receipts never rewind time.
        Some(Self {
            server_at: observed
                .saturating_add(elapsed(received, requested))
                .max(previous.map_or(0, |old| old.now(received))),
            received,
        })
    }

    pub(super) fn now(self, clock: (i64, i64)) -> i64 {
        self.server_at.saturating_add(elapsed(clock, self.received))
    }
}

fn age(
    now: Option<i64>,
    snapshot_at: i64,
    observed: Option<i64>,
    reported: Option<i64>,
) -> Option<i64> {
    let observed = observed.filter(|at| *at > 0 && *at <= snapshot_at)?;
    if reported.is_some_and(|age| age < 0) {
        return None;
    }
    Some(
        snapshot_at
            .saturating_sub(observed)
            .max(reported.unwrap_or(0))
            .saturating_add(now?.saturating_sub(snapshot_at).max(0)),
    )
}

fn current(now: Option<i64>, observed: Option<i64>, limit: i64) -> bool {
    observed
        .filter(|at| *at > 0)
        .zip(now)
        .is_some_and(|(at, now)| at <= now && now.saturating_sub(at) <= limit)
}

fn live_quality(quality: Quality) -> bool {
    matches!(
        quality,
        Quality::Fresh
            | Quality::NoNetProfit
            | Quality::LowLiquidity
            | Quality::RawCrossQuote
            | Quality::RawCustomPair
    )
}

pub(super) fn project(snapshot: &mut OnchainComparisonSnapshot, now: Option<i64>) {
    let at = snapshot.observed_at_ms;
    let limit = snapshot.config.max_age_ms;
    snapshot.onchain_freshness_ms = age(
        now,
        at,
        snapshot.quote_observed_at_ms,
        snapshot.onchain_freshness_ms,
    );
    snapshot.cex_freshness_ms = age(
        now,
        at,
        snapshot.cex_observed_at_ms,
        snapshot.cex_freshness_ms,
    );
    let problem = if snapshot.onchain_freshness_ms.is_none() || snapshot.cex_freshness_ms.is_none()
    {
        Some("报价时效待确认，保留上次价格供参考")
    } else if snapshot.onchain_freshness_ms.is_some_and(|age| age > limit)
        || snapshot.cex_freshness_ms.is_some_and(|age| age > limit)
    {
        Some("链上 或 交易所 报价已过期，等待新报价后再构建")
    } else if snapshot.quote_conversion.as_ref().is_some_and(|quote| {
        age(
            now,
            at,
            Some(quote.observed_at_ms),
            Some(quote.freshness_ms),
        )
        .is_none_or(|age| age > limit)
    }) {
        Some("换汇报价已过期，不能沿用旧汇率判断净收益")
    } else if snapshot
        .quote_usd_valuation
        .as_ref()
        .is_some_and(|quote| !current(now, Some(quote.observed_at_ms), limit))
    {
        Some("美元估值报价已过期，净收益等待复核")
    } else {
        None
    };
    if snapshot.config.enabled && live_quality(snapshot.quality) {
        if let Some(problem) = problem {
            snapshot.quality = Quality::Stale;
            snapshot.degradation_reasons.insert(0, problem.to_owned());
            for row in &mut snapshot.comparisons {
                row.executable = false;
            }
            for row in &mut snapshot.execution_readiness.directions {
                row.build_ready = false;
                row.submit_ready = false;
            }
        }
    }
    let dex = &mut snapshot.dex_comparison;
    if snapshot.config.enabled
        && snapshot.config.dex_comparison.enabled
        && matches!(
            dex.quality,
            DexQuality::Fresh | DexQuality::NoNetProfit | DexQuality::EvidencePending
        )
        && (!current(now, dex.quote_observed_at_ms, limit)
            || dex
                .routes
                .iter()
                .any(|route| !current(now, Some(route.observed_at_ms), limit)))
    {
        dex.quality = DexQuality::Stale;
        dex.problem = Some("链上 双边报价已过期，旧价格仅供参考".into());
        for route in &mut dex.routes {
            route.executable = false;
        }
    }
    let cross = &mut snapshot.cross_chain;
    let cross_limit = snapshot
        .batch
        .items
        .iter()
        .find(|item| item.item_id == cross.peer_item_id)
        .map_or(limit, |peer| limit.min(peer.config.max_age_ms));
    if snapshot.config.enabled
        && snapshot.config.cross_chain.enabled
        && matches!(
            cross.quality,
            CrossQuality::Fresh | CrossQuality::NoNetProfit | CrossQuality::EvidencePending
        )
        && (!current(now, cross.quote_observed_at_ms, cross_limit)
            || cross
                .quote_usd_valuation
                .as_ref()
                .is_some_and(|quote| !current(now, Some(quote.observed_at_ms), cross_limit)))
    {
        cross.quality = CrossQuality::Stale;
        cross.preview_ready = false;
        cross.submit_ready = false;
        cross.problem = Some("跨链报价已过期，等待重新询价".into());
    }
    for item in &mut snapshot.batch.items {
        let limit = item.config.max_age_ms;
        item.onchain_freshness_ms = age(
            now,
            item.observed_at_ms,
            item.quote_observed_at_ms,
            item.onchain_freshness_ms,
        );
        item.cex_freshness_ms = age(
            now,
            item.observed_at_ms,
            item.cex_observed_at_ms,
            item.cex_freshness_ms,
        );
        if !item.config.enabled {
            continue;
        }
        let stale = item.onchain_freshness_ms.is_none_or(|age| age > limit)
            || item.cex_freshness_ms.is_none_or(|age| age > limit)
            || item.quote_conversion.as_ref().is_some_and(|quote| {
                age(
                    now,
                    item.observed_at_ms,
                    Some(quote.observed_at_ms),
                    Some(quote.freshness_ms),
                )
                .is_none_or(|age| age > limit)
            })
            || item
                .quote_usd_valuation
                .as_ref()
                .is_some_and(|quote| !current(now, Some(quote.observed_at_ms), limit));
        if live_quality(item.quality) && stale {
            item.quality = Quality::Stale;
        }
        // Batch summaries omit independent route timestamps; their own observation still bounds age.
        if !current(now, Some(item.observed_at_ms), limit) {
            if item.config.dex_comparison.enabled
                && matches!(
                    item.dex_quality,
                    DexQuality::Fresh | DexQuality::NoNetProfit | DexQuality::EvidencePending
                )
            {
                item.dex_quality = DexQuality::Stale;
            }
            if item.config.cross_chain.enabled
                && matches!(
                    item.cross_chain_quality,
                    CrossQuality::Fresh | CrossQuality::NoNetProfit | CrossQuality::EvidencePending
                )
            {
                item.cross_chain_quality = CrossQuality::Stale;
            }
        }
    }
}
