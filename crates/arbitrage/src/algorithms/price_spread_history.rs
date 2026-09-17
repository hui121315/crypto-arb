//! Executable price-spread observations used to prove convergence before trading.

use crate::algorithms::{market, quote_conversion};
use crate::interfaces::MarketDataSnapshot;
use crate::models::{PriceSpreadConvergence, RawOpportunity};
use dashmap::DashMap;
use shared_types::{
    normalized_venue_name, MarketDataQuality, MarketDataRowEvidence, MarketDataSnapshotOperation,
    MarketDataSourceKind, SpotTick, StrategyKind, TickerInfo,
};
use std::collections::{HashMap, HashSet, VecDeque};

const MAX_TRACKED_PAIRS: usize = 512;
const CURRENT_PAIR_PRIORITY_BUDGET: usize = MAX_TRACKED_PAIRS * 3 / 4;
const MAX_SAMPLES_PER_PAIR: usize = 1_080;
const MAX_AGE_MS: i64 = 45 * 60 * 1_000;
const MIN_SAMPLE_INTERVAL_MS: i64 = 2_000;
const MAX_QUOTE_AGE_MS: i64 = 30_000;
const MAX_FUTURE_QUOTE_SKEW_MS: i64 = 5_000;
const MAX_LEG_QUOTE_SKEW_MS: u64 = 2_000;
const TRACKING_EDGE_CAP_BPS: f64 = 500.0;
const MIN_SAMPLE_COUNT: usize = 60;
const EPISODE_STEP_MS: i64 = 60 * 1_000;
const MIN_FOLLOWUP_MS: i64 = 60 * 1_000;
const MAX_HOLD_MS: i64 = 15 * 60 * 1_000;
const MAX_TERMINAL_STALENESS_MS: i64 = 30 * 1_000;
const MIN_COMPLETED_EPISODES: usize = 3;
const MIN_PROFITABLE_EPISODES: usize = 2;
const MIN_SPAN_MS: i64 = MAX_HOLD_MS + (MIN_COMPLETED_EPISODES as i64 - 1) * EPISODE_STEP_MS;
const MIN_SUCCESS_RATIO: f64 = 0.60;
const MIN_MEDIAN_NET_BPS: f64 = 5.0;
const MIN_LOWER_QUARTILE_NET_BPS: f64 = 0.0;
const FUNDING_EXIT_BUFFER_MS: i64 = 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct SpreadKey {
    symbol: String,
    long_venue: String,
    long_market_symbol: String,
    short_venue: String,
    short_market_symbol: String,
}

#[derive(Debug, Clone, Copy)]
struct SpreadPoint {
    occurred_at_ms: i64,
    long_bid: f64,
    long_ask: f64,
    short_bid: f64,
    short_ask: f64,
}

#[derive(Debug, Default)]
pub(crate) struct PriceSpreadHistoryCache {
    rows: DashMap<SpreadKey, VecDeque<SpreadPoint>>,
}

