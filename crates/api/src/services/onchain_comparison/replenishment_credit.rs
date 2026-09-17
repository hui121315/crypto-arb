use serde_json::Value;
use shared_types::{OnchainReplenishmentLeg, OnchainRpcMode, EVM_NATIVE_TOKEN_ADDRESS};

use crate::state::AppState;

use super::{rpc, rpc_target};

#[path = "replenishment_credit/native.rs"]
mod native;
mod network_cost;
mod solana;
pub(super) mod stock_funding;
pub(super) mod swap;
pub(super) mod wallet;

const SOLANA_SIGNATURE_STATUS_DOCS: &str = "https://solana.com/docs/rpc/http/getsignaturestatuses";
const SOLANA_TRANSACTION_DOCS: &str = "https://solana.com/docs/rpc/http/gettransaction";
const ETHEREUM_RPC_DOCS: &str = "https://ethereum.org/developers/docs/apis/json-rpc/";
const ERC20_TRANSFER_DOCS: &str = "https://eips.ethereum.org/EIPS/eip-20";
const SOLANA_TRANSACTION_MAX_BYTES: usize = 1024 * 1024;
const EVM_EVIDENCE_MAX_BYTES: usize = 1024 * 1024;
const SOLANA_WRAPPED_SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const ERC20_TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DestinationCreditCheck {
    Pending {
        confirmations: Option<u64>,
        source: String,
        problem: String,
    },
    Credited {
        credited_amount_raw: u128,
        confirmations: Option<u64>,
        source: String,
    },
    Rejected {
        source: String,
        problem: String,
    },
}

pub(super) async fn check(
    state: &AppState,
    leg: &OnchainReplenishmentLeg,
    transaction_id: &str,
) -> DestinationCreditCheck {
    let Some(scope) = credit_scope(leg) else {
        return rejected("补仓计划缺少目标地址、代币合约、精度或精确到账数量");
    };
    check_scope(state, &leg.chain, scope, transaction_id, None).await
}

pub(super) async fn check_with_cost(
    state: &AppState,
    leg: &OnchainReplenishmentLeg,
    transaction_id: &str,
) -> (
    DestinationCreditCheck,
    Option<shared_types::OnchainReplenishmentNetworkCost>,
) {
    let Some(scope) = credit_scope(leg) else {
        return (
            rejected("补仓计划缺少目标地址、代币合约、精度或精确到账数量"),
            None,
        );
    };
    let mut cost = None;
    let credit = check_scope(state, &leg.chain, scope, transaction_id, Some(&mut cost)).await;
    (credit, cost)
}

async fn check_scope(
    state: &AppState,
    chain: &str,
    scope: CreditScope,
    transaction_id: &str,
    cost: Option<&mut Option<shared_types::OnchainReplenishmentNetworkCost>>,
) -> DestinationCreditCheck {
    let Some((rpc_url, rpc_source)) = rpc_endpoint(state, chain) else {
        return rejected("当前链没有可用于到账核验的已验证 RPC");
    };
    let target = rpc_target::rpc_target(&rpc_url).await;
    let (url, client) = match target {
        Ok(target) => target,
        Err(problem) => {
            return DestinationCreditCheck::Pending {
                confirmations: None,
                source: rpc_source,
                problem,
            }
        }
    };
    check_on_rpc_with_cost(&client, url.as_str(), chain, &scope, transaction_id, cost).await
}

#[cfg(test)]
async fn check_on_rpc(
    client: &reqwest::Client,
    rpc_url: &str,
    chain: &str,
    scope: &CreditScope,
    transaction_id: &str,
) -> DestinationCreditCheck {
    check_on_rpc_with_cost(client, rpc_url, chain, scope, transaction_id, None).await
}

async fn check_on_rpc_with_cost(
    client: &reqwest::Client,
    rpc_url: &str,
    chain: &str,
    scope: &CreditScope,
    transaction_id: &str,
    cost: Option<&mut Option<shared_types::OnchainReplenishmentNetworkCost>>,
) -> DestinationCreditCheck {
    let (method, expected) = if chain.eq_ignore_ascii_case("solana") {
        (
            "getGenesisHash",
            super::rpc::SOLANA_MAINNET_GENESIS_HASH.to_owned(),
        )
    } else {
        let Some(id) = shared_types::onchain_chain_preset(chain).and_then(|preset| preset.chain_id)
        else {
            return rejected("目标链身份未登记，无法核验到账");
        };
        ("eth_chainId", format!("0x{id:x}"))
    };
    let observed = match rpc::rpc_result(client, rpc_url, method, serde_json::json!([]), 600).await
    {
        Ok(value) => value,
        Err(problem) => return pending("rpc_chain_identity", None, problem),
    };
    if observed.as_str() != Some(expected.as_str()) {
        return rejected("到账核验 RPC 的链身份与目标主网不一致");
    }
    if chain.eq_ignore_ascii_case("solana") {
        check_solana(client, rpc_url, scope, transaction_id, cost).await
    } else {
        check_evm(client, rpc_url, chain, scope, transaction_id, cost).await
    }
}

