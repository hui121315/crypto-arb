use super::connection::ReviewConnection;
use crate::api::rest::ApiError;
use crate::state::read_scope::{bounded_read, ReadScope, ScopedRead};
use leptos::prelude::*;
use shared_types::{problem::codes, ApiProblem, ReviewEnvelope};
use std::future::Future;

pub(super) fn review_envelope_result<T>(
    envelope: ReviewEnvelope<T>,
) -> Result<ReviewEnvelope<T>, ApiProblem> {
    if let Some(problem) = envelope
        .problems
        .iter()
        .find(|problem| problem.code == codes::REVIEW_HISTORY_READ_FAILED)
    {
        return Err(problem.clone());
    }
    Ok(envelope)
}

pub(in crate::panels::modules::review) fn review_request(
    connection: ReviewConnection,
) -> ScopedRead {
    let scope = ReadScope::new(|| {});
    let request = scope.request();
    Effect::new(move |_| {
        scope.track();
        if !connection.available() {
            request.cancel();
        }
    });
    request
}

pub(in crate::panels::modules::review) async fn review_read<T>(
    read: impl Future<Output = Result<T, ApiError>>,
) -> Result<T, ApiProblem> {
    bounded_read(read).await.map_err(|problem| {
        if problem.code == "SHARED_READ_TIMEOUT" {
            ApiProblem::new(
                "REVIEW_READ_TIMEOUT",
                "读取复盘记录超过 15 秒未返回，已停止等待；保留已读取的记录，请重试",
            )
            .with_source("frontend.review")
        } else {
            problem
        }
    })
}