impl PriceSpreadHistoryCache {
    pub(crate) fn observe_candidates(
        &self,
        market: &MarketDataSnapshot,
        opportunities: &[RawOpportunity],
        now_ms: i64,
    ) {
        let perp_ws_keys = fresh_ws_keys(
            &market.perp_ticker_row_evidence,
            MarketDataSnapshotOperation::PerpTickers,
            now_ms,
        );
        let spot_ws_keys = fresh_ws_keys(
            &market.spot_tick_row_evidence,
            MarketDataSnapshotOperation::SpotTicks,
            now_ms,
        );
        let ticker_index = ticker_index(&market.perp_tickers, &perp_ws_keys, now_ms);
        let mut current = opportunities
            .iter()
            .filter(|row| row.extra.strategy_kind == Some(StrategyKind::PerpPriceSpread))
            .map(|row| {
                (
                    tracking_edge_priority(current_entry_edge_bps(row)),
                    spread_key(row),
                )
            })
            .collect::<Vec<_>>();
        current.sort_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
        });
        let current = current.into_iter().map(|(_, key)| key).collect::<Vec<_>>();
        let mut existing = self
            .rows
            .iter()
            .filter_map(|row| {
                let first = row.value().front()?;
                let last = row.value().back()?;
                let max_entry_edge_bps = row
                    .value()
                    .iter()
                    .map(|point| tracking_edge_priority(entry_edge_bps(point)))
                    .max_by(f64::total_cmp)
                    .unwrap_or(0.0);
                Some((
                    max_entry_edge_bps,
                    last.occurred_at_ms.saturating_sub(first.occurred_at_ms),
                    row.value().len(),
                    last.occurred_at_ms,
                    row.key().clone(),
                ))
            })
            .collect::<Vec<_>>();
        existing.sort_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| right.3.cmp(&left.3))
                .then_with(|| left.4.cmp(&right.4))
        });
        let mut seen = HashSet::with_capacity(MAX_TRACKED_PAIRS);
        let mut keys = Vec::with_capacity(MAX_TRACKED_PAIRS);
        append_distinct(
            &mut keys,
            &mut seen,
            current.iter().take(CURRENT_PAIR_PRIORITY_BUDGET).cloned(),
        );
        append_distinct(
            &mut keys,
            &mut seen,
            existing.into_iter().map(|(_, _, _, _, key)| key),
        );
        append_distinct(
            &mut keys,
            &mut seen,
            current.into_iter().skip(CURRENT_PAIR_PRIORITY_BUDGET),
        );
        let retained = keys.iter().cloned().collect::<HashSet<_>>();
        self.rows.retain(|key, _| retained.contains(key));

        for key in keys {
            let Some(point) = executable_point(
                &key,
                &ticker_index,
                &market.spot_ticks,
                &spot_ws_keys,
                now_ms,
            ) else {
                continue;
            };
            self.record(key, point);
        }
        self.prune_inactive(now_ms);
    }

    pub(crate) fn assess(
        &self,
        opportunity: &RawOpportunity,
        total_cost_bps: f64,
        now_ms: i64,
    ) -> PriceSpreadConvergence {
        let Some(points) = self.rows.get(&spread_key(opportunity)) else {
            return PriceSpreadConvergence::default();
        };
        let mut evidence =
            assess_points(&points, current_entry_edge_bps(opportunity), total_cost_bps);
        apply_funding_runway(&mut evidence, opportunity, now_ms);
        evidence
    }

    fn record(&self, key: SpreadKey, point: SpreadPoint) {
        let mut row = self.rows.entry(key).or_default();
        if row.back().is_some_and(|last| {
            point.occurred_at_ms.saturating_sub(last.occurred_at_ms) < MIN_SAMPLE_INTERVAL_MS
        }) {
            return;
        }
        row.push_back(point);
        prune_points(&mut row, point.occurred_at_ms);
    }

    fn prune_inactive(&self, now_ms: i64) {
        let oldest_allowed = now_ms.saturating_sub(MAX_AGE_MS);
        self.rows.retain(|_, points| {
            prune_points(points, now_ms);
            points
                .back()
                .is_some_and(|point| point.occurred_at_ms >= oldest_allowed)
        });
    }
}

fn append_distinct(
    keys: &mut Vec<SpreadKey>,
    seen: &mut HashSet<SpreadKey>,
    candidates: impl IntoIterator<Item = SpreadKey>,
) {
    for key in candidates {
        if keys.len() >= MAX_TRACKED_PAIRS {
            return;
        }
        if seen.insert(key.clone()) {
            keys.push(key);
        }
    }
}

fn assess_points(
    points: &VecDeque<SpreadPoint>,
    current_entry_bps: f64,
    total_cost_bps: f64,
) -> PriceSpreadConvergence {
    let sample_count = points.len();
    let span_ms = points
        .front()
        .zip(points.back())
        .map_or(0, |(first, last)| {
            last.occurred_at_ms.saturating_sub(first.occurred_at_ms)
        });
    let episodes = completed_episodes(points, total_cost_bps.max(0.0));
    let mut net_outcomes = episodes
        .iter()
        .map(|episode| episode.net_bps)
        .collect::<Vec<_>>();
    let mut gross_outcomes = episodes
        .iter()
        .map(|episode| episode.gross_bps)
        .collect::<Vec<_>>();
    let mut profitable_holds = episodes
        .iter()
        .filter(|episode| episode.net_bps > 0.0)
        .map(|episode| episode.hold_ms as f64)
        .collect::<Vec<_>>();
    net_outcomes.sort_by(f64::total_cmp);
    gross_outcomes.sort_by(f64::total_cmp);
    profitable_holds.sort_by(f64::total_cmp);

    let completed_episodes = episodes.len();
    let profitable_episodes = episodes
        .iter()
        .filter(|episode| episode.net_bps > 0.0)
        .count();
    let success_ratio = ratio(profitable_episodes, completed_episodes);
    let expected_gross_bps = percentile(&gross_outcomes, 0.50)
        .map(|gross| gross.max(0.0).min(current_entry_bps.max(0.0)))
        .unwrap_or(0.0);
    let projected_net_bps = expected_gross_bps - total_cost_bps.max(0.0);
    let median_net_bps = percentile(&net_outcomes, 0.50);
    let lower_quartile_net_bps = percentile(&net_outcomes, 0.25);
    let recommended_hold_ms = percentile(&profitable_holds, 0.50)
        .map(|value| value.round() as i64)
        .unwrap_or(MIN_FOLLOWUP_MS)
        .clamp(MIN_FOLLOWUP_MS, MAX_HOLD_MS);
    let convergence_ready = sample_count >= MIN_SAMPLE_COUNT
        && span_ms >= MIN_SPAN_MS
        && completed_episodes >= MIN_COMPLETED_EPISODES
        && profitable_episodes >= MIN_PROFITABLE_EPISODES
        && success_ratio >= MIN_SUCCESS_RATIO
        && median_net_bps.is_some_and(|value| value >= MIN_MEDIAN_NET_BPS)
        && lower_quartile_net_bps.is_some_and(|value| value >= MIN_LOWER_QUARTILE_NET_BPS)
        && projected_net_bps >= MIN_MEDIAN_NET_BPS;

    PriceSpreadConvergence {
        sample_count,
        span_ms,
        completed_episodes,
        profitable_episodes,
        success_ratio,
        expected_gross_bps,
        projected_net_bps,
        lower_quartile_net_bps,
        median_net_bps,
        recommended_hold_ms,
        convergence_ready,
        // `assess_points` proves only historical convergence. The public `assess`
        // path replaces this provisional value with the current funding runway.
        funding_runway_ready: true,
        funding_runway_ms: None,
        required_runway_ms: recommended_hold_ms.saturating_add(FUNDING_EXIT_BUFFER_MS),
        evidence_ready: convergence_ready,
    }
}

