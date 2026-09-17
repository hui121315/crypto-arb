use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditCorrelation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotency_key: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    order_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    run_ids: Vec<String>,
}

impl AuditCorrelation {
    pub(crate) fn request(request_id: Option<String>) -> Self {
        Self {
            request_id: normalized(request_id),
            ..Self::default()
        }
    }

    pub(crate) fn with_action_run_id(mut self, action_run_id: String) -> Self {
        self.action_run_id = normalized(Some(action_run_id));
        self
    }

    pub(crate) fn with_idempotency_key(mut self, idempotency_key: Option<String>) -> Self {
        self.idempotency_key = normalized(idempotency_key);
        self
    }

    pub(crate) fn with_order_ids(mut self, order_ids: impl IntoIterator<Item = String>) -> Self {
        extend_unique(&mut self.order_ids, order_ids);
        self
    }

    pub(crate) fn with_run_ids(mut self, run_ids: impl IntoIterator<Item = String>) -> Self {
        extend_unique(&mut self.run_ids, run_ids);
        self
    }
}

fn extend_unique(target: &mut Vec<String>, values: impl IntoIterator<Item = String>) {
    for value in values {
        let Some(value) = normalized(Some(value)) else {
            continue;
        };
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn normalized(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlation_normalizes_and_deduplicates_identifiers() -> serde_json::Result<()> {
        let correlation = AuditCorrelation::request(Some(" req-1 ".to_owned()))
            .with_action_run_id(" action-1 ".to_owned())
            .with_idempotency_key(Some(" idem-1 ".to_owned()))
            .with_order_ids([" order-1 ".to_owned(), "order-1".to_owned(), String::new()])
            .with_run_ids(["run-1".to_owned(), " run-1 ".to_owned()]);

        let value = serde_json::to_value(correlation)?;

        assert_eq!(value["requestId"], "req-1");
        assert_eq!(value["actionRunId"], "action-1");
        assert_eq!(value["idempotencyKey"], "idem-1");
        assert_eq!(value["orderIds"], serde_json::json!(["order-1"]));
        assert_eq!(value["runIds"], serde_json::json!(["run-1"]));
        Ok(())
    }
}
