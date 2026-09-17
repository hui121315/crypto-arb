use super::{
    PrivateWsHealthStore, PrivateWsRuntimeUpdate, OP_PRIVATE_WS_ACCOUNT_STREAM,
    OP_PRIVATE_WS_ORDER_STREAM,
};
use crate::trading_service::private_ws_events::{PrivateWsApplyOutcome, PrivateWsEvent};

#[derive(Default)]
pub(super) struct PrivateWsEventCounts {
    pub(super) orders: usize,
    pub(super) accounts: usize,
}

impl PrivateWsEventCounts {
    pub(super) fn from_events(events: &[PrivateWsEvent]) -> Self {
        let mut counts = Self::default();
        for event in events {
            match event {
                PrivateWsEvent::Order(_)
                | PrivateWsEvent::OpenOrders(_)
                | PrivateWsEvent::BinanceOrderTrade(_)
                | PrivateWsEvent::Fill(_)
                | PrivateWsEvent::FillWithEvidence(_)
                | PrivateWsEvent::NonUserCancel(_) => counts.orders += 1,
                PrivateWsEvent::Funding(_) | PrivateWsEvent::Liquidation(_) => {
                    counts.accounts += 1;
                }
                PrivateWsEvent::Positions(_)
                | PrivateWsEvent::PositionPatch(_)
                | PrivateWsEvent::Balances(_)
                | PrivateWsEvent::BalancePatch(_)
                | PrivateWsEvent::AssetValuations(_)
                | PrivateWsEvent::AccountSummary(_) => counts.accounts += 1,
                PrivateWsEvent::AccountDirty(_) => {}
            }
        }
        counts
    }
}

#[derive(Default)]
pub(super) struct PrivateWsApplyCounts {
    pub(super) orders: usize,
    pub(super) accounts: usize,
}

impl PrivateWsApplyCounts {
    pub(super) fn from_outcome(outcome: &PrivateWsApplyOutcome) -> Self {
        let mut counts = Self::default();
        if outcome.order.is_some() || outcome.ledger_updated || outcome.open_order_cache_updated {
            counts.orders += 1;
        }
        if outcome.account_cache_updated || outcome.balance_ledger_updated {
            counts.accounts += 1;
        }
        counts
    }
}

impl PrivateWsHealthStore {
    pub(crate) fn record_apply_outcome(&self, venue: &str, outcome: &PrivateWsApplyOutcome) {
        let counts = PrivateWsApplyCounts::from_outcome(outcome);
        if counts.orders > 0 {
            self.record(
                venue,
                PrivateWsRuntimeUpdate::ok(OP_PRIVATE_WS_ORDER_STREAM, "私有 WS 订单事件已入账")
                    .with_rows(counts.orders),
            );
        }
        if let Some(dirty) = outcome.account_cache_dirty.as_ref() {
            self.record_account_dirty(venue, dirty);
        } else if counts.accounts > 0 {
            self.record(
                venue,
                PrivateWsRuntimeUpdate::ok(OP_PRIVATE_WS_ACCOUNT_STREAM, "私有 WS 账户缓存已更新")
                    .with_rows(counts.accounts),
            );
        }
    }

    fn record_account_dirty(
        &self,
        venue: &str,
        dirty: &crate::trading_service::private_ws_events::PrivateAccountDirty,
    ) {
        let message = format!(
            "私有 WS 已收到账户变更，正在后台同步账户快照：venue={}; scope={}",
            dirty.venue,
            dirty.scope.as_str()
        );
        self.record(
            venue,
            PrivateWsRuntimeUpdate::ok(OP_PRIVATE_WS_ACCOUNT_STREAM, &message)
                .with_requested(1)
                .with_rows(0)
                .with_account_dirty(dirty.clone()),
        );
    }

    pub(crate) fn record_apply_durability_failure(
        &self,
        venue: &str,
        outcome: &PrivateWsApplyOutcome,
        error: &str,
    ) {
        if let Some(dirty) = outcome.account_cache_dirty.as_ref() {
            self.record_account_dirty(venue, dirty);
        }
        self.record(
            venue,
            PrivateWsRuntimeUpdate::blocked(
                OP_PRIVATE_WS_ORDER_STREAM,
                format!("私有 WS 账本持久化失败：{error}"),
            )
            .with_rows(outcome.ledger_events.len())
            .with_error(error),
        );
    }
}
