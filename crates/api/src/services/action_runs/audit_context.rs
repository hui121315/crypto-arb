use super::audit_summary::action_result_summary;
use super::*;
use crate::middleware::audit::{AuditEventContext, AuditResourceKind};

pub(super) fn action_event_context(run: &ActionRun) -> AuditEventContext {
    let mut context = AuditEventContext::for_actor(run.actor.as_str());
    context.action_kind = Some(run.kind);
    context.resource_kind = Some(resource_kind(run.kind));
    context.status = Some(action_status(run));
    context.problem_code = run
        .problem
        .as_ref()
        .map(|problem| problem.code.trim().to_owned())
        .filter(|code| !code.is_empty());
    apply_canonical_route(&mut context, run.kind);
    apply_target(&mut context, run);
    if let Some(summary) = action_result_summary(run).as_ref() {
        apply_result_summary(&mut context, run.kind, summary);
        apply_nested_problem(&mut context, summary);
    }
    context
}

fn apply_canonical_route(context: &mut AuditEventContext, kind: ActionRunKind) {
    if kind == ActionRunKind::AutomationLiveUnlock {
        // Preserve source metadata for stored records after retiring the route.
        context.method = Some("POST".to_owned());
        context.path = Some("/api/automation/live-unlock".to_owned());
        return;
    }
    let endpoint = crate::route_specs::route_specs()
        .iter()
        .copied()
        .flat_map(|spec| spec.endpoints().iter().copied())
        .find(|endpoint| endpoint.action_run_kind() == Some(kind));
    if let Some(endpoint) = endpoint {
        context.method = Some(endpoint.methods().to_owned());
        context.path = Some(endpoint.path().to_owned());
    }
}

fn action_status(run: &ActionRun) -> u16 {
    run.problem
        .as_ref()
        .and_then(|problem| problem.status)
        .unwrap_or(match run.status {
            ActionRunStatus::Accepted => 202,
            ActionRunStatus::Succeeded => 200,
            ActionRunStatus::Failed => 400,
        })
}

pub(super) const fn resource_kind(kind: ActionRunKind) -> AuditResourceKind {
    match kind {
        ActionRunKind::TradingRiskConfigUpdate => AuditResourceKind::RiskConfiguration,
        ActionRunKind::TradingAdapterSelect => AuditResourceKind::TradingAdapter,
        ActionRunKind::TradingKillSwitch => AuditResourceKind::KillSwitch,
        ActionRunKind::TradingFeeSnapshotUpsert => AuditResourceKind::FeeSnapshot,
        ActionRunKind::TradingOrderSubmit | ActionRunKind::TradingOrderCancel => {
            AuditResourceKind::Order
        }
        ActionRunKind::TradingOrderReconcile => AuditResourceKind::OrderSet,
        ActionRunKind::AutomationConfigUpdate => AuditResourceKind::AutomationConfiguration,
        ActionRunKind::AutomationControl => AuditResourceKind::AutomationControl,
        ActionRunKind::AutomationLiveUnlock => AuditResourceKind::AutomationLiveUnlock,
        ActionRunKind::HedgeConfirm => AuditResourceKind::HedgeTicket,
        ActionRunKind::StockPlanBuild | ActionRunKind::StockPeerPlanBuild => AuditResourceKind::StockExecutionPlan,
        ActionRunKind::WebhookConfigUpdate | ActionRunKind::WebhookTest => AuditResourceKind::WebhookConfiguration,
        ActionRunKind::MarketSubscriptionsUpdate | ActionRunKind::GateCrossExModeUpdate
        | ActionRunKind::StockBatchUpdate | ActionRunKind::StockMonitorUpdate
        | ActionRunKind::OnchainComparisonConfigUpdate | ActionRunKind::OnchainBatchAdd
        | ActionRunKind::OnchainBatchRemove => {
            AuditResourceKind::MarketSubscriptionConfiguration
        }
        ActionRunKind::VenueCredentialsUpdate
        | ActionRunKind::VenueCredentialsClear
        | ActionRunKind::VenueCredentialsMigrate => AuditResourceKind::VenueCredentials,
        ActionRunKind::OnchainProviderCredentialsUpdate
        | ActionRunKind::OnchainProviderCredentialsClear => AuditResourceKind::ProviderCredentials,
        ActionRunKind::PortfolioClosePosition
        | ActionRunKind::PortfolioClosePair
        | ActionRunKind::PortfolioCloseAll
        | ActionRunKind::PortfolioCloseCompensation
        | ActionRunKind::PortfolioCloseManualTerminal => AuditResourceKind::CloseRun,
    }
}