fn apply_funding_runway(
    evidence: &mut PriceSpreadConvergence,
    opportunity: &RawOpportunity,
    now_ms: i64,
) {
    evidence.required_runway_ms = evidence
        .recommended_hold_ms
        .max(MIN_FOLLOWUP_MS)
        .saturating_add(FUNDING_EXIT_BUFFER_MS);
    evidence.funding_runway_ms = earliest_funding_runway_ms(opportunity, now_ms);
    evidence.funding_runway_ready = evidence
        .funding_runway_ms
        .is_some_and(|runway| runway >= evidence.required_runway_ms);
    evidence.evidence_ready = evidence.convergence_ready && evidence.funding_runway_ready;
}

fn earliest_funding_runway_ms(opportunity: &RawOpportunity, now_ms: i64) -> Option<i64> {
    let long = opportunity.long_rate.next_funding_time;
    let short = opportunity.short_rate.next_funding_time;
    (long > now_ms && short > now_ms).then(|| long.min(short).saturating_sub(now_ms))
}

#[derive(Debug, Clone, Copy)]
struct CompletedEpisode {
    gross_bps: f64,
    net_bps: f64,
    hold_ms: i64,
}

fn completed_episodes(
    points: &VecDeque<SpreadPoint>,
    total_cost_bps: f64,
) -> Vec<CompletedEpisode> {
    let Some(latest) = points.back().map(|point| point.occurred_at_ms) else {
        return Vec::new();
    };
    let mut episodes = Vec::new();
    let mut next_start_ms = i64::MIN;
    for (index, start) in points.iter().enumerate() {
        if start.occurred_at_ms < next_start_ms
            || latest.saturating_sub(start.occurred_at_ms) < MIN_FOLLOWUP_MS
            || entry_edge_bps(start) <= total_cost_bps
        {
            continue;
        }
        let min_exit_ms = start.occurred_at_ms.saturating_add(MIN_FOLLOWUP_MS);
        let max_exit_ms = start.occurred_at_ms.saturating_add(MAX_HOLD_MS);
        let target_exit = points
            .iter()
            .skip(index + 1)
            .skip_while(|point| point.occurred_at_ms < min_exit_ms)
            .take_while(|point| point.occurred_at_ms <= max_exit_ms)
            .find(|point| paired_gross_bps(start, point) >= total_cost_bps + MIN_MEDIAN_NET_BPS);
        let exit = match target_exit {
            Some(point) => point,
            None if latest >= max_exit_ms => {
                let Some(point) = points
                    .iter()
                    .skip(index + 1)
                    .skip_while(|point| point.occurred_at_ms < min_exit_ms)
                    .take_while(|point| point.occurred_at_ms <= max_exit_ms)
                    .last()
                else {
                    continue;
                };
                if max_exit_ms.saturating_sub(point.occurred_at_ms) > MAX_TERMINAL_STALENESS_MS {
                    continue;
                }
                point
            }
            None => continue,
        };
        let gross_bps = paired_gross_bps(start, exit);
        episodes.push(CompletedEpisode {
            gross_bps,
            net_bps: gross_bps - total_cost_bps,
            hold_ms: exit.occurred_at_ms.saturating_sub(start.occurred_at_ms),
        });
        // A single convergence move must not be counted repeatedly by several
        // overlapping entry timestamps. Start the next episode only after the
        // prior exit plus one observation period.
        next_start_ms = exit.occurred_at_ms.saturating_add(EPISODE_STEP_MS);
    }
    episodes
}

type MarketKey = (String, String);

fn fresh_ws_keys(
    rows: &[MarketDataRowEvidence],
    operation: MarketDataSnapshotOperation,
    now_ms: i64,
) -> HashSet<MarketKey> {
    rows.iter()
        .filter(|row| row.operation == operation && fresh_ws_evidence(row, now_ms))
        .map(|row| market::venue_symbol_key(&row.venue, &row.symbol))
        .collect()
}

