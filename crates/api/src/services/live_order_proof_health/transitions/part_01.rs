use dashmap::DashMap;
use shared_types::{
    normalized_venue_name, venue_family, ExecutionMode, LiveOrderState, OrderRecord,
    OrderUpdateSource, VenueOperationStatus,
};

pub(crate) const SOURCE_LIVE_ORDER_PROOF_RUNTIME: &str = "live_order_proof_runtime";

const LIVE_ORDER_PROOF_EXPECTED_STEPS: u64 = 2;
const LIVE_ORDER_PROOF_RETRY_AFTER_MS: u64 = 60_000;

pub(crate) struct LiveOrderProofHealthStore {
    rows: DashMap<String, LiveOrderProofRuntimeHealth>,
    checkpoint: LiveOrderProofCheckpointStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveOrderProofRuntimeHealth {
    pub(crate) venue: String,
    pub(crate) status: VenueOperationStatus,
    pub(crate) message: String,
    pub(crate) request_id: Option<String>,
    pub(crate) requested: Option<u64>,
    pub(crate) rows: Option<u64>,
    pub(crate) freshness_ms: Option<i64>,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) error: Option<String>,
    pub(crate) observed_at_ms: i64,
    pub(crate) place_proof: Option<LiveOrderProofSample>,
    pub(crate) cancel_request: Option<LiveOrderProofSample>,
    pub(crate) cancel_finality: Option<LiveOrderProofSample>,
    pub(crate) last_problem: Option<LiveOrderProofProblem>,
    pub(crate) place_ack_count: u64,
    pub(crate) cancel_requested_count: u64,
    pub(crate) cancel_finality_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveOrderProofSample {
    pub(crate) venue: String,
    #[serde(default)]
    pub(crate) account_scope: Option<String>,
    #[serde(default)]
    pub(crate) product: shared_types::FeeProduct,
    pub(crate) symbol: String,
    pub(crate) internal_order_id: String,
    pub(crate) exchange_order_id: Option<String>,
    pub(crate) client_order_id: Option<String>,
    pub(crate) source: String,
    pub(crate) checked_at_ms: i64,
    pub(crate) request_id: Option<String>,
    pub(crate) native_transport: Option<String>,
    pub(crate) native_request_id: Option<String>,
    pub(crate) native_response_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveOrderProofProblem {
    pub(crate) message: String,
    pub(crate) source: String,
    pub(crate) request_id: Option<String>,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) status: Option<u16>,
    pub(crate) observed_at_ms: i64,
}

pub(crate) struct LiveOrderProofProblemInput<'a> {
    pub(crate) venue: &'a str,
    pub(crate) source: &'a str,
    pub(crate) message: &'a str,
    pub(crate) request_id: Option<String>,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) status: Option<u16>,
}

#[derive(Debug, Clone, Copy)]
enum LiveOrderProofEvent {
    PlaceAck,
    CancelRequested,
    CancelFinality,
}

impl Default for LiveOrderProofHealthStore {
    fn default() -> Self {
        Self {
            rows: DashMap::new(),
            checkpoint: LiveOrderProofCheckpointStore::default(),
        }
    }
}

impl LiveOrderProofHealthStore {
    pub(crate) fn snapshot(&self, now_ms: i64) -> Vec<LiveOrderProofRuntimeHealth> {
        self.rows
            .iter()
            .filter(|row| {
                [&row.place_proof, &row.cancel_request, &row.cancel_finality]
                    .into_iter()
                    .flatten()
                    .all(|sample| self.sample_matches_current_account(sample))
            })
            .map(|row| with_freshness(row.value().clone(), now_ms))
            .collect()
    }

    pub(crate) fn record_place_ack(&self, sample: LiveOrderProofSample) {
        self.record_sample(sample, LiveOrderProofEvent::PlaceAck);
    }

    pub(crate) fn record_cancel_requested(&self, sample: LiveOrderProofSample) {
        self.record_sample(sample, LiveOrderProofEvent::CancelRequested);
    }

    pub(crate) fn record_cancel_finality(&self, sample: LiveOrderProofSample) {
        self.record_sample(sample, LiveOrderProofEvent::CancelFinality);
    }

    pub(crate) fn record_submit_ack_from_record(&self, record: &OrderRecord) {
        if !is_live_adapter_ack(record) || !place_ack_state(record.state) {
            return;
        }
        self.record_place_ack(record_sample(record, "adapter_ack"));
    }

    pub(crate) fn record_cancel_ack_from_record(&self, record: &OrderRecord) {
        if !is_live_adapter_ack(record) {
            return;
        }
        if matches!(
            record.state,
            LiveOrderState::CancelRequested | LiveOrderState::Cancelled
        ) {
            let sample = record_sample(record, "adapter_ack");
            self.record_cancel_requested(sample.clone());
            if record.state == LiveOrderState::Cancelled {
                self.record_cancel_finality(sample);
            }
        }
    }

    pub(crate) fn record_order_query_cancel_finality_from_record(&self, record: &OrderRecord) {
        if record.intent.mode != ExecutionMode::Live
            || record.last_update_source != OrderUpdateSource::OrderQuery
            || record.state != LiveOrderState::Cancelled
        {
            return;
        }
        self.record_cancel_finality(record_sample(record, "order_query"));
    }

    pub(crate) fn record_private_ws_cancel_finality_from_record(
        &self,
        record: &OrderRecord,
        source: &str,
    ) {
        if record.intent.mode != ExecutionMode::Live
            || record.last_update_source != OrderUpdateSource::PrivateWs
            || record.state != LiveOrderState::Cancelled
        {
            return;
        }
        self.record_cancel_finality(record_sample(record, source));
    }

