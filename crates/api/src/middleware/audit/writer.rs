use super::{AuditEvent, AuditSinkHealthSnapshot, AUDIT_WRITER_QUEUE_CAPACITY};
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const DURABLE_ACK_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(5);

pub(super) struct AuditSink {
    path: Option<PathBuf>,
    control: Mutex<WriterControl>,
    handle: Mutex<Option<JoinHandle<()>>>,
    state: Arc<Mutex<AuditSinkState>>,
}

struct WriterControl {
    sender: Option<mpsc::SyncSender<WriterCommand>>,
    shutdown_started: bool,
}

enum WriterCommand {
    Write(QueuedAuditLine),
    Shutdown(mpsc::SyncSender<Result<(), String>>),
}

struct QueuedAuditLine {
    line: String,
    ack: Option<mpsc::SyncSender<Result<(), String>>>,
}

struct AuditSinkState {
    opened: bool,
    writer_alive: bool,
    write_attempts: u64,
    write_successes: u64,
    write_failures: u64,
    last_write_at_ms: Option<i64>,
    last_error_at_ms: Option<i64>,
    last_error: Option<String>,
}

impl AuditSinkState {
    fn new(opened: bool, last_error: Option<String>) -> Self {
        let last_error_at_ms = last_error.as_ref().map(|_| common::time::now_ms());
        Self {
            opened,
            writer_alive: opened,
            write_attempts: 0,
            write_successes: 0,
            write_failures: 0,
            last_write_at_ms: None,
            last_error_at_ms,
            last_error,
        }
    }

    fn pending_writes(&self) -> u64 {
        self.write_attempts
            .saturating_sub(self.write_successes.saturating_add(self.write_failures))
    }

    fn record_attempt(&mut self) {
        self.write_attempts = self.write_attempts.saturating_add(1);
    }

    fn record_write_success(&mut self, now_ms: i64) {
        self.write_successes = self.write_successes.saturating_add(1);
        self.last_write_at_ms = Some(now_ms);
        self.last_error = None;
    }

    fn record_write_failure(&mut self, now_ms: i64, error: String) {
        self.write_failures = self.write_failures.saturating_add(1);
        self.last_error_at_ms = Some(now_ms);
        self.last_error = Some(error);
    }

    fn record_sink_failure(&mut self, now_ms: i64, error: String) {
        self.opened = false;
        self.writer_alive = false;
        self.last_error_at_ms = Some(now_ms);
        self.last_error = Some(error);
    }

    fn record_writer_stopped(&mut self) {
        self.writer_alive = false;
    }

    fn record_writer_disconnected(&mut self, now_ms: i64) {
        self.writer_alive = false;
        self.record_write_failure(now_ms, "writer_disconnected".to_owned());
    }
}

impl AuditSink {
    pub(super) fn new(path: Option<&str>) -> Self {
        let Some(raw) = path.map(PathBuf::from) else {
            return Self::unconfigured();
        };
        let (file, open_error) = open_configured_file(&raw);
        let opened = file.is_some();
        let state = Arc::new(Mutex::new(AuditSinkState::new(opened, open_error)));
        let writer = file.and_then(|file| spawn_writer(&raw, file, &state));
        let (sender, handle) = writer.map_or((None, None), |(sender, handle)| {
            (Some(sender), Some(handle))
        });
        Self {
            path: Some(raw),
            control: Mutex::new(WriterControl {
                sender,
                shutdown_started: false,
            }),
            handle: Mutex::new(handle),
            state,
        }
    }

    fn unconfigured() -> Self {
        Self {
            path: None,
            control: Mutex::new(WriterControl {
                sender: None,
                shutdown_started: false,
            }),
            handle: Mutex::new(None),
            state: Arc::new(Mutex::new(AuditSinkState::new(false, None))),
        }
    }

    pub(super) fn write(&self, event: &AuditEvent<'_>) {
        if self.path.is_none() {
            return;
        }
        match serialize_event(event) {
            Ok(line) => {
                let _ = self.enqueue_line(line, None);
            }
            Err(error) => {
                self.state.lock().record_attempt();
                self.record_failure(format!("serialize_failed: {error}"));
            }
        }
    }

