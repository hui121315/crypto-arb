use super::{json_hash, serde_label, writer, SqlLedgerWriteError, SqlRunFinalityLedgerEvent};
use shared_types::{
    CloseRun, CloseRunCostComponent, CloseRunCostLedgerEvent, ExecutionLedgerEvent,
    ExecutionLedgerPayload, ExecutionLedgerQuality,
};
use tokio_postgres::{GenericClient, Row};

const INSERT_FACT_SQL: &str = "INSERT INTO run_cost_facts \
    (run_kind, run_id, scope, component, event_id, source_order_event_id, \
     source_run_finality_event_id, ticket_id, internal_order_id, exchange, symbol, leg_role, \
     amount, currency, amount_usd, quality, payload, payload_hash, occurred_at_ms, captured_at_ms) \
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
            $17, $18, $19, $20) \
    ON CONFLICT (run_kind, run_id, scope, component, event_id) DO NOTHING";

#[derive(Debug, Clone, PartialEq)]
pub struct SqlRunCostFact {
    pub key: SqlRunCostFactKey,
    pub source: SqlRunCostFactSource,
    pub context: SqlRunCostFactContext,
    pub value: SqlRunCostFactValue,
    pub payload: serde_json::Value,
    pub payload_hash: String,
    pub occurred_at_ms: i64,
    pub captured_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlRunCostFactKey {
    pub run_kind: String,
    pub run_id: String,
    pub scope: String,
    pub component: String,
    pub event_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqlRunCostFactSource {
    pub order_event_id: Option<String>,
    pub run_finality_event_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqlRunCostFactContext {
    pub ticket_id: Option<String>,
    pub internal_order_id: Option<String>,
    pub exchange: Option<String>,
    pub symbol: Option<String>,
    pub leg_role: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqlRunCostFactValue {
    pub amount: f64,
    pub currency: Option<String>,
    pub amount_usd: Option<f64>,
    pub quality: String,
}

pub(super) async fn project_order_event_transaction(
    client: &mut tokio_postgres::Client,
    event: &ExecutionLedgerEvent,
) -> Result<usize, writer::EventWriteError> {
    let transaction = client.transaction().await?;
    let written = upsert_order_event_facts(&transaction, event).await?;
    transaction.commit().await?;
    Ok(written)
}

pub(super) async fn upsert_order_event_facts<C>(
    client: &C,
    event: &ExecutionLedgerEvent,
) -> Result<usize, writer::EventWriteError>
where
    C: GenericClient + Sync,
{
    let facts = facts_from_order_event(event).map_err(writer::EventWriteError::Encoding)?;
    upsert_facts(client, &facts).await
}

pub(super) async fn upsert_finality_facts<C>(
    client: &C,
    event: &SqlRunFinalityLedgerEvent,
) -> Result<usize, writer::EventWriteError>
where
    C: GenericClient + Sync,
{
    let facts = facts_from_finality_event(event).map_err(writer::EventWriteError::Encoding)?;
    upsert_facts(client, &facts).await
}

pub(super) async fn query(
    url: &str,
    run_kind: &str,
    run_id: &str,
) -> Result<Vec<SqlRunCostFact>, SqlLedgerWriteError> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .map_err(SqlLedgerWriteError::from)?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::warn!(%error, "run-cost query postgres connection failed");
        }
    });
    let rows = client
        .query(
            "SELECT run_kind, run_id, scope, component, event_id, source_order_event_id, \
                    source_run_finality_event_id, ticket_id, internal_order_id, exchange, symbol, \
                    leg_role, amount, currency, amount_usd, quality, payload, payload_hash, \
                    occurred_at_ms, captured_at_ms \
             FROM run_cost_facts WHERE run_kind = $1 AND run_id = $2 \
             ORDER BY scope, component, occurred_at_ms, event_id",
            &[&run_kind, &run_id],
        )
        .await
        .map_err(SqlLedgerWriteError::from)?;
    rows.iter().map(fact_from_row).collect()
}

