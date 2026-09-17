#![allow(clippy::panic)]

use super::*;
use common::config::AppConfig;

mod lifecycle;
mod payload;
mod terminal_contract;

fn assert_replay_reason(error: &AppError, reason: &str) {
    let AppError::Domain {
        details: Some(details),
        ..
    } = error
    else {
        panic!("missing replay details");
    };
    assert_eq!(
        details.get("reason").and_then(|value| value.as_str()),
        Some(reason)
    );
}

async fn test_state() -> AppState {
    let mut config = AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    match AppState::new(config).await {
        Ok(state) => state,
        Err(error) => panic!("state init failed: {error}"),
    }
}

fn position_of(ids: &[String], needle: &str) -> usize {
    match ids.iter().position(|id| id == needle) {
        Some(position) => position,
        None => panic!("missing action run id {needle}"),
    }
}