fn apply_target(context: &mut AuditEventContext, run: &ActionRun) {
    let Some(target) = run
        .target
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    match run.kind {
        ActionRunKind::TradingOrderSubmit => context.client_order_id = Some(target.to_owned()),
        ActionRunKind::TradingOrderCancel => context.order_id = Some(target.to_owned()),
        ActionRunKind::TradingFeeSnapshotUpsert => apply_fee_target(context, target),
        ActionRunKind::VenueCredentialsUpdate
        | ActionRunKind::VenueCredentialsClear
        | ActionRunKind::VenueCredentialsMigrate => context.push_venue(target),
        ActionRunKind::MarketSubscriptionsUpdate | ActionRunKind::GateCrossExModeUpdate => {
            context.push_venue(target)
        }
        ActionRunKind::PortfolioClosePosition | ActionRunKind::PortfolioClosePair => {
            if let Some((venue, symbol)) = target.rsplit_once(':') {
                context.push_venue(venue);
                context.push_symbol(symbol);
            }
        }
        ActionRunKind::PortfolioCloseCompensation | ActionRunKind::PortfolioCloseManualTerminal => {
            context.run_id = Some(target.to_owned())
        }
        ActionRunKind::StockPlanBuild | ActionRunKind::StockPeerPlanBuild => context.run_id = Some(target.to_owned()),
        ActionRunKind::TradingRiskConfigUpdate
        | ActionRunKind::TradingAdapterSelect
        | ActionRunKind::TradingKillSwitch
        | ActionRunKind::TradingOrderReconcile
        | ActionRunKind::AutomationConfigUpdate
        | ActionRunKind::AutomationControl
        | ActionRunKind::AutomationLiveUnlock
        | ActionRunKind::HedgeConfirm
        | ActionRunKind::WebhookConfigUpdate
        | ActionRunKind::WebhookTest
        | ActionRunKind::OnchainProviderCredentialsUpdate
        | ActionRunKind::OnchainProviderCredentialsClear
        | ActionRunKind::OnchainComparisonConfigUpdate
        | ActionRunKind::StockBatchUpdate | ActionRunKind::StockMonitorUpdate
        | ActionRunKind::OnchainBatchAdd
        | ActionRunKind::OnchainBatchRemove
        | ActionRunKind::PortfolioCloseAll => {}
    }
}

fn apply_fee_target(context: &mut AuditEventContext, target: &str) {
    let mut pieces = target.rsplitn(3, ':');
    let _product = pieces.next();
    let symbol = pieces.next();
    let venue = pieces.next();
    if let Some(venue) = venue {
        context.push_venue(venue);
    }
    if let Some(symbol) = symbol {
        context.push_symbol(symbol);
    }
}