    pub(super) fn write_durable(&self, event: &AuditEvent<'_>) -> Result<(), String> {
        if self.path.is_none() {
            return Ok(());
        }
        let line = serialize_event(event).map_err(|error| {
            let message = format!("serialize_failed: {error}");
            self.state.lock().record_attempt();
            self.record_failure(message.clone());
            message
        })?;
        let (ack_sender, ack_receiver) = mpsc::sync_channel(1);
        self.enqueue_line(line, Some(ack_sender))?;
        match ack_receiver.recv_timeout(DURABLE_ACK_TIMEOUT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let message = format!("durable_ack_timeout: {}ms", DURABLE_ACK_TIMEOUT.as_millis());
                self.record_failure(message.clone());
                Err(message)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let message = "durable_ack_disconnected".to_owned();
                self.record_failure(message.clone());
                Err(message)
            }
        }
    }

    pub(super) fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        if self.path.is_none() {
            return Ok(());
        }
        let (ack_sender, ack_receiver) = mpsc::sync_channel(1);
        let sender = {
            let mut control = self.control.lock();
            if control.shutdown_started {
                return Ok(());
            }
            control.shutdown_started = true;
            control.sender.take().ok_or_else(|| {
                let message = "audit writer unavailable during shutdown".to_owned();
                self.record_failure(message.clone());
                message
            })?
        };
        enqueue_shutdown(&sender, ack_sender, timeout)?;
        drop(sender);

        let drain_result = match ack_receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
                "audit_shutdown_drain_timeout: {}ms",
                timeout.as_millis()
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("audit_shutdown_drain_disconnected".to_owned())
            }
        };
        if let Err(error) = drain_result {
            self.record_failure(error.clone());
            return Err(error);
        }
        if let Some(handle) = self.handle.lock().take() {
            handle
                .join()
                .map_err(|_| "audit writer thread panicked during shutdown".to_owned())?;
        }
        Ok(())
    }

    fn enqueue_line(
        &self,
        line: String,
        ack: Option<mpsc::SyncSender<Result<(), String>>>,
    ) -> Result<(), String> {
        self.state.lock().record_attempt();
        let control = self.control.lock();
        let Some(sender) = control.sender.as_ref() else {
            let message = "audit writer unavailable".to_owned();
            drop(control);
            self.record_failure(message.clone());
            return Err(message);
        };
        match sender.try_send(WriterCommand::Write(QueuedAuditLine { line, ack })) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                let message = format!("queue_full: capacity={AUDIT_WRITER_QUEUE_CAPACITY}");
                drop(control);
                self.record_failure(message.clone());
                Err(message)
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                drop(control);
                let now_ms = common::time::now_ms();
                self.state.lock().record_writer_disconnected(now_ms);
                Err("writer_disconnected".to_owned())
            }
        }
    }

    fn record_failure(&self, error: String) {
        let now_ms = common::time::now_ms();
        self.state.lock().record_write_failure(now_ms, error);
    }

    pub(super) fn health_snapshot(&self, now_ms: i64) -> AuditSinkHealthSnapshot {
        let path = self.path.as_ref().map(|path| path.display().to_string());
        let queue_capacity = self
            .control
            .lock()
            .sender
            .as_ref()
            .map_or(0, |_| AUDIT_WRITER_QUEUE_CAPACITY as u64);
        let state = self.state.lock();
        AuditSinkHealthSnapshot {
            initialized: true,
            configured: self.path.is_some(),
            opened: state.opened,
            path,
            queue_capacity,
            pending_writes: state.pending_writes(),
            writer_alive: state.writer_alive,
            write_attempts: state.write_attempts,
            write_successes: state.write_successes,
            write_failures: state.write_failures,
            last_write_at_ms: state.last_write_at_ms,
            last_error_at_ms: state.last_error_at_ms,
            last_error: state.last_error.clone(),
            observed_at_ms: now_ms,
        }
    }

    #[cfg(test)]
    pub(super) fn wait_until_idle_for_test(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let idle = {
                let state = self.state.lock();
                state.pending_writes() == 0
            };
            if idle {
                return;
            }
            std::thread::sleep(SHUTDOWN_POLL_INTERVAL);
        }
    }
}

