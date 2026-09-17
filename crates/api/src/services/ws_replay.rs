use crate::{
    services::{execution_runs, market_data, opportunity, risk_config},
    state::AppState,
};
use realtime::channels::{self, WsChannelReplaySource};
use serde_json::Value;
use shared_types::{ExecutionRunEvent, OrderStreamRecordEvent, RiskAlertEvent};

const ORDERS_REPLAY_LIMIT: usize = 50;

pub(crate) struct ReplayPayload {
    pub(crate) channel: &'static str,
    pub(crate) payload: Value,
}

pub(crate) async fn payloads_for_channel(
    channel: &str,
    state: &AppState,
) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let Some(spec) = channels::ws_channel_spec(channel) else {
        return Ok(Vec::new());
    };
    match spec.replay_source {
        WsChannelReplaySource::OpportunitySnapshot => Ok(vec![ReplayPayload {
            channel: channels::ARBITRAGE,
            payload: serde_json::to_value(latest_arbitrage_event(state))?,
        }]),
        WsChannelReplaySource::WatchlistSnapshot => watchlist_payloads(state).await,
        WsChannelReplaySource::FundingRatesSnapshot => funding_rate_payloads(state),
        WsChannelReplaySource::AlertRulesSnapshot => alert_rule_payloads(state).await,
        WsChannelReplaySource::OrderSnapshot => order_payloads(state),
        WsChannelReplaySource::ExecutionRunSnapshot => execution_payloads(state),
        WsChannelReplaySource::PortfolioSnapshot => portfolio_payloads(state),
        WsChannelReplaySource::RiskSnapshot => risk_alert_payloads(state),
        WsChannelReplaySource::SystemHealthSnapshot => system_payloads(state),
        WsChannelReplaySource::AutomationStatusSnapshot => automation_payloads(state),
        WsChannelReplaySource::OnchainComparisonSnapshot => onchain_payloads(state),
        WsChannelReplaySource::StockMarketSnapshot => Ok(vec![ReplayPayload {
            channel: channels::STOCKS,
            payload: serde_json::to_value(state.backpack_stocks().snapshot())?,
        }]),
        WsChannelReplaySource::ReviewRuntimeSnapshot => review_payloads(state),
        WsChannelReplaySource::WebhookRuntimeStatusSnapshot => webhook_payloads(state).await,
    }
}

/// 订阅即拿到完整 webhook 运行态，前端无需再为首包发一次 REST。
async fn webhook_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    Ok(vec![ReplayPayload {
        channel: channels::WEBHOOK,
        payload: serde_json::to_value(crate::services::webhook::status(state).await)?,
    }])
}

fn review_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let Some(entry) = state.review_snapshot().get_arc_now() else {
        return Ok(Vec::new());
    };
    Ok(vec![ReplayPayload {
        channel: channels::REVIEW,
        payload: serde_json::to_value(&entry.value)?,
    }])
}

fn onchain_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    Ok(vec![ReplayPayload {
        channel: channels::ONCHAIN,
        payload: serde_json::to_value(state.onchain_monitor().snapshot().as_ref())?,
    }])
}

fn automation_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    Ok(vec![ReplayPayload {
        channel: channels::AUTOMATION,
        payload: serde_json::to_value(state.automation().snapshot().as_ref())?,
    }])
}

fn funding_rate_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let projection = state.market_data().funding_rows_snapshot_with_evidence();
    let payload = market_data::envelope::funding_rates_envelope(
        projection.rows,
        market_data::MarketSource::LocalCache,
        common::time::now_ms(),
        &state.market_data().runtime_health_snapshot(),
        projection.row_evidence,
    );
    Ok(vec![ReplayPayload {
        channel: channels::FUNDING_RATES,
        payload: serde_json::to_value(payload)?,
    }])
}

async fn watchlist_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let event = shared_types::WatchlistStreamEvent::WatchlistChanged {
        envelope: crate::services::watchlist_alerts::watchlist_envelope(
            state,
            state.watchlist().read().await.clone(),
        ),
        timestamp_ms: common::time::now_ms(),
    };
    Ok(vec![ReplayPayload {
        channel: channels::WATCHLIST,
        payload: serde_json::to_value(event)?,
    }])
}