struct CreditScope {
    destination: String,
    asset_address: String,
    expected_raw: u128,
    required_confirmations: u64,
}

fn credit_scope(leg: &OnchainReplenishmentLeg) -> Option<CreditScope> {
    let destination = leg.destination.address.as_deref()?.trim();
    let asset_address = leg.asset_address.as_deref()?.trim();
    let decimals = leg.asset_decimals?;
    let amount = leg.transfer_amount_exact.as_deref()?;
    Some(CreditScope {
        destination: destination.to_owned(),
        asset_address: asset_address.to_owned(),
        expected_raw: decimal_to_raw(amount, decimals)?,
        required_confirmations: leg
            .network_evidence
            .credit_confirmations
            .unwrap_or(1)
            .max(1),
    })
}

pub(super) fn rpc_endpoint(state: &AppState, chain: &str) -> Option<(String, String)> {
    if let Some(url) = crate::services::onchain_rpc_registry::configured_url(chain) {
        return Some((url, "secure_custom_rpc".to_owned()));
    }
    let snapshot = state.onchain_monitor().snapshot();
    if snapshot.config.chain.eq_ignore_ascii_case(chain)
        && snapshot.config.rpc.mode == OnchainRpcMode::Custom
        && state.onchain_monitor().rpc_status().ready
    {
        return state
            .onchain_monitor()
            .custom_rpc_url()
            .map(|url| (url.as_str().to_owned(), "custom_rpc".to_owned()));
    }
    if chain.eq_ignore_ascii_case("solana") {
        return Some((
            super::solana_token_precision::SOLANA_MAINNET_RPC.to_owned(),
            "system_solana_rpc".to_owned(),
        ));
    }
    let chain = chain.trim().to_ascii_lowercase();
    super::evm_token_identity::public_rpc(&chain)
        .map(|endpoint| (endpoint.url.to_owned(), endpoint.source.to_owned()))
}

