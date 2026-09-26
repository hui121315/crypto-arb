use crate::state::load_state::LoadState;
use shared_types::gate_crossex::GATE_CROSSEX_MARKET_MAX_AGE_MS;
use shared_types::{ApiProblem, GateCrossExMode, GateCrossExModeSnapshot};

#[derive(Clone, Copy)]
pub(super) struct SnapshotClock {
    server_at_ms: i64,
    received: (i64, i64),
}

fn elapsed(now: (i64, i64), then: (i64, i64)) -> i64 {
    now.0
        .saturating_sub(then.0)
        .max(now.1.saturating_sub(then.1))
        .max(0)
}

impl SnapshotClock {
    pub(super) fn advance(
        previous: Option<Self>,
        observed_at_ms: i64,
        requested: (i64, i64),
        received: (i64, i64),
    ) -> Option<Self> {
        if observed_at_ms <= 0 {
            return previous;
        }
        // Same/late snapshots and configuration acknowledgements cannot renew quote age.
        // Include request time conservatively, without comparing server and browser epochs.
        Some(Self {
            server_at_ms: observed_at_ms
                .saturating_add(elapsed(received, requested))
                .max(previous.map_or(0, |old| old.now(received))),
            received,
        })
    }

    fn now(self, clock: (i64, i64)) -> i64 {
        self.server_at_ms
            .saturating_add(elapsed(clock, self.received))
    }
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct QuoteFreshness {
    server_now_ms: Option<i64>,
    snapshot_current: bool,
    pub live_count: usize,
    pub candidate_count: usize,
    pub problem: Option<ApiProblem>,
}

impl QuoteFreshness {
    pub(super) fn new(
        state: &LoadState<GateCrossExModeSnapshot>,
        timing: Option<SnapshotClock>,
        clock: (i64, i64),
    ) -> Self {
        let mut result = Self {
            server_now_ms: timing.map(|timing| timing.now(clock)),
            ..Self::default()
        };
        let LoadState::Ready(snapshot) = state else {
            return result;
        };
        if snapshot.config.mode == GateCrossExMode::Disabled {
            return result;
        }
        result.snapshot_current = result
            .age(snapshot.observed_at_ms)
            .is_some_and(|age| age <= GATE_CROSSEX_MARKET_MAX_AGE_MS);
        result.live_count = snapshot
            .routes
            .iter()
            .filter(|row| result.current(row.observed_at_ms))
            .count();
        result.candidate_count = snapshot
            .candidates
            .iter()
            .filter(|row| result.current(row.synchronized_at_ms))
            .count();
        if !result.snapshot_current
            || result.live_count < snapshot.routes.len()
            || result.candidate_count < snapshot.candidates.len()
        {
            let unknown = result.age(snapshot.observed_at_ms).is_none()
                || snapshot.routes.iter().any(|row| row.observed_at_ms <= 0)
                || snapshot
                    .candidates
                    .iter()
                    .any(|row| row.synchronized_at_ms <= 0);
            result.problem = Some(
                ApiProblem::new(
                    if unknown {
                        "GATE_CROSSEX_QUOTE_AGE_UNKNOWN"
                    } else {
                        "GATE_CROSSEX_QUOTES_STALE"
                    },
                    if unknown {
                        "行情时效待确认，保留上次价格供参考"
                    } else {
                        "报价超过 30 秒，旧价格仅供参考，等待更新或刷新状态"
                    },
                )
                .with_source("frontend.gate_crossex"),
            );
        }
        result
    }

    pub(super) fn age(&self, observed: i64) -> Option<i64> {
        (observed > 0).then_some(())?;
        self.server_now_ms
            .map(|now| now.saturating_sub(observed).max(0))
    }

    pub(super) fn current(&self, observed: i64) -> bool {
        self.snapshot_current
            && self
                .age(observed)
                .is_some_and(|age| age <= GATE_CROSSEX_MARKET_MAX_AGE_MS)
    }

    pub(super) fn label(&self) -> Option<&'static str> {
        self.problem.as_ref().map(|problem| {
            if problem.code == "GATE_CROSSEX_QUOTE_AGE_UNKNOWN" {
                "时效待确认"
            } else if self.live_count > 0 {
                "部分报价已过期"
            } else {
                "报价已过期"
            }
        })
    }
}