fn fresh_ws_evidence(row: &MarketDataRowEvidence, now_ms: i64) -> bool {
    let age_ms = now_ms.saturating_sub(row.health.observed_at_ms);
    row.health.quality == MarketDataQuality::Fresh
        && row.health.source == MarketDataSourceKind::WsPush
        && row.health.observed_at_ms > 0
        && (-MAX_FUTURE_QUOTE_SKEW_MS..=MAX_QUOTE_AGE_MS).contains(&age_ms)
}

fn ticker_index<'a>(
    tickers: &'a [TickerInfo],
    fresh_ws_keys: &HashSet<MarketKey>,
    now_ms: i64,
) -> HashMap<MarketKey, &'a TickerInfo> {
    tickers
        .iter()
        .filter_map(|ticker| {
            let key = market::venue_symbol_key(&ticker.exchange, &ticker.symbol);
            (valid_quote(ticker, now_ms) && fresh_ws_keys.contains(&key)).then_some((key, ticker))
        })
        .collect()
}

fn executable_point(
    key: &SpreadKey,
    tickers: &HashMap<MarketKey, &TickerInfo>,
    spot_ticks: &[SpotTick],
    spot_ws_keys: &HashSet<MarketKey>,
    now_ms: i64,
) -> Option<SpreadPoint> {
    let long = tickers.get(&(key.long_venue.clone(), key.long_market_symbol.clone()))?;
    let short = tickers.get(&(key.short_venue.clone(), key.short_market_symbol.clone()))?;
    let long_ask = market::ticker_ask(long)?;
    let long_bid = market::ticker_bid(long)?;
    let (short_bid, short_ask, conversion_at_ms) =
        normalized_short_prices(key, short, spot_ticks, spot_ws_keys, now_ms)?;
    let occurred_at_ms = synchronized_quote_timestamp(
        long.timestamp,
        short.timestamp,
        (conversion_at_ms > 0).then_some(conversion_at_ms),
    )?;
    Some(SpreadPoint {
        occurred_at_ms,
        long_bid,
        long_ask,
        short_bid,
        short_ask,
    })
}

fn synchronized_quote_timestamp(
    long_at_ms: i64,
    short_at_ms: i64,
    conversion_at_ms: Option<i64>,
) -> Option<i64> {
    if long_at_ms <= 0 || short_at_ms <= 0 || conversion_at_ms.is_some_and(|value| value <= 0) {
        return None;
    }
    let earliest = conversion_at_ms.map_or(long_at_ms.min(short_at_ms), |value| {
        long_at_ms.min(short_at_ms).min(value)
    });
    let latest = conversion_at_ms.map_or(long_at_ms.max(short_at_ms), |value| {
        long_at_ms.max(short_at_ms).max(value)
    });
    (earliest.abs_diff(latest) <= MAX_LEG_QUOTE_SKEW_MS).then_some(latest)
}

fn normalized_short_prices(
    key: &SpreadKey,
    short: &TickerInfo,
    spot_ticks: &[SpotTick],
    spot_ws_keys: &HashSet<MarketKey>,
    now_ms: i64,
) -> Option<(f64, f64, i64)> {
    let long_quote = market::canonical_perp_quote_symbol(&key.long_venue, &key.long_market_symbol)?;
    let short_quote =
        market::canonical_perp_quote_symbol(&key.short_venue, &key.short_market_symbol)?;
    let open =
        quote_conversion::find_on_venue(&key.short_venue, &short_quote, &long_quote, spot_ticks)?;
    let close =
        quote_conversion::find_on_venue(&key.short_venue, &long_quote, &short_quote, spot_ticks)?;
    if !fresh_conversion(open, spot_ws_keys, now_ms)
        || !fresh_conversion(close, spot_ws_keys, now_ms)
    {
        return None;
    }
    let conversion_at_ms = [open.market, close.market]
        .into_iter()
        .flatten()
        .map(SpotTick::best_timestamp_ms)
        .max()
        .unwrap_or(0);
    Some((
        market::ticker_bid(short)? * open.rate,
        market::ticker_ask(short)? / close.rate,
        conversion_at_ms,
    ))
}

fn fresh_conversion(
    conversion: quote_conversion::QuoteConversion<'_>,
    spot_ws_keys: &HashSet<MarketKey>,
    now_ms: i64,
) -> bool {
    conversion.market.is_none_or(|market| {
        let age_ms = now_ms.saturating_sub(market.best_timestamp_ms());
        let key = market::venue_symbol_key(&market.venue, &market.symbol);
        spot_ws_keys.contains(&key)
            && (-MAX_FUTURE_QUOTE_SKEW_MS..=MAX_QUOTE_AGE_MS).contains(&age_ms)
    })
}

fn entry_edge_bps(point: &SpreadPoint) -> f64 {
    (point.short_bid - point.long_ask) / point.long_ask * 10_000.0
}

