use crate::panels::shared::confirmation::ConfirmedAt;
use leptos::prelude::*;
use shared_types::{ApiProblem, TradingStatusResponse};
use std::time::Duration;

use super::{
    load_state::LoadState,
    polling::{now_ms, polling_allowed, retry_deadline_ms},
    read_freshness::ReadFreshness,
    read_scope::{bounded_read, ReadScope},
};

#[derive(Clone, Copy)]
pub(crate) struct TradingStatusState {
    pub(crate) state: RwSignal<LoadState<TradingStatusResponse>>,
    revision: RwSignal<u64>,
    retry_until: RwSignal<Option<u64>>,
    freshness: ReadFreshness,
}

impl TradingStatusState {
    pub(crate) fn invalidate(self) -> u64 {
        self.revision
            .update(|revision| *revision = revision.wrapping_add(1));
        self.retry_until.set(None);
        self.freshness.reset();
        self.state.set(LoadState::Loading);
        self.revision.get_untracked()
    }

    pub(crate) fn accept_receipt(self, status: TradingStatusResponse) {
        // A read started before a successful mutation cannot roll back its receipt.
        self.revision
            .try_update(|revision| *revision = revision.wrapping_add(1));
        self.retry_until.try_set(None);
        self.freshness.confirm(ConfirmedAt::now());
        self.state.try_set(LoadState::Ready(status));
    }

    pub(crate) fn apply_read(
        self,
        revision: u64,
        result: Result<TradingStatusResponse, ApiProblem>,
    ) {
        self.apply_timed_read(revision, ConfirmedAt::now(), result);
    }

    fn apply_timed_read(
        self,
        revision: u64,
        started: ConfirmedAt,
        result: Result<TradingStatusResponse, ApiProblem>,
    ) {
        if self.revision.try_get_untracked() != Some(revision) {
            return;
        }
        let result = result.and_then(|status| {
            if started.expired() {
                Err(trading_status_stale())
            } else {
                self.freshness.confirm(started);
                Ok(status)
            }
        });
        self.retry_until.set(
            result
                .as_ref()
                .err()
                .and_then(|problem| retry_deadline_ms(problem.retry_after_ms, now_ms())),
        );
        self.state.update(|state| state.apply_result(result));
    }
}

pub(crate) fn provide_trading_status() -> TradingStatusState {
    let state = RwSignal::new(LoadState::Loading);
    let status = TradingStatusState {
        state,
        revision: RwSignal::new(0),
        retry_until: RwSignal::new(None),
        freshness: ReadFreshness::new(state, trading_status_stale()),
    };
    let scope = ReadScope::new(move || {
        status.invalidate();
    });
    scope.poll(
        Duration::from_secs(5),
        move || {
            status
                .retry_until
                .try_get_untracked()
                .is_some_and(|until| polling_allowed(true, until, now_ms()))
        },
        move |client| {
            let revision = status.revision.get_untracked();
            let started = ConfirmedAt::now();
            async move {
                (
                    revision,
                    started,
                    bounded_read(client.trading_status()).await,
                )
            }
        },
        move |(revision, started, result)| status.apply_timed_read(revision, started, result),
    );
    provide_context(status);
    status
}

fn trading_status_stale() -> ApiProblem {
    ApiProblem::new("TRADING_STATUS_STALE", "交易模式与风控状态超过 15 秒未确认")
        .with_source("trading_status")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_read_cannot_replace_saved_status_or_mark_it_stale() -> Result<(), serde_json::Error> {
        let initial: TradingStatusResponse = serde_json::from_value(serde_json::json!({
            "adapter": "mock", "openOrderCount": 0,
            "risk": { "liveTradingEnabled": false, "killSwitchActive": false,
                "maxOrderNotional": 12.75, "maxOpenOrders": 10, "maxHedgeImbalancePct": 0.05,
                "liquidationWarnPct": 0.02, "liquidationDangerPct": 0.01,
                "allowedExchanges": [], "allowedSymbols": [] },
            "wsChannels": { "orders": "orders", "execution": "execution", "riskAlerts": "risk-alerts" }
        }))?;
        let owner = Owner::new();
        owner.with(|| {
            let state = RwSignal::new(LoadState::Ready(initial.clone()));
            let status = TradingStatusState {
                state,
                revision: RwSignal::new(0),
                retry_until: RwSignal::new(None),
                freshness: ReadFreshness::new(state, trading_status_stale()),
            };
            let mut saved = initial.clone();
            saved.risk.auto_profit_close.min_net_profit_usd = 0.125;
            status.accept_receipt(saved.clone());
            status.apply_read(0, Ok(initial));
            status.apply_read(0, Err(ApiProblem::new("TIMEOUT", "old request")));
            assert!(
                matches!(status.state.get_untracked(), LoadState::Ready(value) if value == saved)
            );
            let mut next = saved;
            next.risk.kill_switch_active = true;
            status.apply_read(1, Ok(next.clone()));
            assert!(
                matches!(status.state.get_untracked(), LoadState::Ready(value) if value == next)
            );
        });
        Ok(())
    }
}
