use super::ShutdownToken;
use tokio::sync::Notify;
use tokio::time::Interval;

const EVENT_COALESCE_WINDOW: std::time::Duration = std::time::Duration::from_millis(100);

pub(in crate::lifecycle) enum RefreshTrigger {
    Event,
    Interval,
    Shutdown,
}

pub(in crate::lifecycle) async fn next(
    tick: &mut Interval,
    refresh: &Notify,
    shutdown: &ShutdownToken,
) -> RefreshTrigger {
    tokio::select! {
        biased;
        () = shutdown.cancelled() => RefreshTrigger::Shutdown,
        () = refresh.notified() => RefreshTrigger::Event,
        _ = tick.tick() => RefreshTrigger::Interval,
    }
}

pub(in crate::lifecycle) async fn coalesce(refresh: &Notify, shutdown: &ShutdownToken) -> bool {
    let deadline = tokio::time::sleep(EVENT_COALESCE_WINDOW);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return false,
            () = &mut deadline => return true,
            () = refresh.notified() => {}
        }
    }
}