fn facts_from_order_event(event: &ExecutionLedgerEvent) -> Result<Vec<SqlRunCostFact>, String> {
    let Some(run_id) = event.order.run_id.as_deref() else {
        return Ok(Vec::new());
    };
    let scope = if event.order.reduce_only == Some(true) {
        "unwind"
    } else {
        "open"
    };
    let Some(value) = order_fact_value(&event.payload)? else {
        return Ok(Vec::new());
    };
    let scope = if value.component == "funding" {
        "run"
    } else {
        scope
    };
    let payload_hash = json_hash(&value.payload)?;
    Ok(vec![SqlRunCostFact {
        key: SqlRunCostFactKey {
            run_kind: "execution_run".to_owned(),
            run_id: run_id.to_owned(),
            scope: scope.to_owned(),
            component: value.component.to_owned(),
            event_id: event.event_id.clone(),
        },
        source: SqlRunCostFactSource {
            order_event_id: Some(event.event_id.clone()),
            run_finality_event_id: None,
        },
        context: SqlRunCostFactContext {
            ticket_id: event.order.ticket_id.clone(),
            internal_order_id: Some(event.order.identity.internal_order_id.clone()),
            exchange: Some(event.order.exchange.clone()),
            symbol: Some(event.order.symbol.clone()),
            leg_role: event.order.leg_role.as_ref().map(serde_label).transpose()?,
        },
        value: SqlRunCostFactValue {
            amount: value.amount,
            currency: value.currency,
            amount_usd: value.amount_usd,
            quality: value.quality,
        },
        payload: value.payload,
        payload_hash,
        occurred_at_ms: event.occurred_at_ms,
        captured_at_ms: event.captured_at_ms,
    }])
}

struct OrderFactValue {
    component: &'static str,
    amount: f64,
    currency: Option<String>,
    amount_usd: Option<f64>,
    quality: String,
    payload: serde_json::Value,
}

fn order_fact_value(payload: &ExecutionLedgerPayload) -> Result<Option<OrderFactValue>, String> {
    let value = match payload {
        ExecutionLedgerPayload::FillSnapshot(fill) => {
            let Some(fee) = fill.fee.as_ref() else {
                return Ok(None);
            };
            if fee.quality != ExecutionLedgerQuality::Actual || !fee.amount.is_finite() {
                return Ok(None);
            }
            fee_fact_value(fee)?
        }
        ExecutionLedgerPayload::FeeSnapshot(fee) => {
            if fee.quality != ExecutionLedgerQuality::Actual || !fee.amount.is_finite() {
                return Ok(None);
            }
            fee_fact_value(fee)?
        }
        ExecutionLedgerPayload::Slippage(slippage) => {
            if slippage.quality != ExecutionLedgerQuality::Actual
                || !slippage.amount_usd.is_finite()
            {
                return Ok(None);
            }
            OrderFactValue {
                component: "slippage",
                amount: slippage.amount_usd,
                currency: Some("USD".to_owned()),
                amount_usd: Some(slippage.amount_usd),
                quality: serde_label(&slippage.quality)?,
                payload: serde_json::to_value(slippage).map_err(|error| error.to_string())?,
            }
        }
        ExecutionLedgerPayload::FundingPayment(payment) => {
            if payment.quality != ExecutionLedgerQuality::Actual || !payment.amount.is_finite() {
                return Ok(None);
            }
            OrderFactValue {
                component: "funding",
                amount: payment.amount,
                currency: Some(payment.currency.clone()),
                amount_usd: usd_amount(payment.amount, Some(&payment.currency)),
                quality: serde_label(&payment.quality)?,
                payload: serde_json::to_value(payment).map_err(|error| error.to_string())?,
            }
        }
        ExecutionLedgerPayload::OrderState { .. }
        | ExecutionLedgerPayload::OrderbookEvidence(_) => return Ok(None),
    };
    Ok(Some(value))
}

fn fee_fact_value(fee: &shared_types::FeeLedgerSnapshot) -> Result<OrderFactValue, String> {
    Ok(OrderFactValue {
        component: "fee",
        amount: fee.amount,
        currency: fee.currency.clone(),
        amount_usd: usd_amount(fee.amount, fee.currency.as_deref()),
        quality: serde_label(&fee.quality)?,
        payload: serde_json::to_value(fee).map_err(|error| error.to_string())?,
    })
}

