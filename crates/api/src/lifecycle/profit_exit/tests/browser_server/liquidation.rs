//! Synthetic liquidation prices over real paper fills; not exchange margin evidence.
use super::{AppState, BackgroundTasks, MarketPhase};
use crate::services::{portfolio, portfolio_snapshot_envelope};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::time::Duration;

pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks, phase: Arc<AtomicU8>) {
    let state = state.clone();
    let shutdown = tasks.shutdown_token();
    // This is the only portfolio writer in the explicit liquidation fixture.
    tasks.supervise("portfolio", 500, move || {
        let state = state.clone();
        let shutdown = shutdown.clone();
        let phase = phase.clone();
        async move {
            let mut tick = tokio::time::interval(Duration::from_millis(500));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    _ = tick.tick() => {}
                }
                let mut snapshot = portfolio::snapshot(&state).await.expect("paper portfolio");
                let current = phase.load(Ordering::Relaxed);
                for row in &mut snapshot.positions {
                    if row.venue == "binance" && row.side == shared_types::PositionSide::Long {
                        row.liquidation_price = match current {
                            value if value == MarketPhase::LiquidationSafe as u8 => {
                                Some(row.mark_price * 0.8)
                            }
                            value if value == MarketPhase::LiquidationNear as u8 => {
                                Some(row.mark_price * 0.95)
                            }
                            _ => None,
                        };
                    }
                }
                ::portfolio::annotate_liquidation_distance(&mut snapshot.positions);
                let now = snapshot.server_now_ms;
                let envelope = portfolio_snapshot_envelope::snapshot_envelope(
                    snapshot.clone(),
                    "isolated_paper_liquidation_fixture",
                    now,
                );
                state.cache_portfolio_snapshot(snapshot);
                state.cache_portfolio_snapshot_envelope(envelope.clone());
                if state
                    .ws_hub()
                    .subscriber_count(realtime::channels::PORTFOLIO)
                    > 0
                {
                    state.ws_hub().publish_throttled(
                        realtime::channels::PORTFOLIO,
                        realtime::WsMessage::json(&envelope).expect("paper portfolio envelope"),
                    );
                }
                state
                    .task_registry()
                    .record_result_timed("portfolio", now, Ok(()));
            }
        }
    });
}
