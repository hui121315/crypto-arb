use super::super::super::data::{ConfirmHedgeRequest, ConfirmHedgeSeed, ExecutionPreview};
use shared_types::{ExecutionEnvironment, HedgeLegRole, OrderCompilePlan};

pub(super) fn confirm_request(preview: ExecutionPreview) -> Option<ConfirmHedgeRequest> {
    let (long_venue, short_venue) = plan_venues(&preview.order_plans);
    let client_order_ids = plan_client_order_ids(&preview.order_plans);
    let ExecutionPreview {
        opportunity_id,
        idempotency_key,
        ticket_id,
        execution_mode_label,
        ..
    } = preview;
    let key = idempotency_key?;
    Some(ConfirmHedgeRequest {
        seed: ConfirmHedgeSeed::new(
            opportunity_id,
            key,
            ticket_id,
            execution_environment(execution_mode_label),
            long_venue,
            short_venue,
        ),
        mode_label: execution_mode_label,
        client_order_ids,
    })
}

fn execution_environment(label: &str) -> ExecutionEnvironment {
    if label == "实盘" {
        ExecutionEnvironment::Live
    } else {
        ExecutionEnvironment::Paper
    }
}

fn plan_venues(plans: &[OrderCompilePlan]) -> (Option<String>, Option<String>) {
    let venue = |role| {
        plans
            .iter()
            .find(|plan| plan.role == role)
            .map(|plan| plan.exchange.clone())
    };
    (venue(HedgeLegRole::Long), venue(HedgeLegRole::Short))
}

fn plan_client_order_ids(plans: &[OrderCompilePlan]) -> Vec<String> {
    let mut ids = Vec::new();
    for plan in plans {
        for id in [
            Some(plan.client_order_id_policy.public_client_order_id.as_str()),
            plan.client_order_id_policy.venue_client_order_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !id.trim().is_empty() && !ids.iter().any(|existing| existing == id) {
                ids.push(id.to_owned());
            }
        }
    }
    ids
}
