use super::*;

pub(super) const DOCS: &str =
    "https://geth.ethereum.org/docs/developers/evm-tracing/built-in-tracers#calltracer";

pub(super) enum CreditError {
    Unavailable(String),
    Invalid(String),
}

pub(super) async fn credit(
    client: &reqwest::Client,
    rpc_url: &str,
    transaction_id: &str,
    receipt: &Value,
    scope: &CreditScope,
) -> Result<u128, CreditError> {
    change(client, rpc_url, transaction_id, receipt, scope)
        .await
        .map(|value| value.max(0) as u128)
}

pub(super) async fn change(
    client: &reqwest::Client,
    rpc_url: &str,
    transaction_id: &str,
    receipt: &Value,
    scope: &CreditScope,
) -> Result<i128, CreditError> {
    let transaction = rpc::rpc_result(
        client,
        rpc_url,
        "eth_getTransactionByHash",
        serde_json::json!([transaction_id]),
        703,
    )
    .await
    .map_err(CreditError::Unavailable)?;
    if !transaction
        .get("hash")
        .and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(transaction_id))
        || transaction.get("blockHash") != receipt.get("blockHash")
        || (receipt.get("from").is_some() && transaction.get("from") != receipt.get("from"))
    {
        return Err(CreditError::Invalid(
            "原生币交易与已确认 receipt 不一致".into(),
        ));
    }
    let recipient = transaction
        .get("to")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let sender = transaction
        .get("from")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let amount = transaction
        .get("value")
        .and_then(Value::as_str)
        .and_then(parse_hex_u128)
        .ok_or_else(|| CreditError::Invalid("原生币交易缺少精确 value".into()))?;
    if recipient.eq_ignore_ascii_case(&scope.destination)
        && !sender.eq_ignore_ascii_case(&scope.destination)
        && transaction.get("input").and_then(Value::as_str) == Some("0x")
        && receipt
            .get("gasUsed")
            .and_then(Value::as_str)
            .and_then(parse_hex_u64)
            == Some(21_000)
    {
        return amount
            .try_into()
            .map_err(|_| CreditError::Invalid("原生币数量溢出".into()));
    }
    // Bridge/swap contracts pay native assets via internal calls, not tx.value.
    let trace = rpc::rpc_result_with_limit(
        client,
        rpc_url,
        "debug_traceTransaction",
        serde_json::json!([transaction_id, {"tracer":"callTracer","timeout":"5s",
            "tracerConfig":{"onlyTopCall":false,"withLog":false}}]),
        705,
        1024 * 1024,
    )
    .await
    .map_err(|problem| {
        CreditError::Unavailable(format!(
            "原生币内部到账需要支持 callTracer 的目标链 RPC：{problem}"
        ))
    })?;
    if !trace
        .get("from")
        .and_then(Value::as_str)
        .is_some_and(|from| from.eq_ignore_ascii_case(sender))
        || !trace
            .get("to")
            .and_then(Value::as_str)
            .is_some_and(|to| to.eq_ignore_ascii_case(recipient))
        || trace
            .get("value")
            .and_then(Value::as_str)
            .and_then(parse_hex_u128)
            != Some(amount)
    {
        return Err(CreditError::Invalid(
            "原生币调用跟踪与已确认交易不一致".into(),
        ));
    }
    trace_change(&trace, &scope.destination).map_err(CreditError::Invalid)
}

#[cfg(test)]
fn trace_credit(trace: &Value, destination: &str) -> Result<u128, String> {
    Ok(trace_change(trace, destination)?.max(0) as u128)
}

fn trace_change(trace: &Value, destination: &str) -> Result<i128, String> {
    let destination = normalized_evm_address(destination).ok_or("原生币收款地址无效")?;
    let (mut incoming, mut outgoing) = (0_u128, 0_u128);
    let mut stack = vec![trace];
    let mut visited = 0_u32;
    while let Some(frame) = stack.pop() {
        visited += 1;
        if visited > 16_384 {
            return Err("原生币调用跟踪超出核验上限".into());
        }
        // Reverting a parent rolls back every transfer in its subtree.
        if frame.get("error").is_some_and(|error| !error.is_null()) {
            continue;
        }
        let kind = frame
            .get("type")
            .and_then(Value::as_str)
            .ok_or("调用跟踪缺少 type")?;
        if matches!(
            kind,
            "CALL" | "CREATE" | "CREATE2" | "SELFDESTRUCT" | "SUICIDE"
        ) {
            let from = frame
                .get("from")
                .and_then(Value::as_str)
                .and_then(normalized_evm_address)
                .ok_or("调用发送方无效")?;
            let to = frame
                .get("to")
                .and_then(Value::as_str)
                .and_then(normalized_evm_address)
                .ok_or("调用接收方无效")?;
            let value = frame
                .get("value")
                .and_then(Value::as_str)
                .and_then(parse_hex_u128)
                .ok_or("调用金额无效")?;
            if to == destination {
                incoming = incoming.checked_add(value).ok_or("原生币入账溢出")?;
            }
            if from == destination {
                outgoing = outgoing.checked_add(value).ok_or("原生币支出溢出")?;
            }
        } else if !matches!(kind, "STATICCALL" | "DELEGATECALL" | "CALLCODE") {
            return Err(format!("未知原生币调用类型 {kind}"));
        }
        if let Some(calls) = frame.get("calls") {
            stack.extend(calls.as_array().ok_or("调用跟踪 calls 不是数组")?);
        }
    }
    signed_change(incoming, outgoing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_credit_excludes_reverted_subtrees_and_delegated_value() {
        let wallet = format!("0x{:040x}", 1);
        let router = format!("0x{:040x}", 2);
        let payment = json!({"type":"CALL","from":router,"to":wallet,"value":"0x64"});
        let trace = json!({"type":"CALL","from":wallet,"to":router,"value":"0xa",
            "calls":[payment,
                {"type":"DELEGATECALL","from":router,"to":wallet,"value":"0xffff"},
                {"type":"CALL","from":router,"to":wallet,"value":"0xffff",
                    "error":"execution reverted", "calls":[payment]},
                {"type":"CALL","from":wallet,"to":wallet,"value":"0xffff"}]});
        assert_eq!(trace_credit(&trace, &wallet), Ok(90));
    }

    #[test]
    fn native_credit_does_not_accept_incomplete_call_values() {
        let wallet = format!("0x{:040x}", 1);
        let trace = json!({"type":"CALL","from":wallet,"to":wallet});
        assert!(trace_credit(&trace, &wallet).is_err());
    }
}