fn apply_result_summary(
    context: &mut AuditEventContext,
    kind: ActionRunKind,
    summary: &serde_json::Value,
) {
    match kind {
        ActionRunKind::TradingOrderSubmit | ActionRunKind::TradingOrderCancel => {
            context.order_id = summary_string(summary, "/internalOrderId")
                .or_else(|| summary_string(summary, "/exchangeOrderId"))
                .or_else(|| context.order_id.take());
            context.client_order_id = summary_string(summary, "/clientOrderId")
                .or_else(|| context.client_order_id.take());
            push_pointer(context, summary, "/exchange", Subject::Venue);
            push_pointer(context, summary, "/symbol", Subject::Symbol);
        }
        ActionRunKind::HedgeConfirm => {
            context.run_id = summary_string(summary, "/executionRun/runId");
            context.ticket_id = summary_string(summary, "/executionRun/ticketId");
            push_pointer(
                context,
                summary,
                "/executionRun/longLeg/exchange",
                Subject::Venue,
            );
            push_pointer(
                context,
                summary,
                "/executionRun/shortLeg/exchange",
                Subject::Venue,
            );
            push_pointer(
                context,
                summary,
                "/executionRun/longLeg/symbol",
                Subject::Symbol,
            );
            push_pointer(
                context,
                summary,
                "/executionRun/shortLeg/symbol",
                Subject::Symbol,
            );
            context.order_id = first_array_string(summary, "/executionRun/longLeg/orderIds")
                .or_else(|| first_array_string(summary, "/executionRun/shortLeg/orderIds"));
        }
        ActionRunKind::PortfolioClosePosition
        | ActionRunKind::PortfolioClosePair
        | ActionRunKind::PortfolioCloseAll
        | ActionRunKind::PortfolioCloseCompensation
        | ActionRunKind::PortfolioCloseManualTerminal => {
            context.run_id =
                summary_string(summary, "/closeRunId").or_else(|| context.run_id.take());
            context.order_id = first_array_string(summary, "/costReconciliation/evidenceOrderIds");
        }
        ActionRunKind::TradingFeeSnapshotUpsert => {
            push_pointer(context, summary, "/venue", Subject::Venue);
            push_pointer(context, summary, "/symbol", Subject::Symbol);
        }
        ActionRunKind::VenueCredentialsUpdate
        | ActionRunKind::VenueCredentialsClear
        | ActionRunKind::VenueCredentialsMigrate => {
            push_pointer(context, summary, "/venue", Subject::Venue);
        }
        ActionRunKind::MarketSubscriptionsUpdate | ActionRunKind::GateCrossExModeUpdate
        | ActionRunKind::StockBatchUpdate | ActionRunKind::StockMonitorUpdate | ActionRunKind::StockPlanBuild
        | ActionRunKind::StockPeerPlanBuild => {}
        ActionRunKind::TradingRiskConfigUpdate
        | ActionRunKind::TradingAdapterSelect
        | ActionRunKind::TradingKillSwitch
        | ActionRunKind::TradingOrderReconcile
        | ActionRunKind::AutomationConfigUpdate
        | ActionRunKind::AutomationControl
        | ActionRunKind::AutomationLiveUnlock
        | ActionRunKind::WebhookConfigUpdate
        | ActionRunKind::WebhookTest
        | ActionRunKind::OnchainProviderCredentialsUpdate
        | ActionRunKind::OnchainProviderCredentialsClear
        | ActionRunKind::OnchainComparisonConfigUpdate
        | ActionRunKind::OnchainBatchAdd
        | ActionRunKind::OnchainBatchRemove => {}
    }
}

fn apply_nested_problem(context: &mut AuditEventContext, summary: &serde_json::Value) {
    if context.problem_code.is_some() {
        return;
    }
    context.problem_code = [
        "/executionRun/finalityProblem/code",
        "/executionRun/valuationProblem/code",
        "/executionRun/unwindProblem/code",
    ]
    .iter()
    .find_map(|pointer| summary_string(summary, pointer));
}

#[derive(Clone, Copy)]
enum Subject {
    Venue,
    Symbol,
}

fn push_pointer(
    context: &mut AuditEventContext,
    summary: &serde_json::Value,
    pointer: &str,
    subject: Subject,
) {
    let Some(value) = summary.pointer(pointer).and_then(serde_json::Value::as_str) else {
        return;
    };
    match subject {
        Subject::Venue => context.push_venue(value),
        Subject::Symbol => context.push_symbol(value),
    }
}

fn summary_string(summary: &serde_json::Value, pointer: &str) -> Option<String> {
    summary
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn first_array_string(summary: &serde_json::Value, pointer: &str) -> Option<String> {
    summary
        .pointer(pointer)
        .and_then(serde_json::Value::as_array)
        .and_then(|values| values.iter().find_map(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests;