    pub(crate) fn record_problem(&self, problem: LiveOrderProofProblemInput<'_>) {
        let now_ms = common::time::now_ms();
        let key = normalized_venue_name(problem.venue);
        let mut entry = self
            .rows
            .entry(key.clone())
            .or_insert_with(|| empty_runtime_health(problem.venue, now_ms));
        let row = entry.value_mut();
        row.venue = problem.venue.to_owned();
        row.last_problem = Some(LiveOrderProofProblem {
            message: problem.message.to_owned(),
            source: problem.source.to_owned(),
            request_id: problem.request_id,
            retry_after_ms: problem.retry_after_ms,
            status: problem.status,
            observed_at_ms: now_ms,
        });
        refresh_runtime_row(row);
        drop(entry);
        self.remove_checkpoint_exact(&key);
    }

    pub(crate) fn invalidate_credentials_update(&self, venue: &str) {
        let exact_key = normalized_venue_name(venue);
        let family_key = normalized_venue_name(venue_family(venue));
        let keys = self
            .rows
            .iter()
            .filter_map(|entry| {
                let key = entry.key();
                let key_family = normalized_venue_name(venue_family(key));
                (key == &exact_key || key_family == family_key).then(|| key.clone())
            })
            .collect::<Vec<_>>();

        for key in keys {
            self.rows.remove(&key);
        }
        self.remove_checkpoint_family(venue);
    }

    fn record_sample(&self, sample: LiveOrderProofSample, event: LiveOrderProofEvent) {
        // A response from a pinned old adapter must not prove the newly saved account.
        if !self.sample_matches_current_account(&sample) {
            return;
        }
        let key = normalized_venue_name(&sample.venue);
        let mut entry = self
            .rows
            .entry(key.clone())
            .or_insert_with(|| empty_runtime_health(&sample.venue, sample.checked_at_ms));
        let row = entry.value_mut();
        row.venue = sample.venue.clone();
        let preserve_completed_pair = should_preserve_completed_pair(row, event, &sample);
        match event {
            LiveOrderProofEvent::PlaceAck => {
                row.place_ack_count = row.place_ack_count.saturating_add(1);
                if !preserve_completed_pair {
                    row.place_proof = Some(sample);
                }
            }
            LiveOrderProofEvent::CancelRequested => {
                row.cancel_requested_count = row.cancel_requested_count.saturating_add(1);
                if !preserve_completed_pair {
                    row.cancel_request = Some(sample);
                }
            }
            LiveOrderProofEvent::CancelFinality => {
                row.cancel_finality_count = row.cancel_finality_count.saturating_add(1);
                if !preserve_completed_pair {
                    row.cancel_finality = Some(sample);
                }
            }
        }
        refresh_runtime_row(row);
        let persist_completed_pair = !preserve_completed_pair && has_complete_remote_proof(row);
        drop(entry);
        if persist_completed_pair {
            self.persist_complete_checkpoint(&key);
        }
    }

    fn sample_matches_current_account(&self, sample: &LiveOrderProofSample) -> bool {
        self.checkpoint
            .credential_fingerprint
            .is_none_or(|resolve| {
                let expected = resolve(&sample.venue, sample.product);
                expected.is_some() && sample.account_scope == expected
            })
    }
}

fn empty_runtime_health(venue: &str, observed_at_ms: i64) -> LiveOrderProofRuntimeHealth {
    LiveOrderProofRuntimeHealth {
        venue: venue.to_owned(),
        status: VenueOperationStatus::Unknown,
        message: "尚未取得 live 下单/撤单远程证明样本".to_owned(),
        request_id: None,
        requested: Some(LIVE_ORDER_PROOF_EXPECTED_STEPS),
        rows: Some(0),
        freshness_ms: Some(0),
        retry_after_ms: None,
        error: None,
        observed_at_ms,
        place_proof: None,
        cancel_request: None,
        cancel_finality: None,
        last_problem: None,
        place_ack_count: 0,
        cancel_requested_count: 0,
        cancel_finality_count: 0,
    }
}

fn refresh_runtime_row(row: &mut LiveOrderProofRuntimeHealth) {
    row.requested = Some(LIVE_ORDER_PROOF_EXPECTED_STEPS);
    row.rows = Some(completed_proof_rows(row));
    row.observed_at_ms = latest_observed_at(row).unwrap_or(row.observed_at_ms);
    row.request_id = latest_request_id(row);
    row.freshness_ms = Some(0);

    if problem_is_newer_than_latest_proof(row) {
        apply_problem_status(row);
        return;
    }
    if has_complete_remote_proof(row) {
        row.status = VenueOperationStatus::Ok;
        row.message = "live 下单 ack 与撤单终态远程证明均已闭环".to_owned();
        row.retry_after_ms = None;
        row.error = None;
    } else if has_any_sample(row) {
        row.status = VenueOperationStatus::Warn;
        row.message = incomplete_message(row);
        row.retry_after_ms = Some(LIVE_ORDER_PROOF_RETRY_AFTER_MS);
        row.error = Some(row.message.clone());
    } else {
        row.status = VenueOperationStatus::Unknown;
        row.message = "尚未取得 live 下单/撤单远程证明样本".to_owned();
        row.retry_after_ms = None;
        row.error = None;
    }
}

fn apply_problem_status(row: &mut LiveOrderProofRuntimeHealth) {
    let Some(problem) = row.last_problem.as_ref() else {
        return;
    };
    row.status = VenueOperationStatus::Blocked;
    row.message = format!("live 下单/撤单远程证明失败：{}", problem.message);
    row.retry_after_ms = problem.retry_after_ms;
    row.error = Some(problem.message.clone());
}
