pub(crate) fn webhook_delivery_diagnostic(value: &str) -> String {
    match value.trim() {
        "webhook delivery failed; target details redacted" | "webhook connection failed" => {
            "Webhook 连接失败（目标地址已安全隐藏）".to_owned()
        }
        "webhook delivery timed out" => "Webhook 连接超时".to_owned(),
        "webhook DNS resolution failed" => "Webhook DNS 解析失败".to_owned(),
        "webhook response body failed" => "Webhook 响应读取失败".to_owned(),
        "Bark acknowledgement is not valid JSON" => "Bark 返回内容不是有效 JSON".to_owned(),
        value => value.to_owned(),
    }
}

pub(crate) fn webhook_delivery_message(
    error: Option<&str>,
    response_message: Option<&str>,
) -> Option<String> {
    error
        .or(response_message)
        .map(webhook_delivery_diagnostic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_redaction_and_timeout_are_actionable() {
        assert_eq!(
            webhook_delivery_diagnostic("webhook delivery failed; target details redacted"),
            "Webhook 连接失败（目标地址已安全隐藏）"
        );
        assert_eq!(
            webhook_delivery_diagnostic("webhook delivery timed out"),
            "Webhook 连接超时"
        );
    }
}
