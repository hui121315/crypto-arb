use super::*;

const PRIVATE_WS_RESTART_MIN_DELAY: Duration = Duration::from_secs(30);
const PRIVATE_WS_RESTART_MAX_DELAY: Duration = Duration::from_secs(15 * 60);

pub(super) struct VenueTask<K> {
    key: Option<K>,
    handle: Option<JoinHandle<()>>,
    consecutive_exits: u32,
    retry_not_before: Option<Instant>,
}

impl<K> Drop for VenueTask<K> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl<K> Default for VenueTask<K> {
    fn default() -> Self {
        Self {
            key: None,
            handle: None,
            consecutive_exits: 0,
            retry_not_before: None,
        }
    }
}

impl<K> VenueTask<K>
where
    K: Clone + PartialEq + std::fmt::Debug,
{
    pub(super) fn reconcile(
        &mut self,
        venue: &'static str,
        health: &crate::services::private_ws_health::PrivateWsHealthStore,
        key: K,
        spawn: impl FnOnce(K) -> Option<JoinHandle<()>>,
    ) {
        let same_key = self.key.as_ref() == Some(&key);
        if same_key {
            if self.should_wait_before_restart(venue) {
                return;
            }
        } else {
            self.reset_for_new_key(venue, health);
        }

        self.start(venue, key, spawn);
    }

    fn should_wait_before_restart(&mut self, venue: &'static str) -> bool {
        if self
            .handle
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
        {
            return true;
        }
        if self.handle.take().is_some() {
            self.schedule_restart(venue);
            return true;
        }
        self.retry_not_before
            .is_some_and(|not_before| Instant::now() < not_before)
    }

    fn schedule_restart(&mut self, venue: &'static str) {
        self.consecutive_exits = self.consecutive_exits.saturating_add(1);
        let delay = private_ws_restart_delay(self.consecutive_exits);
        self.retry_not_before = Instant::now().checked_add(delay);
        warn!(
            %venue,
            consecutive_exits = self.consecutive_exits,
            retry_after_secs = delay.as_secs(),
            "private ws venue task exited; restart delayed"
        );
    }

    fn reset_for_new_key(
        &mut self,
        venue: &'static str,
        health: &crate::services::private_ws_health::PrivateWsHealthStore,
    ) {
        self.abort(venue, health);
        self.consecutive_exits = 0;
        self.retry_not_before = None;
    }

    fn start(
        &mut self,
        venue: &'static str,
        key: K,
        spawn: impl FnOnce(K) -> Option<JoinHandle<()>>,
    ) {
        if let Some(handle) = spawn(key.clone()) {
            self.key = Some(key);
            self.handle = Some(handle);
            self.retry_not_before = None;
            info!(%venue, "private ws venue task started");
        } else {
            self.key = None;
        }
    }

    fn abort(
        &mut self,
        venue: &'static str,
        health: &crate::services::private_ws_health::PrivateWsHealthStore,
    ) {
        if let Some(handle) = self.handle.take() {
            let was_running = !handle.is_finished();
            handle.abort();
            if was_running {
                health.record_task_aborted(venue);
                debug!(%venue, "private ws venue task aborted");
            }
        }
    }
}

fn private_ws_restart_delay(consecutive_exits: u32) -> Duration {
    let exponent = consecutive_exits.saturating_sub(1).min(5);
    PRIVATE_WS_RESTART_MIN_DELAY
        .saturating_mul(1_u32 << exponent)
        .min(PRIVATE_WS_RESTART_MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_ws_restart_delay_is_exponential_and_bounded() {
        assert_eq!(private_ws_restart_delay(1), Duration::from_secs(30));
        assert_eq!(private_ws_restart_delay(2), Duration::from_secs(60));
        assert_eq!(private_ws_restart_delay(3), Duration::from_secs(120));
        assert_eq!(private_ws_restart_delay(6), Duration::from_secs(900));
        assert_eq!(private_ws_restart_delay(u32::MAX), Duration::from_secs(900));
    }
}
