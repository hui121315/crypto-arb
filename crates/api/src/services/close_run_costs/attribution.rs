use super::*;

#[derive(Debug, Default)]
pub(super) struct CostParts {
    pub(super) fee: ObservedComponent,
    pub(super) slippage: ObservedComponent,
    pub(super) evidence_order_ids: Vec<String>,
}

impl CostParts {
    pub(super) fn has_evidence(&self) -> bool {
        self.fee.required > 0 || self.slippage.required > 0
    }

    pub(super) fn complete(&self) -> bool {
        !self.has_evidence() || (self.fee.complete() && self.slippage.complete())
    }

    pub(super) fn total(&self) -> f64 {
        self.fee.sum + self.slippage.sum
    }

    pub(super) fn push_evidence(&mut self, order: &OrderRecord) {
        self.evidence_order_ids.push(order.intent.id.clone());
    }

    pub(super) fn push_missing(&self, fee: &str, slippage: &str, fields: &mut Vec<String>) {
        if self.fee.missing() {
            fields.push(fee.to_owned());
        }
        if self.slippage.missing() {
            fields.push(slippage.to_owned());
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct RunCostParts {
    pub(super) funding: ObservedComponent,
    pub(super) manual_handling: ObservedComponent,
}

impl RunCostParts {
    pub(super) fn has_evidence(&self) -> bool {
        self.funding.has_observed()
            || self.manual_handling.required > 0
            || self.manual_handling.has_observed()
    }

    pub(super) fn complete(&self) -> bool {
        self.funding.complete() && self.manual_handling.complete()
    }

    pub(super) fn total(&self) -> f64 {
        self.funding.sum + self.manual_handling.sum
    }

    pub(super) fn push_missing(&self, fields: &mut Vec<String>) {
        if self.manual_handling.missing() {
            fields.push("manual_handling".to_owned());
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ObservedComponent {
    pub(super) required: usize,
    pub(super) observed: usize,
    pub(super) sum: f64,
    pub(super) event_ids: Vec<String>,
}

impl ObservedComponent {
    pub(super) fn observe_order_value(
        &mut self,
        value: Option<f64>,
        events: &[CloseRunCostLedgerEvent],
    ) {
        self.required += 1;
        let event_ids = actual_event_ids(events, CloseRunCostComponent::Fee);
        if let Some(value) = value.filter(|value| value.is_finite()) {
            if event_ids.is_empty() {
                return;
            }
            self.observed += 1;
            self.sum += value;
            self.event_ids.extend(event_ids);
        }
    }

    pub(super) fn observe_event_sum(
        &mut self,
        events: &[CloseRunCostLedgerEvent],
        component: CloseRunCostComponent,
    ) {
        self.required += 1;
        self.observe_events(events, component);
    }

    pub(super) fn observe_optional_event_sum(
        &mut self,
        events: &[CloseRunCostLedgerEvent],
        component: CloseRunCostComponent,
    ) {
        if events
            .iter()
            .any(|event| actual_cost_event(event, component))
        {
            self.required += 1;
            self.observe_events(events, component);
        }
    }

    pub(super) fn observe_events(
        &mut self,
        events: &[CloseRunCostLedgerEvent],
        component: CloseRunCostComponent,
    ) {
        let mut event_ids = Vec::new();
        let mut sum = 0.0;
        for event in events
            .iter()
            .filter(|event| actual_cost_event(event, component))
        {
            if event_ids.iter().any(|id| id == &event.event_id) {
                continue;
            }
            event_ids.push(event.event_id.clone());
            sum += event.amount_usd;
        }
        if !event_ids.is_empty() {
            self.observed += 1;
            self.sum += sum;
            self.event_ids.extend(event_ids);
        }
    }

    pub(super) fn has_observed(&self) -> bool {
        self.observed > 0
    }

    pub(super) fn observed_sum(&self) -> Option<f64> {
        (self.observed > 0).then_some(self.sum)
    }

    pub(super) fn complete(&self) -> bool {
        self.required == self.observed
    }

    pub(super) fn missing(&self) -> bool {
        self.required > self.observed
    }

    pub(super) fn event_ids(&self) -> Vec<String> {
        let mut ids = self.event_ids.clone();
        ids.sort();
        ids.dedup();
        ids
    }
}

fn actual_event_ids(
    events: &[CloseRunCostLedgerEvent],
    component: CloseRunCostComponent,
) -> Vec<String> {
    let mut ids = events
        .iter()
        .filter(|event| actual_cost_event(event, component))
        .map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn actual_cost_event(event: &CloseRunCostLedgerEvent, component: CloseRunCostComponent) -> bool {
    event.component == component
        && event.quality == ExecutionLedgerQuality::Actual
        && event.amount_usd.is_finite()
        && !event.event_id.trim().is_empty()
}
