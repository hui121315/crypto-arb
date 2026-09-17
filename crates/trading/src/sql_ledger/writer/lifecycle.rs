use super::{SqlLedgerWrite, SqlLedgerWriteError, ACK_TIMEOUT};
use tokio::sync::{mpsc, Notify};

const ENQUEUE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterState {
    Running,
    Draining,
    ShuttingDown,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::sql_ledger) enum ShutdownStart {
    Lead,
    WaitForDrain,
    WaitForStop,
    Done,
}

#[derive(Debug)]
pub(in crate::sql_ledger) struct WriterControl {
    state: parking_lot::Mutex<WriterState>,
    changed: Notify,
}

pub(in crate::sql_ledger) struct DrainGuard<'a> {
    control: &'a WriterControl,
}

pub(in crate::sql_ledger) struct ShutdownLeaderGuard<'a> {
    control: &'a WriterControl,
    rollback: bool,
}

impl WriterControl {
    pub(in crate::sql_ledger) fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(WriterState::Running),
            changed: Notify::new(),
        }
    }

    pub(in crate::sql_ledger) fn try_send(
        &self,
        sender: &mpsc::Sender<SqlLedgerWrite>,
        write: SqlLedgerWrite,
    ) -> Result<(), SqlLedgerWriteError> {
        let state = self.state.lock();
        if *state != WriterState::Running {
            return Err(SqlLedgerWriteError::QueueClosed);
        }
        sender.try_send(write).map_err(|error| match &error {
            mpsc::error::TrySendError::Full(_) => SqlLedgerWriteError::QueueFull,
            mpsc::error::TrySendError::Closed(_) => SqlLedgerWriteError::QueueClosed,
        })
    }

    pub(in crate::sql_ledger) async fn send(
        &self,
        sender: &mpsc::Sender<SqlLedgerWrite>,
        write: SqlLedgerWrite,
    ) -> Result<(), SqlLedgerWriteError> {
        let permit = tokio::time::timeout(ENQUEUE_TIMEOUT, sender.reserve())
            .await
            .map_err(|_| SqlLedgerWriteError::QueueFull)?
            .map_err(|_| SqlLedgerWriteError::QueueClosed)?;
        let state = self.state.lock();
        if *state != WriterState::Running {
            return Err(SqlLedgerWriteError::QueueClosed);
        }
        permit.send(write);
        Ok(())
    }

    pub(in crate::sql_ledger) fn begin_drain(&self) -> Result<DrainGuard<'_>, SqlLedgerWriteError> {
        let mut state = self.state.lock();
        if *state != WriterState::Running {
            return Err(SqlLedgerWriteError::QueueClosed);
        }
        *state = WriterState::Draining;
        Ok(DrainGuard { control: self })
    }

    pub(in crate::sql_ledger) fn finish_drain(&self) {
        let mut state = self.state.lock();
        if *state == WriterState::Draining {
            *state = WriterState::Running;
            self.changed.notify_waiters();
        }
    }

    pub(in crate::sql_ledger) fn begin_shutdown(&self) -> ShutdownStart {
        let mut state = self.state.lock();
        match *state {
            WriterState::Running => {
                *state = WriterState::ShuttingDown;
                ShutdownStart::Lead
            }
            WriterState::Stopped => ShutdownStart::Done,
            WriterState::Draining => ShutdownStart::WaitForDrain,
            WriterState::ShuttingDown => ShutdownStart::WaitForStop,
        }
    }

    pub(in crate::sql_ledger) fn abort_shutdown(&self) {
        let mut state = self.state.lock();
        if *state == WriterState::ShuttingDown {
            *state = WriterState::Running;
            self.changed.notify_waiters();
        }
    }

    pub(in crate::sql_ledger) fn shutdown_leader_guard(&self) -> ShutdownLeaderGuard<'_> {
        ShutdownLeaderGuard {
            control: self,
            rollback: true,
        }
    }

    pub(in crate::sql_ledger) fn mark_stopped(&self) {
        *self.state.lock() = WriterState::Stopped;
        self.changed.notify_waiters();
    }

    pub(in crate::sql_ledger) async fn wait_for_drain(&self) -> Result<(), SqlLedgerWriteError> {
        self.wait_while(WriterState::Draining).await
    }

    pub(in crate::sql_ledger) async fn wait_for_shutdown(&self) -> Result<(), SqlLedgerWriteError> {
        self.wait_while(WriterState::ShuttingDown).await
    }

    async fn wait_while(&self, expected: WriterState) -> Result<(), SqlLedgerWriteError> {
        let changed = self.changed.notified();
        if *self.state.lock() != expected {
            return Ok(());
        }
        tokio::time::timeout(ACK_TIMEOUT, changed)
            .await
            .map_err(|_| SqlLedgerWriteError::AckTimeout)
    }
}

impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        self.control.finish_drain();
    }
}

impl ShutdownLeaderGuard<'_> {
    pub(in crate::sql_ledger) fn command_enqueued(&mut self) {
        self.rollback = false;
    }
}

impl Drop for ShutdownLeaderGuard<'_> {
    fn drop(&mut self) {
        if self.rollback {
            self.control.abort_shutdown();
        }
    }
}

pub(in crate::sql_ledger) async fn send_barrier(
    sender: &mpsc::Sender<SqlLedgerWrite>,
    write: SqlLedgerWrite,
) -> Result<(), SqlLedgerWriteError> {
    tokio::time::timeout(ENQUEUE_TIMEOUT, sender.send(write))
        .await
        .map_err(|_| SqlLedgerWriteError::QueueFull)?
        .map_err(|_| SqlLedgerWriteError::QueueClosed)
}