fn facts_from_finality_event(
    finality: &SqlRunFinalityLedgerEvent,
) -> Result<Vec<SqlRunCostFact>, String> {
    if finality.run_kind != "close_run" {
        return Ok(Vec::new());
    }
    let run = serde_json::from_value::<CloseRun>(finality.payload.clone())
        .map_err(|error| format!("close-run cost payload decode failed: {error}"))?;
    let mut facts = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for leg in &run.legs {
        let context = SqlRunCostFactContext {
            ticket_id: leg
                .pair_evidence
                .as_ref()
                .map(|pair| pair.ticket_id.clone()),
            internal_order_id: leg.order.as_ref().map(|order| order.intent.id.clone()),
            exchange: Some(leg.venue.clone()),
            symbol: Some(leg.symbol.clone()),
            leg_role: Some(serde_label(&leg.side)?),
        };
        push_close_cost_facts(
            &mut facts,
            &mut seen,
            &CloseFactBatch {
                run: &run,
                finality,
                scope: "close",
                events: &leg.cost_events,
                context: &context,
            },
        )?;
    }
    if let Some(plan) = &run.unwind_plan {
        for attempt in &plan.compensation_attempts {
            let context = SqlRunCostFactContext {
                ticket_id: None,
                internal_order_id: attempt.order.as_ref().map(|order| order.intent.id.clone()),
                exchange: Some(attempt.venue.clone()),
                symbol: Some(attempt.symbol.clone()),
                leg_role: Some(serde_label(&attempt.side)?),
            };
            push_close_cost_facts(
                &mut facts,
                &mut seen,
                &CloseFactBatch {
                    run: &run,
                    finality,
                    scope: "compensation",
                    events: &attempt.cost_events,
                    context: &context,
                },
            )?;
        }
    }
    for event in &run.cost_events {
        if event.component == CloseRunCostComponent::ManualHandling {
            if finality.source != "manual" {
                continue;
            }
            validate_manual_cost_source(&run, event)?;
        }
        let scope = match event.component {
            CloseRunCostComponent::Funding | CloseRunCostComponent::ManualHandling => "run",
            CloseRunCostComponent::Fee | CloseRunCostComponent::Slippage => "close",
        };
        push_close_cost_facts(
            &mut facts,
            &mut seen,
            &CloseFactBatch {
                run: &run,
                finality,
                scope,
                events: std::slice::from_ref(event),
                context: &SqlRunCostFactContext::default(),
            },
        )?;
    }
    Ok(facts)
}

struct CloseFactBatch<'a> {
    run: &'a CloseRun,
    finality: &'a SqlRunFinalityLedgerEvent,
    scope: &'a str,
    events: &'a [CloseRunCostLedgerEvent],
    context: &'a SqlRunCostFactContext,
}

fn push_close_cost_facts(
    facts: &mut Vec<SqlRunCostFact>,
    seen: &mut std::collections::BTreeSet<(String, String)>,
    batch: &CloseFactBatch<'_>,
) -> Result<(), String> {
    for event in batch.events {
        if event.quality != ExecutionLedgerQuality::Actual || !event.amount_usd.is_finite() {
            continue;
        }
        let component = serde_label(&event.component)?;
        if !seen.insert((component.clone(), event.event_id.clone())) {
            continue;
        }
        let payload = serde_json::to_value(event).map_err(|error| error.to_string())?;
        let payload_hash = json_hash(&payload)?;
        facts.push(SqlRunCostFact {
            key: SqlRunCostFactKey {
                run_kind: "close_run".to_owned(),
                run_id: batch.run.id.clone(),
                scope: batch.scope.to_owned(),
                component,
                event_id: event.event_id.clone(),
            },
            source: SqlRunCostFactSource {
                order_event_id: None,
                run_finality_event_id: Some(batch.finality.event_id.clone()),
            },
            context: batch.context.clone(),
            value: SqlRunCostFactValue {
                amount: event.amount_usd,
                currency: Some("USD".to_owned()),
                amount_usd: Some(event.amount_usd),
                quality: serde_label(&event.quality)?,
            },
            payload,
            payload_hash,
            occurred_at_ms: event.occurred_at_ms,
            captured_at_ms: event.captured_at_ms,
        });
    }
    Ok(())
}

async fn upsert_facts<C>(
    client: &C,
    facts: &[SqlRunCostFact],
) -> Result<usize, writer::EventWriteError>
where
    C: GenericClient + Sync,
{
    let mut written = 0usize;
    for fact in facts {
        let inserted = client
            .execute(
                INSERT_FACT_SQL,
                &[
                    &fact.key.run_kind,
                    &fact.key.run_id,
                    &fact.key.scope,
                    &fact.key.component,
                    &fact.key.event_id,
                    &fact.source.order_event_id,
                    &fact.source.run_finality_event_id,
                    &fact.context.ticket_id,
                    &fact.context.internal_order_id,
                    &fact.context.exchange,
                    &fact.context.symbol,
                    &fact.context.leg_role,
                    &fact.value.amount,
                    &fact.value.currency,
                    &fact.value.amount_usd,
                    &fact.value.quality,
                    &fact.payload,
                    &fact.payload_hash,
                    &fact.occurred_at_ms,
                    &fact.captured_at_ms,
                ],
            )
            .await?;
        if inserted == 1 {
            written = written.saturating_add(1);
            continue;
        }
        let existing = read_fact(client, &fact.key).await?;
        if !same_immutable_fact(&existing, fact) {
            return Err(writer::EventWriteError::IntegrityConflict {
                event_id: fact.key.event_id.clone(),
            });
        }
    }
    Ok(written)
}

