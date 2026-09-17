use shared_types::WebhookDeliveryRecord;

pub(crate) fn request_failure(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "webhook delivery timed out".to_owned()
    } else if error.is_connect() {
        "webhook connection failed".to_owned()
    } else if error.is_body() {
        "webhook response body failed".to_owned()
    } else if error.is_decode() {
        "webhook response decode failed".to_owned()
    } else {
        "webhook delivery request failed".to_owned()
    }
}

pub(crate) fn sanitize_delivery_record(mut record: WebhookDeliveryRecord) -> WebhookDeliveryRecord {
    record.response_message = record.response_message.map(sanitize_diagnostic);
    record.error = record.error.map(sanitize_diagnostic);
    record
}

fn sanitize_diagnostic(message: String) -> String {
    if message == "webhook delivery failed; target details redacted" {
        return "webhook connection failed".to_owned();
    }
    if !message.contains("http://") && !message.contains("https://") {
        return message;
    }
    let lower = message.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        "webhook delivery timed out".to_owned()
    } else if lower.contains("connect") {
        "webhook connection failed".to_owned()
    } else if lower.contains("dns") {
        "webhook DNS resolution failed".to_owned()
    } else {
        "webhook connection failed".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        WebhookApplicationAck, WebhookDeliveryStatus, WebhookEventKind, WebhookProvider,
    };

    #[test]
    fn delivery_diagnostics_never_expose_target_urls() {
        let record = sanitize_delivery_record(WebhookDeliveryRecord {
            event_id: "secret-redaction".to_owned(),
            kind: WebhookEventKind::Test,
            provider: WebhookProvider::Bark,
            status: WebhookDeliveryStatus::Failed,
            attempts: 1,
            response_status: None,
            application_ack: WebhookApplicationAck::Unknown,
            response_message: None,
            error: Some(
                "error sending request for url (https://api.day.app/private-device-key?group=test)"
                    .to_owned(),
            ),
            updated_at_ms: 1,
        });

        let encoded = serde_json::to_string(&record).unwrap_or_default();
        assert!(!encoded.contains("private-device-key"));
        assert!(!encoded.contains("https://"));
        assert_eq!(record.error.as_deref(), Some("webhook connection failed"));
    }
}