async fn check_solana(
    client: &reqwest::Client,
    rpc_url: &str,
    scope: &CreditScope,
    transaction_id: &str,
    cost: Option<&mut Option<shared_types::OnchainReplenishmentNetworkCost>>,
) -> DestinationCreditCheck {
    let status = match rpc::rpc_result(
        client,
        rpc_url,
        "getSignatureStatuses",
        serde_json::json!([[transaction_id], { "searchTransactionHistory": true }]),
        601,
    )
    .await
    {
        Ok(value) => value,
        Err(problem) => return pending(SOLANA_SIGNATURE_STATUS_DOCS, None, problem),
    };
    let Some(status) = status
        .get("value")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .filter(|row| !row.is_null())
    else {
        return pending(
            SOLANA_SIGNATURE_STATUS_DOCS,
            None,
            "Solana 尚未索引该链上交易".to_owned(),
        );
    };
    let confirmations = status.get("confirmations").and_then(Value::as_u64);
    let confirmation_status = status
        .get("confirmationStatus")
        .and_then(Value::as_str)
        .unwrap_or("processed");
    let finality_ready = confirmation_status == "finalized"
        || confirmation_status == "confirmed"
            && (scope.required_confirmations == 1
                || confirmations.is_some_and(|value| value >= scope.required_confirmations));
    if !finality_ready {
        return pending(
            SOLANA_SIGNATURE_STATUS_DOCS,
            confirmations,
            format!("Solana 确认数尚未达到 {}", scope.required_confirmations),
        );
    }
    let transaction = match rpc::rpc_result_with_limit(
        client,
        rpc_url,
        "getTransaction",
        serde_json::json!([
            transaction_id,
            {
                "commitment": if confirmation_status == "finalized" { "finalized" } else { "confirmed" },
                "encoding": "jsonParsed",
                "maxSupportedTransactionVersion": 0
            }
        ]),
        602,
        SOLANA_TRANSACTION_MAX_BYTES,
    )
    .await
    {
        Ok(value) if !value.is_null() => value,
        Ok(_) => {
            return pending(
                SOLANA_TRANSACTION_DOCS,
                confirmations,
                "Solana 已确认签名，但交易明细尚未可读".to_owned(),
            )
        }
        Err(problem) => return pending(SOLANA_TRANSACTION_DOCS, confirmations, problem),
    };
    let confirmed_slot = status.get("slot").and_then(Value::as_u64);
    if confirmed_slot.is_none()
        || transaction
            .pointer("/transaction/signatures/0")
            .and_then(Value::as_str)
            != Some(transaction_id)
        || transaction.get("slot").and_then(Value::as_u64) != confirmed_slot
    {
        return rejected("Solana 交易签名或所在 slot 与已确认交易不一致");
    }
    if transaction.get("meta").is_none_or(Value::is_null) {
        return pending(
            SOLANA_TRANSACTION_DOCS,
            confirmations,
            "Solana 交易元数据尚未可读".into(),
        );
    }
    if transaction.pointer("/meta/err").is_none() {
        return pending(
            SOLANA_TRANSACTION_DOCS,
            confirmations,
            "Solana 交易明细缺少执行结果，尚不能确认到账".into(),
        );
    }
    if status.get("err") != transaction.pointer("/meta/err") {
        return pending(
            SOLANA_TRANSACTION_DOCS,
            confirmations,
            "Solana 签名状态与交易执行结果不一致".into(),
        );
    }
    if let Some(cost) = cost {
        *cost = network_cost::solana(&transaction, transaction_id);
    }
    if transaction
        .pointer("/meta/err")
        .is_some_and(|error| !error.is_null())
    {
        return DestinationCreditCheck::Rejected {
            source: SOLANA_TRANSACTION_DOCS.to_owned(),
            problem: format!("Solana 交易元数据报告失败：{}", transaction["meta"]["err"]),
        };
    }
    let credited = if scope.asset_address == SOLANA_WRAPPED_SOL_MINT {
        solana::native_credit(&transaction, &scope.destination)
    } else {
        solana::token_credit(&transaction, &scope.destination, &scope.asset_address)
    };
    match credited {
        Ok(value) if value >= scope.expected_raw => DestinationCreditCheck::Credited {
            credited_amount_raw: value,
            confirmations,
            source: SOLANA_TRANSACTION_DOCS.to_owned(),
        },
        Ok(value) => DestinationCreditCheck::Rejected {
            source: SOLANA_TRANSACTION_DOCS.to_owned(),
            problem: format!(
                "交易已确认，但目标地址仅收到原始数量 {value}，低于预期 {}",
                scope.expected_raw
            ),
        },
        Err(problem) => DestinationCreditCheck::Rejected {
            source: SOLANA_TRANSACTION_DOCS.to_owned(),
            problem,
        },
    }
}

async fn check_evm(
    client: &reqwest::Client,
    rpc_url: &str,
    chain: &str,
    scope: &CreditScope,
    transaction_id: &str,
    cost: Option<&mut Option<shared_types::OnchainReplenishmentNetworkCost>>,
) -> DestinationCreditCheck {
    let receipt = match rpc::rpc_result_with_limit(
        client,
        rpc_url,
        "eth_getTransactionReceipt",
        serde_json::json!([transaction_id]),
        701,
        EVM_EVIDENCE_MAX_BYTES,
    )
    .await
    {
        Ok(value) if !value.is_null() => value,
        Ok(_) => return pending(ETHEREUM_RPC_DOCS, None, "EVM receipt 尚未生成".to_owned()),
        Err(problem) => return pending(ETHEREUM_RPC_DOCS, None, problem),
    };
    if !receipt
        .get("transactionHash")
        .and_then(Value::as_str)
        .is_some_and(|hash| hash.eq_ignore_ascii_case(transaction_id))
    {
        return rejected("EVM receipt 与待核验的交易哈希不一致");
    }
    match receipt.get("status").and_then(Value::as_str) {
        Some("0x0" | "0x1") => {}
        _ => {
            return pending(
                ETHEREUM_RPC_DOCS,
                None,
                "EVM receipt 缺少成功终态".to_owned(),
            )
        }
    }
    let confirmations = evm_confirmations(client, rpc_url, &receipt).await;
    if confirmations.unwrap_or_default() < scope.required_confirmations {
        return pending(
            ETHEREUM_RPC_DOCS,
            confirmations,
            format!("EVM 确认数尚未达到 {}", scope.required_confirmations),
        );
    }
    if let Some(cost) = cost {
        *cost = network_cost::evm(client, rpc_url, chain, &receipt).await;
    }
    if receipt["status"] == "0x0" {
        return DestinationCreditCheck::Rejected {
            source: ETHEREUM_RPC_DOCS.to_owned(),
            problem: "EVM receipt status=0x0；交易失败，网络费仍可能已经扣除".to_owned(),
        };
    }
    let credited = if scope
        .asset_address
        .eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS)
    {
        match native::credit(client, rpc_url, transaction_id, &receipt, scope).await {
            Ok(value) => Ok(value),
            Err(native::CreditError::Unavailable(problem)) => {
                return pending(native::DOCS, confirmations, problem)
            }
            Err(native::CreditError::Invalid(problem)) => Err(problem),
        }
    } else {
        evm_token_credit(&receipt, scope)
    };
    match credited {
        Ok(value) if value >= scope.expected_raw => DestinationCreditCheck::Credited {
            credited_amount_raw: value,
            confirmations,
            source: if scope
                .asset_address
                .eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS)
            {
                format!("{ETHEREUM_RPC_DOCS} · {}", native::DOCS)
            } else {
                ERC20_TRANSFER_DOCS.to_owned()
            },
        },
        Ok(value) => DestinationCreditCheck::Rejected {
            source: ETHEREUM_RPC_DOCS.to_owned(),
            problem: format!(
                "交易已确认，但目标地址仅收到原始数量 {value}，低于预期 {}",
                scope.expected_raw
            ),
        },
        Err(problem) => DestinationCreditCheck::Rejected {
            source: ETHEREUM_RPC_DOCS.to_owned(),
            problem,
        },
    }
}