fn validate_manual_cost_source(
    run: &CloseRun,
    event: &CloseRunCostLedgerEvent,
) -> Result<(), String> {
    let evidence = run
        .unwind_plan
        .as_ref()
        .and_then(|plan| plan.manual_terminal_evidence.as_ref())
        .ok_or_else(|| "manual run-cost fact lacks terminal evidence".to_owned())?;
    if evidence.manual_handling_event_id.as_deref() != Some(event.event_id.as_str())
        || evidence.manual_handling_cost_usd != Some(event.amount_usd)
        || event.source != shared_types::OrderUpdateSource::Manual
    {
        return Err("manual run-cost fact does not match terminal evidence".to_owned());
    }
    Ok(())
}

fn same_immutable_fact(existing: &SqlRunCostFact, requested: &SqlRunCostFact) -> bool {
    if existing == requested {
        return true;
    }
    let finality_snapshot_only_differs = existing.source.order_event_id.is_none()
        && requested.source.order_event_id.is_none()
        && existing.source.run_finality_event_id.is_some()
        && requested.source.run_finality_event_id.is_some();
    finality_snapshot_only_differs
        && existing.key == requested.key
        && existing.context == requested.context
        && existing.value == requested.value
        && existing.payload == requested.payload
        && existing.payload_hash == requested.payload_hash
        && existing.occurred_at_ms == requested.occurred_at_ms
        && existing.captured_at_ms == requested.captured_at_ms
}

async fn read_fact<C>(
    client: &C,
    key: &SqlRunCostFactKey,
) -> Result<SqlRunCostFact, writer::EventWriteError>
where
    C: GenericClient + Sync,
{
    let row = client
        .query_one(
            "SELECT run_kind, run_id, scope, component, event_id, source_order_event_id, \
                    source_run_finality_event_id, ticket_id, internal_order_id, exchange, symbol, \
                    leg_role, amount, currency, amount_usd, quality, payload, payload_hash, \
                    occurred_at_ms, captured_at_ms \
             FROM run_cost_facts \
             WHERE run_kind = $1 AND run_id = $2 AND scope = $3 AND component = $4 AND event_id = $5",
            &[
                &key.run_kind,
                &key.run_id,
                &key.scope,
                &key.component,
                &key.event_id,
            ],
        )
        .await?;
    fact_from_row(&row).map_err(|error| writer::EventWriteError::Encoding(error.to_string()))
}

fn fact_from_row(row: &Row) -> Result<SqlRunCostFact, SqlLedgerWriteError> {
    Ok(SqlRunCostFact {
        key: SqlRunCostFactKey {
            run_kind: row.try_get("run_kind").map_err(SqlLedgerWriteError::from)?,
            run_id: row.try_get("run_id").map_err(SqlLedgerWriteError::from)?,
            scope: row.try_get("scope").map_err(SqlLedgerWriteError::from)?,
            component: row
                .try_get("component")
                .map_err(SqlLedgerWriteError::from)?,
            event_id: row.try_get("event_id").map_err(SqlLedgerWriteError::from)?,
        },
        source: SqlRunCostFactSource {
            order_event_id: row
                .try_get("source_order_event_id")
                .map_err(SqlLedgerWriteError::from)?,
            run_finality_event_id: row
                .try_get("source_run_finality_event_id")
                .map_err(SqlLedgerWriteError::from)?,
        },
        context: SqlRunCostFactContext {
            ticket_id: row
                .try_get("ticket_id")
                .map_err(SqlLedgerWriteError::from)?,
            internal_order_id: row
                .try_get("internal_order_id")
                .map_err(SqlLedgerWriteError::from)?,
            exchange: row.try_get("exchange").map_err(SqlLedgerWriteError::from)?,
            symbol: row.try_get("symbol").map_err(SqlLedgerWriteError::from)?,
            leg_role: row.try_get("leg_role").map_err(SqlLedgerWriteError::from)?,
        },
        value: SqlRunCostFactValue {
            amount: row.try_get("amount").map_err(SqlLedgerWriteError::from)?,
            currency: row.try_get("currency").map_err(SqlLedgerWriteError::from)?,
            amount_usd: row
                .try_get("amount_usd")
                .map_err(SqlLedgerWriteError::from)?,
            quality: row.try_get("quality").map_err(SqlLedgerWriteError::from)?,
        },
        payload: row.try_get("payload").map_err(SqlLedgerWriteError::from)?,
        payload_hash: row
            .try_get("payload_hash")
            .map_err(SqlLedgerWriteError::from)?,
        occurred_at_ms: row
            .try_get("occurred_at_ms")
            .map_err(SqlLedgerWriteError::from)?,
        captured_at_ms: row
            .try_get("captured_at_ms")
            .map_err(SqlLedgerWriteError::from)?,
    })
}

