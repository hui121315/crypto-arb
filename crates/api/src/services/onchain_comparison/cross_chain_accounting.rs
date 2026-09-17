use crate::services::onchain_cross_chain_run_store::accounting;
use crate::state::AppState;
use shared_types::{
    OnchainCrossChainRun, OnchainExecutionAccountingStatus as Status, WebhookEventKind,
};
use std::collections::BTreeSet;

pub(crate) async fn refresh_pending(state: &AppState, now_ms: i64) {
    if state.onchain_cross_chain_runs().readiness().is_err() {
        return;
    }
    let mut rows = state.onchain_cross_chain_runs().runs(128, now_ms).rows;
    // Valuation and outbox writes are separate. Resume an unqueued notification after a crash.
    rows.retain(|run| {
        run.accounting_refresh_due(now_ms)
            || notification_due(run, state.webhook().event_known(&event_id(run)))
    });
    rows.sort_by_key(|run| run.updated_at_ms);
    if !rows.is_empty() {
        let offset = now_ms.div_euclid(5_000).rem_euclid(rows.len() as i64) as usize;
        rows.rotate_left(offset);
    }
    rows.truncate(4);
    let mut needed = BTreeSet::new();
    for row in &rows {
        if let Some(a) = row
            .accounting
            .as_ref()
            .filter(|a| a.status == Status::PendingValuation)
        {
            if let Ok(assets) = accounting::valuation_symbols(a) {
                needed.extend(assets);
            }
        }
    }
    let mut config = state.onchain_monitor().snapshot().config.clone();
    config.cex_venue = "kraken".into();
    config.max_age_ms = config.max_age_ms.clamp(1, 30_000);
    let assets = needed.into_iter().take(16).collect::<Vec<_>>();
    if !assets.is_empty() {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            super::usd_valuation::refresh_assets(state, &config, &assets),
        )
        .await;
    }
    for run in rows {
        let Some(a) = &run.accounting else {
            continue;
        };
        if a.status == Status::Valued {
            emit(state, &run).await;
            continue;
        }
        let now_ms = common::time::now_ms();
        let result = accounting::valuation_symbols(a)
            .and_then(|assets| {
                assets
                    .iter()
                    .map(|asset| super::usd_valuation::evidence(state, &config, asset, now_ms))
                    .collect::<Result<Vec<_>, _>>()
            })
            .and_then(|rates| {
                state.onchain_cross_chain_runs().record_accounting_value(
                    &run.run_id,
                    &a.flows,
                    rates,
                    now_ms,
                )
            });
        match result {
            Ok(updated)
                if updated
                    .accounting
                    .as_ref()
                    .is_some_and(|a| a.status == Status::Valued) =>
            {
                emit(state, &updated).await
            }
            Ok(_) => (),
            Err(problem) => {
                let _ = state.onchain_cross_chain_runs().record_accounting_problem(
                    &run.run_id,
                    &a.flows,
                    problem,
                    now_ms,
                );
            }
        }
    }
}

async fn emit(state: &AppState, run: &OnchainCrossChainRun) {
    let id = event_id(run);
    let payload = notification_payload(run);
    if let Err(problem) = crate::services::webhook::emit_idempotent(
        state,
        WebhookEventKind::ExecutionResult,
        id,
        payload,
    )
    .await
    {
        tracing::warn!(%problem, run_id = %run.run_id, "cross-chain accounting webhook enqueue failed");
    }
}

fn notification_payload(run: &OnchainCrossChainRun) -> serde_json::Value {
    let value = run
        .accounting
        .as_ref()
        .and_then(|a| a.usd_value.as_ref())
        .map_or("待核算", |v| v.net_usd_exact.as_str());
    let changes = run
        .accounting
        .iter()
        .flat_map(|a| &a.net_assets)
        .take(8)
        .map(|row| format!("{} · {} {}", row.chain, row.amount_exact, row.asset.symbol))
        .collect::<Vec<_>>()
        .join("\n");
    let scope = format!("四步回执与已选独立费用：授权 {} 笔、补库 {} 笔。未选费用未包含，美元为折算，不代表完整交易利润。", run.build.approval_costs.len(), run.build.replenishment_costs.len());
    serde_json::json!({
        "title":"CROSSLINE 跨链收支核算完成", "runId":run.run_id, "buildId":run.build.build_id,
        "accounting":run.accounting, "scope":scope,
        "message":format!("跨链四步收支已核算\n已记录收支折合 {value} USD\n{changes}\n{scope}\n记录：{}", run.run_id)
    })
}

fn event_id(run: &OnchainCrossChainRun) -> String {
    format!("{}:cross-chain:accounting:valued", run.run_id)
}

fn notification_due(run: &OnchainCrossChainRun, known: bool) -> bool {
    !known
        && run.status == shared_types::OnchainCrossChainRunStatus::Completed
        && run
            .accounting
            .as_ref()
            .is_some_and(|a| a.status == Status::Valued && a.usd_value.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_chain_accounting_notification_recovers_without_revaluing_or_repeating() {
        let mut run: OnchainCrossChainRun = serde_json::from_value(serde_json::json!({
            "runId":"queued-once", "idempotencyKey":"fixture", "status":"completed",
            "authorization":{"actor":"fixture","authorizedAtMs":1,"validUntilMs":2,"confirmationVersion":"test"},
            "createdAtMs":1,"updatedAtMs":2,"nextAction":"done",
            "build":{"buildId":"fixture","provider":"lifi","sourceChain":"ethereum","peerChain":"base","legs":[],
                "initialQuoteAmountRaw":"1","finalQuoteAmountRaw":"1","atomic":false,"monitorOnly":false,"previewReady":true,"submitReady":true,
                "quoteObservedAtMs":1,"builtAtMs":1,"validUntilMs":2},
            "accounting":{"status":"valued","flows":[],"netAssets":[],"problems":[],
                "usdValue":{"netUsdExact":"-2.123456","valuedAtMs":3,"rates":[]}}
        })).unwrap();
        assert!(!run.accounting_refresh_due(9_000_000));
        assert!(notification_due(&run, false));
        assert!(!notification_due(&run, true));
        let id = event_id(&run);
        let payload = notification_payload(&run);
        let message = payload["message"].as_str().unwrap();
        assert!(message.contains("-2.123456 USD"));
        assert!(message.contains("不代表完整交易利润"));
        run.updated_at_ms = 10_000;
        assert_eq!(id, event_id(&run));
        run.accounting.as_mut().unwrap().usd_value = None;
        assert!(!notification_due(&run, false));
        run.accounting.as_mut().unwrap().status = Status::PendingValuation;
        assert!(!notification_due(&run, false));
    }
}
