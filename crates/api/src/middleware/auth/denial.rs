use crate::services::action_runs;
use axum::extract::Request;
use axum::response::Response;
use common::AppError;

pub(super) fn reject(request: &Request, error: AppError) -> Result<Response, AppError> {
    let problem = error.to_api_problem();
    action_runs::record_auth_denial(request.method(), request.uri().path(), &problem)?;
    Err(error)
}