fn paired_gross_bps(open: &SpreadPoint, close: &SpreadPoint) -> f64 {
    ((close.long_bid - open.long_ask) + (open.short_bid - close.short_ask)) / open.long_ask
        * 10_000.0
}

fn valid_quote(ticker: &TickerInfo, now_ms: i64) -> bool {
    let age_ms = now_ms.saturating_sub(ticker.timestamp);
    ticker.timestamp > 0
        && (-MAX_FUTURE_QUOTE_SKEW_MS..=MAX_QUOTE_AGE_MS).contains(&age_ms)
        && market::ticker_bid(ticker).is_some()
        && market::ticker_ask(ticker).is_some()
}

fn tracking_edge_priority(edge_bps: f64) -> f64 {
    if edge_bps.is_finite() {
        edge_bps.clamp(0.0, TRACKING_EDGE_CAP_BPS)
    } else {
        0.0
    }
}

fn spread_key(opportunity: &RawOpportunity) -> SpreadKey {
    SpreadKey {
        symbol: opportunity.symbol.to_ascii_uppercase(),
        long_venue: normalized_venue_name(&opportunity.long_exchange),
        long_market_symbol: opportunity
            .extra
            .long_market_symbol
            .as_deref()
            .unwrap_or(&opportunity.symbol)
            .to_ascii_uppercase(),
        short_venue: normalized_venue_name(&opportunity.short_exchange),
        short_market_symbol: opportunity
            .extra
            .short_market_symbol
            .as_deref()
            .unwrap_or(&opportunity.symbol)
            .to_ascii_uppercase(),
    }
}

fn current_entry_edge_bps(opportunity: &RawOpportunity) -> f64 {
    opportunity
        .extra
        .price_deviation
        .filter(|value| value.is_finite())
        .unwrap_or(opportunity.single_yield)
        .max(0.0)
        * 10_000.0
}

fn prune_points(points: &mut VecDeque<SpreadPoint>, now_ms: i64) {
    let oldest_allowed = now_ms.saturating_sub(MAX_AGE_MS);
    while points.front().is_some_and(|point| {
        point.occurred_at_ms < oldest_allowed || points.len() > MAX_SAMPLES_PER_PAIR
    }) {
        points.pop_front();
    }
}