async fn evm_confirmations(
    client: &reqwest::Client,
    rpc_url: &str,
    receipt: &Value,
) -> Option<u64> {
    let included = parse_hex_u64(receipt.get("blockNumber")?.as_str()?)?;
    let block = rpc::rpc_result_with_limit(
        client,
        rpc_url,
        "eth_getBlockByNumber",
        serde_json::json!([receipt["blockNumber"], false]),
        704,
        EVM_EVIDENCE_MAX_BYTES,
    )
    .await
    .ok()?;
    let receipt_hash = receipt.get("blockHash")?.as_str()?;
    if !block
        .get("hash")?
        .as_str()?
        .eq_ignore_ascii_case(receipt_hash)
    {
        return None;
    }
    let latest = rpc::rpc_result(
        client,
        rpc_url,
        "eth_blockNumber",
        serde_json::json!([]),
        702,
    )
    .await
    .ok()?
    .as_str()
    .and_then(parse_hex_u64)?;
    latest.checked_sub(included)?.checked_add(1)
}

fn evm_token_credit(receipt: &Value, scope: &CreditScope) -> Result<u128, String> {
    Ok(evm_token_change(receipt, scope)?.max(0) as u128)
}

fn evm_token_change(receipt: &Value, scope: &CreditScope) -> Result<i128, String> {
    let destination = normalized_evm_address(&scope.destination).ok_or("目标 EVM 地址格式无效")?;
    let logs = receipt
        .get("logs")
        .and_then(Value::as_array)
        .ok_or("EVM receipt 缺少 logs")?;
    let (mut incoming, mut outgoing) = (0_u128, 0_u128);
    let mut indices = std::collections::BTreeSet::new();
    for log in logs {
        if !log
            .get("address")
            .and_then(Value::as_str)
            .is_some_and(|address| address.eq_ignore_ascii_case(&scope.asset_address))
        {
            continue;
        }
        let Some(topics) = log.get("topics").and_then(Value::as_array) else {
            continue;
        };
        if !topics
            .first()
            .and_then(Value::as_str)
            .is_some_and(|topic| topic.eq_ignore_ascii_case(ERC20_TRANSFER_TOPIC))
        {
            continue;
        }
        if topics.len() != 3 || log.get("removed").and_then(Value::as_bool) == Some(true) {
            return Err("ERC-20 转账日志格式不符或已被链重组移除".into());
        }
        let sender = topics[1]
            .as_str()
            .and_then(topic_address)
            .ok_or("Transfer 缺少有效发送方")?;
        let recipient = topics[2]
            .as_str()
            .and_then(topic_address)
            .ok_or("Transfer 缺少有效接收方")?;
        if sender != destination && recipient != destination {
            continue;
        }
        let index = log
            .get("logIndex")
            .and_then(Value::as_str)
            .and_then(parse_hex_u64)
            .ok_or("Transfer 缺少有效日志序号")?;
        if !indices.insert(index) {
            return Err("receipt 含重复转账日志，无法确认真实到账".into());
        }
        let amount = log
            .get("data")
            .and_then(Value::as_str)
            .filter(|data| data.len() == 66)
            .and_then(parse_hex_u128)
            .ok_or("ERC-20 Transfer 日志数量无效或超出精确计量范围")?;
        if recipient == destination {
            incoming = incoming.checked_add(amount).ok_or("ERC-20 到账数量溢出")?;
        }
        if sender == destination {
            outgoing = outgoing.checked_add(amount).ok_or("ERC-20 支出数量溢出")?;
        }
    }
    signed_change(incoming, outgoing)
}

