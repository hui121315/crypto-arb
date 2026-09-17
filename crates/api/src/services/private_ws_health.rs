use crate::trading_service::private_ws_events::{
    PrivateAccountDirty, PrivateAccountScope, PrivateWsEvent,
};
use dashmap::DashMap;
use shared_types::VenueOperationStatus;
pub(crate) use shared_types::{
    OP_PRIVATE_WS_ACCOUNT_STREAM, OP_PRIVATE_WS_ORDER_STREAM, OP_PRIVATE_WS_SESSION,
    OP_PRIVATE_WS_SUBSCRIBE,
};
use std::collections::BTreeSet;
pub(crate) const SOURCE_PRIVATE_WS_RUNTIME: &str = "private_ws_runtime";
#[cfg(test)]
const TEST_FRESHNESS_WINDOW_MS: i64 = 120_000;
const ACCOUNT_REFETCH_GRACE_MS: i64 = 5_000;
const PRIVATE_WS_RETRY_AFTER_MS: u64 = 15_000;

mod counts;
mod runtime;
mod state;
mod update;
use counts::PrivateWsEventCounts;
use state::PrivateWsHealthKey;
use update::PrivateWsRuntimeUpdate;

#[derive(Default)]
pub(crate) struct PrivateWsHealthStore {
    rows: DashMap<PrivateWsHealthKey, PrivateWsRuntimeHealth>,
    subscription_acks: DashMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrivateWsRuntimeHealth {
    pub(crate) venue: String,
    pub(crate) operation: &'static str,
    pub(crate) status: VenueOperationStatus,
    pub(crate) message: String,
    pub(crate) request_id: Option<String>,
    pub(crate) requested: Option<u64>,
    pub(crate) rows: Option<u64>,
    pub(crate) freshness_ms: Option<i64>,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) error: Option<String>,
    pub(crate) observed_at_ms: i64,
    pub(crate) ok_count: u64,
    pub(crate) warn_count: u64,
    pub(crate) blocked_count: u64,
    pub(crate) last_problem: Option<String>,
    pub(crate) last_problem_at_ms: Option<i64>,
    pub(crate) account_dirty: Option<PrivateAccountDirty>,
}

impl PrivateWsHealthStore {
    pub(crate) fn record_events(&self, venue: &str, events: &[PrivateWsEvent]) {
        let counts = PrivateWsEventCounts::from_events(events);
        if counts.orders > 0 {
            self.record(
                venue,
                PrivateWsRuntimeUpdate::ok(OP_PRIVATE_WS_ORDER_STREAM, "私有 WS 收到订单事件")
                    .with_rows(counts.orders),
            );
        }
        if counts.accounts > 0 {
            self.record(
                venue,
                PrivateWsRuntimeUpdate::ok(OP_PRIVATE_WS_ACCOUNT_STREAM, "私有 WS 收到账户事件")
                    .with_rows(counts.accounts),
            );
        }
    }

    pub(crate) fn record_stream_progress(
        &self,
        venue: &str,
        operation: &'static str,
        label: &str,
        samples: usize,
        expected: usize,
    ) {
        let ready = expected > 0 && samples == expected;
        let status = if ready {
            VenueOperationStatus::Ok
        } else {
            VenueOperationStatus::Unknown
        };
        let message = if ready {
            format!("私有 WS {label}已取得 {samples}/{expected} 条权威样本")
        } else {
            format!("私有 WS {label}等待权威样本：{samples}/{expected}")
        };
        self.record(
            venue,
            PrivateWsRuntimeUpdate::new(operation, status, message)
                .with_requested(expected)
                .with_rows(samples),
        );
    }

    pub(crate) fn record_subscriptions_proven(&self, venue: &str, expected: usize) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::ok(
                OP_PRIVATE_WS_SUBSCRIBE,
                "私有 WS 全部配置通道均已产生权威运行样本",
            )
            .with_requested(expected)
            .with_rows(expected),
        );
    }
}

#[cfg(test)]
mod tests;