fn percentile(sorted: &[f64], quantile: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let index = ((sorted.len() - 1) as f64 * quantile.clamp(0.0, 1.0)).round() as usize;
    sorted.get(index).copied()
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

pub(crate) fn convergence_blocker(evidence: PriceSpreadConvergence) -> Option<String> {
    if evidence.evidence_ready {
        return None;
    }
    if evidence.convergence_ready && !evidence.funding_runway_ready {
        return Some(format!(
            "永续价差当前 Funding 窗口不足：剩余 {:.1} 分钟，预计持有与退出缓冲需要 {:.1} 分钟；继续监控，等待结算后再评估",
            evidence.funding_runway_ms.unwrap_or(0).max(0) as f64 / 60_000.0,
            evidence.required_runway_ms.max(0) as f64 / 60_000.0,
        ));
    }
    Some(
        format!(
            "永续价差收敛证据不足：可执行报价样本 {}/{MIN_SAMPLE_COUNT}，跨度 {:.1}/{:.0} 分钟，历史闭环 {}/{MIN_COMPLETED_EPISODES}，盈利闭环 {}，预计扣费后 {:.1}bps；仅观察不执行",
            evidence.sample_count,
            evidence.span_ms.max(0) as f64 / 60_000.0,
            MIN_SPAN_MS as f64 / 60_000.0,
            evidence.completed_episodes,
            evidence.profitable_episodes,
            evidence.projected_net_bps,
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithms::market;
    use crate::models::RawOpportunityExtra;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use shared_types::{
        ArbitrageType, FundingRateData, MarketDataHealth, MarketDataRowEvidence,
        MarketDataSnapshotOperation, MarketDataSourceKind,
    };

    #[test]
    fn repeated_profitable_reversions_unlock_evidence() {
        let mut points = VecDeque::new();
        for index in 0..=240 {
            let within_minute = index % 12;
            let (entry, residual) = if within_minute == 0 {
                (90.0, 100.0)
            } else if within_minute >= 6 {
                (0.0, 10.0)
            } else {
                (0.0, 45.0)
            };
            points.push_back(point(index * 5_000, entry, residual));
        }

        let evidence = assess_points(&points, 90.0, 28.0);

        assert!(evidence.evidence_ready, "{evidence:?}");
        assert!(evidence.completed_episodes >= MIN_COMPLETED_EPISODES);
        assert!(evidence.projected_net_bps >= MIN_MEDIAN_NET_BPS);
        assert!(convergence_blocker(evidence).is_none());
    }

    #[test]
    fn mature_convergence_waits_when_funding_is_too_close() {
        let mut points = VecDeque::new();
        for index in 0..=240 {
            let within_minute = index % 12;
            let (entry, residual) = if within_minute == 0 {
                (90.0, 100.0)
            } else if within_minute >= 6 {
                (0.0, 10.0)
            } else {
                (0.0, 45.0)
            };
            points.push_back(point(index * 5_000, entry, residual));
        }
        let mut evidence = assess_points(&points, 90.0, 28.0);
        let now_ms = 10_000;
        let mut opportunity = raw();
        opportunity.long_rate.next_funding_time =
            now_ms + evidence.recommended_hold_ms + FUNDING_EXIT_BUFFER_MS - 1;
        opportunity.short_rate.next_funding_time = now_ms + 8 * 3_600_000;

        apply_funding_runway(&mut evidence, &opportunity, now_ms);

        assert!(evidence.convergence_ready);
        assert!(!evidence.funding_runway_ready);
        assert!(!evidence.evidence_ready);
        assert!(convergence_blocker(evidence)
            .as_deref()
            .is_some_and(|detail| detail.contains("Funding 窗口不足")));
    }

    #[test]
    fn persistent_residual_stays_fail_closed() {
        let points = (0..=240)
            .map(|index| point(index * 5_000, 60.0, 70.0))
            .collect::<VecDeque<_>>();

        let evidence = assess_points(&points, 60.0, 28.0);

        assert!(!evidence.evidence_ready);
        assert!(evidence.projected_net_bps < 0.0);
        assert!(convergence_blocker(evidence).is_some());
    }

    #[test]
    fn episode_uses_first_causal_target_instead_of_future_best_quote() {
        let points = VecDeque::from([
            point(0, 90.0, 100.0),
            point(MIN_FOLLOWUP_MS, 0.0, 55.0),
            point(MIN_FOLLOWUP_MS + 5_000, 0.0, 0.0),
        ]);

        let episodes = completed_episodes(&points, 28.0);

        assert_eq!(episodes.len(), 1);
        assert!((episodes[0].gross_bps - 35.0).abs() < 1e-9);
        assert!((episodes[0].net_bps - 7.0).abs() < 1e-9);
        assert_eq!(episodes[0].hold_ms, MIN_FOLLOWUP_MS);
    }

    #[test]
    fn mature_episode_records_horizon_loss_when_target_never_trades() {
        let points = VecDeque::from([
            point(0, 90.0, 100.0),
            point(MAX_HOLD_MS - 5_000, 0.0, 80.0),
            point(MAX_HOLD_MS, 0.0, 80.0),
        ]);

        let episodes = completed_episodes(&points, 28.0);

        assert_eq!(episodes.len(), 1);
        assert!((episodes[0].gross_bps - 10.0).abs() < 1e-9);
        assert!((episodes[0].net_bps + 18.0).abs() < 1e-9);
        assert_eq!(episodes[0].hold_ms, MAX_HOLD_MS);
    }

    #[test]
    fn one_late_reversion_cannot_multiply_overlapping_episodes() {
        let mut points = (0..15)
            .map(|minute| point(minute * EPISODE_STEP_MS, 90.0, 100.0))
            .collect::<VecDeque<_>>();
        points.push_back(point(MAX_HOLD_MS, 0.0, 10.0));

        let episodes = completed_episodes(&points, 28.0);

        assert_eq!(episodes.len(), 1, "{episodes:?}");
        assert_eq!(episodes[0].hold_ms, MAX_HOLD_MS);
    }

    #[test]
    fn paired_cash_flow_uses_all_four_executable_prices() {
        let open = quote_point(0, 99.0, 100.0, 102.0, 103.0);
        let close = quote_point(MIN_FOLLOWUP_MS, 110.0, 111.0, 110.0, 111.0);

        let gross_bps = paired_gross_bps(&open, &close);

        assert!((gross_bps - 100.0).abs() < 1e-9);
    }

    #[test]
    fn candidate_pair_keeps_recording_after_entry_edge_disappears() {
        let cache = PriceSpreadHistoryCache::default();
        let opportunity = raw();
        let first = snapshot(
            vec![
                ticker_at("a", 99.0, 100.0, 10_000),
                ticker_at("b", 101.0, 102.0, 10_000),
            ],
            Vec::new(),
        );
        cache.observe_candidates(&first, std::slice::from_ref(&opportunity), 10_000);
        let second = snapshot(
            vec![
                ticker_at("a", 100.0, 101.0, 15_000),
                ticker_at("b", 100.5, 101.5, 15_000),
            ],
            Vec::new(),
        );
        cache.observe_candidates(&second, &[], 15_000);

        let points = cache
            .rows
            .get(&spread_key(&opportunity))
            .expect("tracked pair");
        assert_eq!(points.len(), 2);
        assert!(entry_edge_bps(&points[1]) < 0.0);
    }

    #[test]
    fn unchanged_quotes_do_not_accumulate_scan_cadence_samples() {
        let cache = PriceSpreadHistoryCache::default();
        let opportunity = raw();
        let market = snapshot(
            vec![
                ticker_at("a", 99.0, 100.0, 10_000),
                ticker_at("b", 101.0, 102.0, 10_000),
            ],
            Vec::new(),
        );

        for scan_at_ms in [10_000, 12_000, 14_000] {
            cache.observe_candidates(&market, std::slice::from_ref(&opportunity), scan_at_ms);
        }

        let points = cache
            .rows
            .get(&spread_key(&opportunity))
            .expect("tracked pair");
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].occurred_at_ms, 10_000);
    }

    #[test]
    fn recent_pair_survives_when_current_candidates_exceed_capacity() {
        let cache = PriceSpreadHistoryCache::default();
        let prior = raw();
        cache.rows.insert(
            spread_key(&prior),
            VecDeque::from([point(10_000, 90.0, 100.0)]),
        );
        let current = (0..MAX_TRACKED_PAIRS)
            .map(|index| raw_for_symbol(&format!("ASSET{index}")))
            .collect::<Vec<_>>();

        cache.observe_candidates(&MarketDataSnapshot::default(), &current, 10_000);

        assert!(cache.rows.contains_key(&spread_key(&prior)));
    }

    #[test]
    fn stale_quote_does_not_extend_convergence_history() {
        let cache = PriceSpreadHistoryCache::default();
        let opportunity = raw();
        let now_ms = 100_000;
        let mut stale = ticker("a", 99.0, 100.0);
        stale.timestamp = now_ms - MAX_QUOTE_AGE_MS - 1;
        let mut fresh = ticker("b", 101.0, 102.0);
        fresh.timestamp = now_ms;

        let market = snapshot(vec![stale, fresh], Vec::new());
        cache.observe_candidates(&market, std::slice::from_ref(&opportunity), now_ms);

        assert!(!cache.rows.contains_key(&spread_key(&opportunity)));
    }

    #[test]
    fn time_skewed_legs_do_not_extend_convergence_history() {
        let cache = PriceSpreadHistoryCache::default();
        let opportunity = raw();
        let now_ms = 100_000;
        let long = ticker_at("a", 99.0, 100.0, now_ms);
        let short = ticker_at(
            "b",
            101.0,
            102.0,
            now_ms - i64::try_from(MAX_LEG_QUOTE_SKEW_MS).unwrap_or(i64::MAX) - 1,
        );

        let market = snapshot(vec![long, short], Vec::new());
        cache.observe_candidates(&market, std::slice::from_ref(&opportunity), now_ms);

        assert!(!cache.rows.contains_key(&spread_key(&opportunity)));
    }

    #[test]
    fn rest_baseline_leg_does_not_extend_convergence_history() {
        let cache = PriceSpreadHistoryCache::default();
        let opportunity = raw();
        let now_ms = 100_000;
        let mut market = snapshot(
            vec![
                ticker_at("a", 99.0, 100.0, now_ms),
                ticker_at("b", 101.0, 102.0, now_ms),
            ],
            Vec::new(),
        );
        market.perp_ticker_row_evidence[1].health.source = MarketDataSourceKind::RestBaseline;

        cache.observe_candidates(&market, std::slice::from_ref(&opportunity), now_ms);

        assert!(!cache.rows.contains_key(&spread_key(&opportunity)));
    }

    #[test]
    fn cross_quote_history_uses_both_sides_of_the_fx_book() {
        let cache = PriceSpreadHistoryCache::default();
        let mut opportunity = raw();
        opportunity.long_exchange = "hyperliquid".into();
        opportunity.short_exchange = "binance".into();
        opportunity.extra.long_market_symbol = Some("BTC".into());
        opportunity.extra.short_market_symbol = Some("BTCUSDT".into());
        let now_ms = 10_000;
        let mut long = ticker("hyperliquid", 99.0, 100.0);
        long.symbol = "BTC".into();
        long.timestamp = now_ms;
        let mut short = ticker("binance", 102.0, 103.0);
        short.timestamp = now_ms;
        let fx = spot_tick("binance", "USDT/USDC", 0.999, 1.001, now_ms);

        let market = snapshot(vec![long, short], vec![fx]);
        cache.observe_candidates(&market, std::slice::from_ref(&opportunity), now_ms);

        let points = cache
            .rows
            .get(&spread_key(&opportunity))
            .expect("cross-quote point");
        assert!((points[0].short_bid - 102.0 * 0.999).abs() < 1e-9);
        assert!((points[0].short_ask - 103.0 * 1.001).abs() < 1e-9);
    }

    #[test]
    fn high_edge_history_survives_capacity_pressure() {
        let cache = PriceSpreadHistoryCache::default();
        let target = raw_for_symbol("TARGET");
        cache.rows.insert(
            spread_key(&target),
            VecDeque::from([point(10_000, 400.0, 410.0)]),
        );
        for index in 0..MAX_TRACKED_PAIRS {
            let row = raw_for_symbol(&format!("OLD{index}"));
            cache.rows.insert(
                spread_key(&row),
                VecDeque::from([point(10_000, 20.0, 30.0)]),
            );
        }
        let current = (0..MAX_TRACKED_PAIRS)
            .map(|index| raw_for_symbol(&format!("NEW{index}")))
            .collect::<Vec<_>>();

        cache.observe_candidates(&MarketDataSnapshot::default(), &current, 10_000);

        assert!(cache.rows.contains_key(&spread_key(&target)));
        assert!(cache.rows.len() <= MAX_TRACKED_PAIRS);
    }

    fn raw() -> RawOpportunity {
        raw_for_symbol("BTC")
    }

    fn raw_for_symbol(symbol: &str) -> RawOpportunity {
        RawOpportunity {
            symbol: symbol.into(),
            arb_type: ArbitrageType::CrossExchange,
            long_exchange: "a".into(),
            short_exchange: "b".into(),
            long_rate: rate("a", symbol),
            short_rate: rate("b", symbol),
            spread_8h: 0.01,
            single_yield: 0.01,
            extra: RawOpportunityExtra {
                strategy_kind: Some(StrategyKind::PerpPriceSpread),
                price_deviation: Some(0.01),
                long_market_symbol: Some(format!("{symbol}USDT")),
                short_market_symbol: Some(format!("{symbol}USDT")),
                ..Default::default()
            },
        }
    }

    fn rate(exchange: &str, symbol: &str) -> FundingRateData {
        market::zero_rate(symbol, exchange, 1_000_000.0, 1)
    }

    fn ticker(exchange: &str, bid: f64, ask: f64) -> TickerInfo {
        ticker_at(exchange, bid, ask, 1)
    }

    fn ticker_at(exchange: &str, bid: f64, ask: f64, timestamp: i64) -> TickerInfo {
        TickerInfo {
            symbol: "BTCUSDT".into(),
            exchange: exchange.into(),
            bid,
            ask,
            last: (bid + ask) / 2.0,
            volume_24h: 1_000_000.0,
            timestamp,
        }
    }

    fn spot_tick(venue: &str, symbol: &str, bid: f64, ask: f64, timestamp: i64) -> SpotTick {
        SpotTick {
            venue: venue.into(),
            symbol: symbol.into(),
            bid: Decimal::from_f64_retain(bid).unwrap_or(dec!(0)),
            ask: Decimal::from_f64_retain(ask).unwrap_or(dec!(0)),
            last: Decimal::from_f64_retain(ask).unwrap_or(dec!(0)),
            bid_size: Some(dec!(1000000)),
            ask_size: Some(dec!(1000000)),
            volume_24h: dec!(1000000),
            exchange_ts_ms: Some(timestamp),
            received_at_ms: timestamp,
        }
    }

    fn snapshot(tickers: Vec<TickerInfo>, spot_ticks: Vec<SpotTick>) -> MarketDataSnapshot {
        let perp_ticker_row_evidence = tickers
            .iter()
            .map(|row| {
                ws_evidence(
                    &row.exchange,
                    &row.symbol,
                    MarketDataSnapshotOperation::PerpTickers,
                    row.timestamp,
                )
            })
            .collect();
        let spot_tick_row_evidence = spot_ticks
            .iter()
            .map(|row| {
                ws_evidence(
                    &row.venue,
                    &row.symbol,
                    MarketDataSnapshotOperation::SpotTicks,
                    row.best_timestamp_ms(),
                )
            })
            .collect();
        MarketDataSnapshot {
            perp_tickers: tickers.into(),
            perp_ticker_row_evidence,
            spot_ticks: spot_ticks.into(),
            spot_tick_row_evidence,
            ..Default::default()
        }
    }

    fn ws_evidence(
        venue: &str,
        symbol: &str,
        operation: MarketDataSnapshotOperation,
        observed_at_ms: i64,
    ) -> MarketDataRowEvidence {
        MarketDataRowEvidence {
            venue: venue.into(),
            symbol: symbol.into(),
            operation,
            health: MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: MarketDataSourceKind::WsPush,
                freshness_ms: Some(0),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms,
                coverage: None,
                problem: None,
            },
        }
    }

    fn point(occurred_at_ms: i64, entry_edge_bps: f64, exit_residual_bps: f64) -> SpreadPoint {
        debug_assert!(exit_residual_bps >= entry_edge_bps);
        let reference = 100.0;
        SpreadPoint {
            occurred_at_ms,
            long_bid: reference,
            long_ask: reference,
            short_bid: reference * (1.0 + entry_edge_bps / 10_000.0),
            short_ask: reference * (1.0 + exit_residual_bps / 10_000.0),
        }
    }

    fn quote_point(
        occurred_at_ms: i64,
        long_bid: f64,
        long_ask: f64,
        short_bid: f64,
        short_ask: f64,
    ) -> SpreadPoint {
        SpreadPoint {
            occurred_at_ms,
            long_bid,
            long_ask,
            short_bid,
            short_ask,
        }
    }
}