fn signed_change(incoming: u128, outgoing: u128) -> Result<i128, String> {
    if incoming >= outgoing {
        (incoming - outgoing)
            .try_into()
            .map_err(|_| "净入账超出精确核算范围".into())
    } else {
        i128::try_from(outgoing - incoming)
            .map(|value| -value)
            .map_err(|_| "净支出超出精确核算范围".into())
    }
}

pub(super) fn decimal_to_raw(value: &str, decimals: u8) -> Option<u128> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('-') || value.contains(['e', 'E']) {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > usize::from(decimals)
    {
        return None;
    }
    let mut digits = String::with_capacity(whole.len() + usize::from(decimals));
    digits.push_str(if whole.is_empty() { "0" } else { whole });
    digits.push_str(fraction);
    digits.extend(std::iter::repeat_n(
        '0',
        usize::from(decimals) - fraction.len(),
    ));
    digits.parse::<u128>().ok()
}

fn normalized_evm_address(value: &str) -> Option<String> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    (value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn topic_address(value: &str) -> Option<String> {
    let value = value.strip_prefix("0x")?;
    if value.len() != 64 || !value.get(..24)?.bytes().all(|byte| byte == b'0') {
        return None;
    }
    normalized_evm_address(value.get(24..)?)
}

fn parse_hex_u64(value: &str) -> Option<u64> {
    let value = value.strip_prefix("0x")?;
    (!value.is_empty())
        .then(|| u64::from_str_radix(value, 16).ok())
        .flatten()
}

fn parse_hex_u128(value: &str) -> Option<u128> {
    let value = value.strip_prefix("0x")?;
    (!value.is_empty())
        .then(|| u128::from_str_radix(value, 16).ok())
        .flatten()
}

fn pending(source: &str, confirmations: Option<u64>, problem: String) -> DestinationCreditCheck {
    DestinationCreditCheck::Pending {
        confirmations,
        source: source.to_owned(),
        problem,
    }
}

fn rejected(problem: &str) -> DestinationCreditCheck {
    DestinationCreditCheck::Rejected {
        source: "replenishment_plan".to_owned(),
        problem: problem.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_decimal_amount_compiles_without_float_rounding() {
        assert_eq!(decimal_to_raw("100.01", 6), Some(100_010_000));
        assert_eq!(decimal_to_raw("0.000001", 6), Some(1));
        assert_eq!(decimal_to_raw("0.0000001", 6), None);
        assert_eq!(decimal_to_raw("1e2", 6), None);
    }

    #[test]
    fn erc20_receipt_requires_contract_recipient_and_amount() {
        let scope = CreditScope {
            destination: "0x00000000000000000000000000000000000000ab".to_owned(),
            asset_address: "0x00000000000000000000000000000000000000cd".to_owned(),
            expected_raw: 100,
            required_confirmations: 1,
        };
        let receipt = serde_json::json!({
            "logs": [{
                "address": scope.asset_address,
                "topics": [
                    ERC20_TRANSFER_TOPIC,
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "0x00000000000000000000000000000000000000000000000000000000000000ab"
                ],
                "data": format!("0x{:064x}", 100),
                "logIndex": "0x0"
            }]
        });
        assert_eq!(evm_token_credit(&receipt, &scope), Ok(100));
    }

    #[test]
    fn solana_token_delta_is_scoped_to_owner_and_mint() {
        let transaction = serde_json::json!({
            "transaction": { "message": { "accountKeys": ["wallet", "token-account"] } },
            "meta": {
                "preTokenBalances": [{
                    "accountIndex": 1,
                    "owner": "wallet",
                    "mint": "mint",
                    "uiTokenAmount": { "amount": "10" }
                }],
                "postTokenBalances": [{
                    "accountIndex": 1,
                    "owner": "wallet",
                    "mint": "mint",
                    "uiTokenAmount": { "amount": "110" }
                }]
            }
        });
        assert_eq!(
            solana::token_credit(&transaction, "wallet", "mint"),
            Ok(100)
        );
    }
}

#[cfg(test)]
#[path = "replenishment_credit/rpc_tests.rs"]
pub(super) mod rpc_tests;
