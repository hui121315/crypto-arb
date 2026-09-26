use crate::panels::modules::opportunity_counts::snapshot_clock;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

const MAX_RETAINED_TICKET_CLOCKS: usize = 128;

#[derive(Default)]
pub(in crate::panels::modules::execution) struct TicketClockHistory {
    entries: VecDeque<(String, TicketClock)>,
    retired_created_at_ms: i64,
}

impl TicketClockHistory {
    pub(super) fn retain(
        &mut self,
        ticket_id: &str,
        received: TicketClock,
    ) -> Result<TicketClock, shared_types::ApiProblem> {
        if let Some((_, original)) = self.entries.iter().find(|(id, _)| id == ticket_id) {
            return Ok(original.clone());
        }
        // Eviction must not let a replayed old ticket acquire a new clock.
        if received.created_at_ms <= self.retired_created_at_ms {
            return Err(shared_types::ApiProblem::new(
                "EXECUTION_PREVIEW_HISTORY_RETIRED",
                "返回的是已移出保留范围的旧交易检查，请刷新预览；不能重新确认这份旧报价",
            ).with_source("execution.preview.clock"));
        }
        self.entries.push_back((ticket_id.to_owned(), received.clone()));
        if self.entries.len() > MAX_RETAINED_TICKET_CLOCKS {
            if let Some((_, oldest)) = self.entries.pop_front() {
                self.retired_created_at_ms = self.retired_created_at_ms.max(oldest.created_at_ms);
            }
        }
        Ok(received)
    }
}

#[derive(Clone, Debug)]
pub(in crate::panels::modules::execution) struct TicketClock {
    created_at_ms: i64,
    requested_at: (i64, i64),
    latest: Arc<AtomicI64>,
}

impl PartialEq for TicketClock {
    fn eq(&self, other: &Self) -> bool {
        self.created_at_ms == other.created_at_ms
            && self.requested_at == other.requested_at
            && Arc::ptr_eq(&self.latest, &other.latest)
    }
}

impl TicketClock {
    pub(in crate::panels::modules::execution) fn new(created_at_ms: i64, requested_at: (i64, i64)) -> Self {
        Self {
            created_at_ms,
            requested_at,
            latest: Arc::new(AtomicI64::new(created_at_ms)),
        }
    }

    pub(in crate::panels::modules::execution) fn now_ms(&self) -> i64 {
        let (wall, monotonic) = snapshot_clock();
        // Start before the request: slow responses must not receive a fresh TTL.
        // Wall elapsed covers sleep; monotonic elapsed survives clock rollback.
        let elapsed = wall
            .saturating_sub(self.requested_at.0)
            .max(monotonic.saturating_sub(self.requested_at.1))
            .max(0);
        let now = self.created_at_ms.saturating_add(elapsed);
        // Clones and same-ticket refreshes must never resurrect expired evidence.
        self.latest.fetch_max(now, Ordering::Relaxed).max(now)
    }
}