fn enqueue_shutdown(
    sender: &mpsc::SyncSender<WriterCommand>,
    ack: mpsc::SyncSender<Result<(), String>>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut command = WriterCommand::Shutdown(ack);
    loop {
        match sender.try_send(command) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(next)) if Instant::now() < deadline => {
                command = next;
                std::thread::sleep(SHUTDOWN_POLL_INTERVAL);
            }
            Err(mpsc::TrySendError::Full(_)) => {
                return Err(format!(
                    "audit_shutdown_queue_timeout: {}ms",
                    timeout.as_millis()
                ));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err("audit_shutdown_writer_disconnected".to_owned());
            }
        }
    }
}

fn spawn_writer(
    path: &Path,
    file: File,
    state: &Arc<Mutex<AuditSinkState>>,
) -> Option<(mpsc::SyncSender<WriterCommand>, JoinHandle<()>)> {
    let (sender, receiver) = mpsc::sync_channel(AUDIT_WRITER_QUEUE_CAPACITY);
    let writer_state = Arc::clone(state);
    match std::thread::Builder::new()
        .name("crossline-audit-jsonl-writer".to_owned())
        .spawn(move || writer_loop(file, &receiver, writer_state.as_ref()))
    {
        Ok(handle) => Some((sender, handle)),
        Err(error) => {
            let message = format!("writer_spawn_failed: {error}");
            state
                .lock()
                .record_sink_failure(common::time::now_ms(), message);
            tracing::warn!(path = %path.display(), %error, "audit: failed to spawn writer");
            None
        }
    }
}

fn writer_loop(
    mut file: File,
    receiver: &mpsc::Receiver<WriterCommand>,
    state: &Mutex<AuditSinkState>,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            WriterCommand::Write(queued) => write_queued_line(&mut file, state, queued),
            WriterCommand::Shutdown(ack) => {
                let result = file
                    .sync_data()
                    .map_err(|error| format!("shutdown_sync_failed: {error}"));
                if let Err(error) = result.as_ref() {
                    state
                        .lock()
                        .record_write_failure(common::time::now_ms(), error.clone());
                }
                state.lock().record_writer_stopped();
                let _ = ack.send(result);
                return;
            }
        }
    }
    state
        .lock()
        .record_writer_disconnected(common::time::now_ms());
}

fn write_queued_line(file: &mut File, state: &Mutex<AuditSinkState>, queued: QueuedAuditLine) {
    let result = writeln!(file, "{}", queued.line)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_data())
        .map_err(|error| format!("write_failed: {error}"));
    let now_ms = common::time::now_ms();
    match &result {
        Ok(()) => state.lock().record_write_success(now_ms),
        Err(error) => {
            state.lock().record_write_failure(now_ms, error.clone());
            tracing::warn!(%error, "audit: failed to durably write log line");
        }
    }
    if let Some(ack) = queued.ack {
        let _ = ack.send(result);
    }
}

fn open_configured_file(raw: &Path) -> (Option<File>, Option<String>) {
    let directory_error = ensure_parent_dir(raw);
    let (file, file_error) = open_append_file(raw);
    (file, file_error.or(directory_error))
}

fn ensure_parent_dir(raw: &Path) -> Option<String> {
    let parent = raw.parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    match std::fs::create_dir_all(parent) {
        Ok(()) => None,
        Err(error) => {
            tracing::warn!(
                path = %raw.display(),
                %error,
                "audit: failed to create log directory"
            );
            Some(format!("create_dir_failed: {error}"))
        }
    }
}

fn open_append_file(raw: &Path) -> (Option<File>, Option<String>) {
    match OpenOptions::new().create(true).append(true).open(raw) {
        Ok(file) => (Some(file), None),
        Err(error) => {
            tracing::warn!(
                path = %raw.display(),
                %error,
                "audit: failed to open log file"
            );
            (None, Some(format!("open_failed: {error}")))
        }
    }
}

fn serialize_event(event: &AuditEvent<'_>) -> Result<String, serde_json::Error> {
    serde_json::to_string(event).inspect_err(|error| {
        tracing::warn!(%error, "audit: failed to serialize event");
    })
}
