//! In-memory `ActionRun` ledger for high-risk user mutations.

use crate::middleware::audit::{self, AuditEvent};
use crate::state::AppState;
use axum::http::{HeaderMap, StatusCode};
use common::AppError;
use dashmap::mapref::entry::Entry;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;
use shared_types::{
    problem::codes, ActionMutationDiff, ActionRun, ActionRunKind, ActionRunStatus, ApiProblem,
};
use std::any::type_name;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

const MAX_ACTION_RUNS: usize = 512;
const RECENT_ACTION_RUNS: usize = 100;

mod audit_context;
mod audit_log;
mod audit_summary;
mod auth_denial;
mod lifecycle;
mod mutate;
#[cfg(test)]
mod tests;

pub(crate) use auth_denial::record_auth_denial;
#[cfg(test)]
pub(crate) use lifecycle::recent;
pub(crate) use lifecycle::{
    begin, begin_idempotent, explicit_idempotency_key, get, recent_envelope, ActionRunBegin,
    ActionRunStart,
};
#[cfg(test)]
pub(crate) use mutate::finish_status;
pub(crate) use mutate::{
    fail_response, finish_result_with_payload, finish_result_with_payload_and_mutation,
    finish_status_with_payload, replay_payload,
};
