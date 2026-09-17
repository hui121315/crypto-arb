use super::*;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(super) struct PrivateWsHealthKey {
    pub(super) venue: String,
    pub(super) operation: &'static str,
}

impl PrivateWsHealthStore {
    pub(crate) fn order_session_owns_cache(&self, venue: &str) -> bool {
        self.operation_is_ready(venue, OP_PRIVATE_WS_SESSION, false)
            && self.operation_is_ready(venue, OP_PRIVATE_WS_SUBSCRIBE, false)
            && self.operation_is_ready(venue, OP_PRIVATE_WS_ORDER_STREAM, false)
    }

    pub(crate) fn account_session_owns_cache(&self, venue: &str) -> bool {
        self.operation_is_ready(venue, OP_PRIVATE_WS_SESSION, false)
            && self.operation_is_ready(venue, OP_PRIVATE_WS_SUBSCRIBE, false)
            && self.operation_is_ready(venue, OP_PRIVATE_WS_ACCOUNT_STREAM, true)
    }

    fn operation_is_ready(
        &self,
        venue: &str,
        operation: &'static str,
        require_clean: bool,
    ) -> bool {
        self.rows
            .get(&PrivateWsHealthKey {
                venue: venue.to_owned(),
                operation,
            })
            .is_some_and(|row| {
                row.status == VenueOperationStatus::Ok
                    && (!require_clean || row.account_dirty.is_none())
            })
    }

    pub(crate) fn snapshot(&self, now_ms: i64) -> Vec<PrivateWsRuntimeHealth> {
        self.rows
            .iter()
            .map(|row| stale_adjusted(row.value().clone(), now_ms))
            .collect()
    }

    pub(super) fn record(&self, venue: &str, update: PrivateWsRuntimeUpdate) {
        let now_ms = common::time::now_ms();
        let key = PrivateWsHealthKey {
            venue: venue.to_owned(),
            operation: update.operation,
        };
        let mut entry = self
            .rows
            .entry(key)
            .or_insert_with(|| PrivateWsRuntimeHealth {
                venue: venue.to_owned(),
                operation: update.operation,
                status: VenueOperationStatus::Unknown,
                message: String::new(),
                request_id: None,
                requested: None,
                rows: None,
                freshness_ms: None,
                retry_after_ms: None,
                error: None,
                observed_at_ms: now_ms,
                ok_count: 0,
                warn_count: 0,
                blocked_count: 0,
                last_problem: None,
                last_problem_at_ms: None,
                account_dirty: None,
            });
        let health = entry.value_mut();
        match update.status {
            VenueOperationStatus::Ok => health.ok_count = health.ok_count.saturating_add(1),
            VenueOperationStatus::Warn => health.warn_count = health.warn_count.saturating_add(1),
            VenueOperationStatus::Blocked => {
                health.blocked_count = health.blocked_count.saturating_add(1);
            }
            VenueOperationStatus::Unsupported | VenueOperationStatus::Unknown => {}
        }
        if matches!(
            update.status,
            VenueOperationStatus::Warn | VenueOperationStatus::Blocked
        ) {
            health.last_problem = Some(update.message.clone());
            health.last_problem_at_ms = Some(now_ms);
        }
        health.status = update.status;
        health.message = update.message;
        health.request_id = update.request_id;
        health.requested = update.requested;
        health.rows = update.rows;
        health.freshness_ms = Some(0);
        health.retry_after_ms = update.retry_after_ms;
        health.error = update.error;
        health.account_dirty = update.account_dirty;
        health.observed_at_ms = now_ms;
    }

    pub(crate) fn record_account_cache_refreshed(
        &self,
        venue: &str,
        refreshed_scope: PrivateAccountScope,
    ) {
        let key = PrivateWsHealthKey {
            venue: venue.to_owned(),
            operation: OP_PRIVATE_WS_ACCOUNT_STREAM,
        };
        let Some(mut entry) = self.rows.get_mut(&key) else {
            return;
        };
        let health = entry.value_mut();
        let Some(dirty) = health.account_dirty.as_ref() else {
            return;
        };
        let remaining_scope = remaining_dirty_scope(dirty.scope, refreshed_scope);
        if remaining_scope == Some(dirty.scope) {
            return;
        }

        let now_ms = common::time::now_ms();
        health.status = VenueOperationStatus::Ok;
        health.error = None;
        health.retry_after_ms = None;
        health.freshness_ms = Some(0);
        health.observed_at_ms = now_ms;
        health.ok_count = health.ok_count.saturating_add(1);
        if let Some(scope) = remaining_scope {
            if let Some(dirty) = health.account_dirty.as_mut() {
                dirty.scope = scope;
            }
            health.message = format!(
                "私有 WS 账户变更正在后台同步，仍需刷新 {} 快照",
                scope.as_str()
            );
        } else {
            health.account_dirty = None;
            health.message = "私有 WS 账户变更已通过后台快照同步".to_owned();
        }
    }

    pub(super) fn record_stream_waiting(&self, venue: &str, operation: &'static str, label: &str) {
        self.record(
            venue,
            PrivateWsRuntimeUpdate::new(
                operation,
                VenueOperationStatus::Unknown,
                format!("私有 WS 订阅已发送，等待{label}事件样本"),
            )
            .with_rows(0),
        );
    }
}

fn remaining_dirty_scope(
    dirty: PrivateAccountScope,
    refreshed: PrivateAccountScope,
) -> Option<PrivateAccountScope> {
    match (dirty, refreshed) {
        (_, PrivateAccountScope::All)
        | (PrivateAccountScope::Balances, PrivateAccountScope::Balances)
        | (PrivateAccountScope::Positions, PrivateAccountScope::Positions) => None,
        (PrivateAccountScope::All, PrivateAccountScope::Balances) => {
            Some(PrivateAccountScope::Positions)
        }
        (PrivateAccountScope::All, PrivateAccountScope::Positions) => {
            Some(PrivateAccountScope::Balances)
        }
        (scope, _) => Some(scope),
    }
}

fn stale_adjusted(mut row: PrivateWsRuntimeHealth, now_ms: i64) -> PrivateWsRuntimeHealth {
    let freshness_ms = now_ms.saturating_sub(row.observed_at_ms);
    row.freshness_ms = Some(freshness_ms);
    if row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM
        && row.account_dirty.is_some()
        && freshness_ms > ACCOUNT_REFETCH_GRACE_MS
    {
        let message = format!(
            "私有 WS 账户变更已等待 {}ms，后台账户快照尚未同步完成",
            freshness_ms
        );
        row.status = VenueOperationStatus::Warn;
        row.message = message.clone();
        row.error = Some(message);
        row.retry_after_ms = Some(2_000);
    }
    row
}