fn usd_amount(amount: f64, currency: Option<&str>) -> Option<f64> {
    currency
        .filter(|currency| is_usd_settlement_currency(currency))
        .map(|_| amount)
}

fn is_usd_settlement_currency(currency: &str) -> bool {
    shared_types::is_usd_pegged_settlement_currency(currency)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_stablecoin_fee_is_normalized_without_absolute_value() {
        let event = fixture_event(ExecutionLedgerPayload::FeeSnapshot(
            shared_types::FeeLedgerSnapshot {
                amount: -0.25,
                currency: Some("USDC".to_owned()),
                quality: ExecutionLedgerQuality::Actual,
            },
        ));
        let facts = facts_from_order_event(&event).expect("fee fact");

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key.scope, "open");
        assert_eq!(facts[0].value.amount, -0.25);
        assert_eq!(facts[0].value.amount_usd, Some(-0.25));
    }

    #[test]
    fn non_usd_funding_keeps_native_amount_without_fabricated_usd() {
        let event = fixture_event(ExecutionLedgerPayload::FundingPayment(
            shared_types::FundingPaymentLedgerRecord {
                amount: 0.5,
                currency: "BTC".to_owned(),
                funding_time_ms: 9,
                quality: ExecutionLedgerQuality::Actual,
            },
        ));
        let facts = facts_from_order_event(&event).expect("funding fact");

        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key.scope, "run");
        assert_eq!(facts[0].value.amount_usd, None);
    }

    #[test]
    fn repeated_finality_snapshot_keeps_first_identical_fact_source() {
        let payload = serde_json::json!({"eventId": "cost-1"});
        let fact = SqlRunCostFact {
            key: SqlRunCostFactKey {
                run_kind: "close_run".to_owned(),
                run_id: "close-1".to_owned(),
                scope: "close".to_owned(),
                component: "fee".to_owned(),
                event_id: "cost-1".to_owned(),
            },
            source: SqlRunCostFactSource {
                order_event_id: None,
                run_finality_event_id: Some("finality-1".to_owned()),
            },
            context: SqlRunCostFactContext::default(),
            value: SqlRunCostFactValue {
                amount: -0.1,
                currency: Some("USD".to_owned()),
                amount_usd: Some(-0.1),
                quality: "actual".to_owned(),
            },
            payload_hash: json_hash(&payload).expect("payload hash"),
            payload,
            occurred_at_ms: 1,
            captured_at_ms: 2,
        };
        let mut later = fact.clone();
        later.source.run_finality_event_id = Some("finality-2".to_owned());

        assert!(same_immutable_fact(&fact, &later));
    }

    fn fixture_event(payload: ExecutionLedgerPayload) -> ExecutionLedgerEvent {
        ExecutionLedgerEvent {
            event_id: "cost-event-1".to_owned(),
            event_type: shared_types::ExecutionLedgerEventType::FeeSnapshot,
            source: shared_types::OrderUpdateSource::PrivateWs,
            order: shared_types::ExecutionLedgerOrderRef {
                run_id: Some("run-1".to_owned()),
                ticket_id: Some("ticket-1".to_owned()),
                leg_role: Some(shared_types::HedgeLegRole::Long),
                reduce_only: Some(false),
                exchange: "okx".to_owned(),
                symbol: "BTC-USDT".to_owned(),
                side: shared_types::OrderSide::Buy,
                identity: shared_types::VenueOrderIdentity {
                    internal_order_id: "order-1".to_owned(),
                    public_client_order_id: "client-1".to_owned(),
                    product: shared_types::FeeProduct::Perp,
                    venue_client_order_id: None,
                    exchange_order_id: Some("exchange-1".to_owned()),
                    client_order_id_policy: None,
                    transport_metadata: Default::default(),
                },
            },
            payload,
            occurred_at_ms: 9,
            captured_at_ms: 10,
        }
    }
}
