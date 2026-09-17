use crate::trading_service::private_ws_events::{PrivateAccountDirty, PrivateAccountScope};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivateAccountRefresh {
    pub(crate) venue: String,
    pub(crate) scope: PrivateAccountScope,
    pub(crate) wait_for_followup_ws: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueuedPrivateAccountRefresh {
    scope: PrivateAccountScope,
    wait_for_followup_ws: bool,
}

#[derive(Default)]
pub(crate) struct PrivateAccountRefreshQueue {
    scopes: Mutex<BTreeMap<String, QueuedPrivateAccountRefresh>>,
    ready: Notify,
}

impl PrivateAccountRefreshQueue {
    pub(crate) fn enqueue(&self, dirty: impl IntoIterator<Item = PrivateAccountDirty>) {
        let mut scopes = self.scopes.lock();
        let mut queued = false;
        for dirty in dirty {
            let venue = dirty.venue.trim().to_ascii_lowercase();
            if venue.is_empty() {
                continue;
            }
            let wait_for_followup_ws = waits_for_followup_account_ws(&dirty.reason);
            scopes
                .entry(venue)
                .and_modify(|refresh| {
                    refresh.scope = merge_scope(refresh.scope, dirty.scope);
                    refresh.wait_for_followup_ws &= wait_for_followup_ws;
                })
                .or_insert(QueuedPrivateAccountRefresh {
                    scope: dirty.scope,
                    wait_for_followup_ws,
                });
            queued = true;
        }
        drop(scopes);
        if queued {
            self.ready.notify_one();
        }
    }

    pub(crate) async fn notified(&self) {
        self.ready.notified().await;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.scopes.lock().is_empty()
    }

    pub(crate) fn drain(&self) -> Vec<PrivateAccountRefresh> {
        std::mem::take(&mut *self.scopes.lock())
            .into_iter()
            .map(|(venue, refresh)| PrivateAccountRefresh {
                venue,
                scope: refresh.scope,
                wait_for_followup_ws: refresh.wait_for_followup_ws,
            })
            .collect()
    }
}

fn waits_for_followup_account_ws(reason: &str) -> bool {
    matches!(reason, "fill_event" | "terminal_order_update")
}

const fn merge_scope(left: PrivateAccountScope, right: PrivateAccountScope) -> PrivateAccountScope {
    match (left, right) {
        (PrivateAccountScope::Balances, PrivateAccountScope::Balances) => {
            PrivateAccountScope::Balances
        }
        (PrivateAccountScope::Positions, PrivateAccountScope::Positions) => {
            PrivateAccountScope::Positions
        }
        _ => PrivateAccountScope::All,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_merges_bursts_by_normalized_venue_and_scope() {
        let queue = PrivateAccountRefreshQueue::default();
        queue.enqueue([
            PrivateAccountDirty::new("binance", PrivateAccountScope::Balances, "balance"),
            PrivateAccountDirty::new("BINANCE", PrivateAccountScope::Positions, "position"),
            PrivateAccountDirty::new("okx", PrivateAccountScope::Balances, "first"),
        ]);
        queue.enqueue([PrivateAccountDirty::new(
            "okx",
            PrivateAccountScope::Balances,
            "second",
        )]);

        assert_eq!(
            queue.drain(),
            vec![
                PrivateAccountRefresh {
                    venue: "binance".into(),
                    scope: PrivateAccountScope::All,
                    wait_for_followup_ws: false,
                },
                PrivateAccountRefresh {
                    venue: "okx".into(),
                    scope: PrivateAccountScope::Balances,
                    wait_for_followup_ws: false,
                },
            ]
        );
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn queue_ignores_empty_venue_keys() {
        let queue = PrivateAccountRefreshQueue::default();
        queue.enqueue([PrivateAccountDirty::new(
            "  ",
            PrivateAccountScope::All,
            "invalid",
        )]);

        assert!(queue.drain().is_empty());
    }

    #[test]
    fn order_driven_dirty_waits_for_followup_account_ws() {
        let queue = PrivateAccountRefreshQueue::default();
        queue.enqueue([
            PrivateAccountDirty::new("binance", PrivateAccountScope::All, "fill_event"),
            PrivateAccountDirty::new(
                "okx",
                PrivateAccountScope::Positions,
                "terminal_order_update",
            ),
        ]);

        assert_eq!(
            queue.drain(),
            vec![
                PrivateAccountRefresh {
                    venue: "binance".into(),
                    scope: PrivateAccountScope::All,
                    wait_for_followup_ws: true,
                },
                PrivateAccountRefresh {
                    venue: "okx".into(),
                    scope: PrivateAccountScope::Positions,
                    wait_for_followup_ws: true,
                },
            ]
        );
    }

    #[test]
    fn explicit_rest_refresh_wins_when_dirty_reasons_merge() {
        let queue = PrivateAccountRefreshQueue::default();
        queue.enqueue([
            PrivateAccountDirty::new("binance", PrivateAccountScope::All, "fill_event"),
            PrivateAccountDirty::new(
                "binance",
                PrivateAccountScope::Positions,
                "position_delta_requires_rest_refresh",
            ),
        ]);

        assert_eq!(
            queue.drain(),
            vec![PrivateAccountRefresh {
                venue: "binance".into(),
                scope: PrivateAccountScope::All,
                wait_for_followup_ws: false,
            }]
        );
    }
}