async fn alert_rule_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let event = shared_types::AlertStreamEvent::AlertRulesChanged {
        envelope: Box::new(crate::services::watchlist_alerts::alert_rules_envelope(
            state,
            state.alert_rules().read().await.clone(),
        )),
        timestamp_ms: common::time::now_ms(),
    };
    Ok(vec![ReplayPayload {
        channel: channels::ALERTS,
        payload: serde_json::to_value(event)?,
    }])
}

fn system_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let Some(health) = state.system_health_snapshot().value_now() else {
        return Ok(Vec::new());
    };
    Ok(vec![ReplayPayload {
        channel: channels::SYSTEM,
        payload: serde_json::to_value(health)?,
    }])
}

fn portfolio_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let Some(entry) = state.portfolio_snapshot_envelope().get_arc_now() else {
        return Ok(Vec::new());
    };
    Ok(vec![ReplayPayload {
        channel: channels::PORTFOLIO,
        payload: serde_json::to_value(&entry.value)?,
    }])
}

fn order_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let (records, _) = state
        .trading_service()
        .list_orders_page(0, ORDERS_REPLAY_LIMIT, None, None);
    let timestamp_ms = common::time::now_ms();
    records
        .into_iter()
        .map(|record| {
            Ok(ReplayPayload {
                channel: channels::ORDERS,
                payload: serde_json::to_value(OrderStreamRecordEvent {
                    event: "order_snapshot_replay".to_owned(),
                    record,
                    timestamp_ms,
                })?,
            })
        })
        .collect()
}

fn execution_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    execution_runs::recent(state)
        .into_iter()
        .map(|run| {
            Ok(ReplayPayload {
                channel: channels::EXECUTION,
                payload: serde_json::to_value(ExecutionRunEvent {
                    event: "execution_run_updated".to_owned(),
                    execution_run: Some(run),
                    timestamp_ms: common::time::now_ms(),
                })?,
            })
        })
        .collect()
}

fn risk_alert_payloads(state: &AppState) -> Result<Vec<ReplayPayload>, serde_json::Error> {
    let risk = risk_config::snapshot(&state.trading_service().risk_config());
    Ok(vec![ReplayPayload {
        channel: channels::RISK_ALERTS,
        payload: serde_json::to_value(RiskAlertEvent {
            event: "risk_snapshot_replay".to_owned(),
            risk: Some(risk),
            execution_run: None,
            timestamp_ms: common::time::now_ms(),
        })?,
    }])
}

fn latest_arbitrage_event(state: &AppState) -> shared_types::OpportunityStreamEvent {
    if let Some(snapshot) = state.opportunity_index().read() {
        return opportunity::stream_event(opportunity::OpportunityStreamEventInput {
            source_rows: snapshot.rows(),
            meta: snapshot.report().meta.clone(),
            cached_at: snapshot.cached_at(),
            snapshot_id: Some(snapshot.snapshot_id()),
            source: "snapshot-replay",
            status: shared_types::OpportunityEnvelopeStatus::Fresh,
            scope: shared_types::OpportunityEnvelopeScope::MainP0,
            query_key: main_p0_replay_query_key(),
            retry_after_ms: None,
            error: None,
            full_window_rows: true,
        });
    }

    let error = opportunity::warming_error();
    opportunity::stream_event(opportunity::OpportunityStreamEventInput {
        source_rows: &[],
        meta: shared_types::OpportunityScanMeta::default(),
        cached_at: chrono::Utc::now(),
        snapshot_id: None,
        source: "warming",
        status: shared_types::OpportunityEnvelopeStatus::Warming,
        scope: shared_types::OpportunityEnvelopeScope::MainP0,
        query_key: main_p0_replay_query_key(),
        retry_after_ms: error.retry_after_ms,
        error: Some(error),
        full_window_rows: true,
    })
}

fn main_p0_replay_query_key() -> String {
    "scope=main_p0;strategy=perp_cross,perp_price_spread,spot_perp,cross_spot_perp,spot_cross;symbol=*;minYield=*;limit=*;fast=false;fresh=false".into()
}

#[cfg(test)]
mod tests;
