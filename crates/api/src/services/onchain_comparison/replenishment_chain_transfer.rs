//! Exact wallet-to-venue transfer compilation for replenishment.
//!
//! Official contracts:
//! - <https://ethereum.org/developers/docs/apis/json-rpc/>
//! - <https://eips.ethereum.org/EIPS/eip-20>
//! - <https://solana.com/docs/rpc/http/getlatestblockhash>
//! - <https://solana.com/docs/rpc/http/gettokenaccountsbyowner>
//! - <https://solana.com/docs/rpc/http/sendtransaction>
//! - <https://solana.com/docs/tokens/advanced/cpi>

use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use serde_json::Value;
use shared_types::{
    OnchainComparisonConfig, OnchainReplenishmentLeg, OnchainTransferDirection,
    OnchainUnsignedTransaction, EVM_NATIVE_TOKEN_ADDRESS,
};

use crate::state::AppState;

use super::{execution_submit, replenishment_credit, rpc, rpc_target};

const ETHEREUM_RPC_DOCS: &str = "https://ethereum.org/developers/docs/apis/json-rpc/";
const SOLANA_RPC_DOCS: &str = "https://solana.com/docs/rpc/http/sendtransaction";
const SOLANA_WRAPPED_SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const SOLANA_SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const SOLANA_TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const SOLANA_TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const ERC20_TRANSFER_SELECTOR: &str = "a9059cbb";
const ERC20_BALANCE_OF_SELECTOR: &str = "70a08231";

pub(super) struct PreparedChainDeposit {
    submission: execution_submit::IndependentChainSubmission,
    transaction_id: String,
    evidence_source: &'static str,
}

impl PreparedChainDeposit {
    pub(super) fn transaction_id(&self) -> &str {
        &self.transaction_id
    }

    pub(super) fn evidence_source(&self) -> &'static str {
        self.evidence_source
    }

    pub(super) async fn broadcast(self) -> execution_submit::IndependentChainOutcome {
        execution_submit::broadcast_independent_chain_transaction(self.submission).await
    }
}

pub(super) async fn prepare(
    state: &AppState,
    config: &OnchainComparisonConfig,
    leg: &OnchainReplenishmentLeg,
) -> Result<PreparedChainDeposit, String> {
    validate_scope(config, leg)?;
    let rpc_url = execution_submit::verified_submission_rpc(state, config).await?;
    let (url, client) = rpc_target::rpc_target(&rpc_url).await?;
    let amount_raw = replenishment_credit::decimal_to_raw(
        leg.transfer_amount_exact
            .as_deref()
            .ok_or_else(|| "链上转账缺少精确数量".to_owned())?,
        leg.asset_decimals
            .ok_or_else(|| "链上转账缺少代币精度".to_owned())?,
    )
    .filter(|value| *value > 0)
    .ok_or_else(|| "链上转账数量无法转换为最小单位".to_owned())?;
    let transaction = if config.chain.eq_ignore_ascii_case("solana") {
        build_solana(&client, url.as_str(), config, leg, amount_raw).await?
    } else {
        build_evm(&client, url.as_str(), config, leg, amount_raw).await?
    };
    let submission =
        execution_submit::prepare_independent_rpc_transaction(state, config, &transaction).await?;
    let transaction_id = submission.transaction_id().to_owned();
    Ok(PreparedChainDeposit {
        submission,
        transaction_id,
        evidence_source: if config.chain.eq_ignore_ascii_case("solana") {
            SOLANA_RPC_DOCS
        } else {
            ETHEREUM_RPC_DOCS
        },
    })
}

fn validate_scope(
    config: &OnchainComparisonConfig,
    leg: &OnchainReplenishmentLeg,
) -> Result<(), String> {
    if leg.direction != OnchainTransferDirection::DepositToCex {
        return Err("链上转账编译器只接受链上转入交易所的资金腿".to_owned());
    }
    if !leg.chain.eq_ignore_ascii_case(&config.chain) {
        return Err("补仓计划链与当前签名钱包链不一致".to_owned());
    }
    if config.wallet_address.trim().is_empty() {
        return Err("当前链上钱包地址未配置".to_owned());
    }
    if leg.source_address.as_deref() != Some(config.wallet_address.trim()) {
        return Err("补仓计划来源钱包与当前签名钱包不一致，需要重新授权".to_owned());
    }
    if leg.destination.address.as_deref().is_none_or(str::is_empty) {
        return Err("交易所官方充值地址缺失".to_owned());
    }
    if leg.asset_address.as_deref().is_none_or(str::is_empty) {
        return Err("链上资产合约或 Mint 缺失".to_owned());
    }
    Ok(())
}

async fn build_evm(
    client: &reqwest::Client,
    rpc_url: &str,
    config: &OnchainComparisonConfig,
    leg: &OnchainReplenishmentLeg,
    amount_raw: u128,
) -> Result<OnchainUnsignedTransaction, String> {
    let preset = shared_types::onchain_chain_preset(&config.chain)
        .ok_or_else(|| format!("{} 没有已核验的 EVM chain id", config.chain))?;
    let chain_id = preset
        .chain_id
        .ok_or_else(|| format!("{} 不是 EVM 链", config.chain))?;
    let observed_chain_id =
        rpc_hex_u128(client, rpc_url, "eth_chainId", serde_json::json!([]), 701).await?;
    if observed_chain_id != u128::from(chain_id) {
        return Err(format!(
            "自定义 RPC chainId={observed_chain_id}，与计划 {chain_id} 不一致"
        ));
    }
    let wallet = evm_address(&config.wallet_address)?;
    let destination = evm_address(
        leg.destination
            .address
            .as_deref()
            .ok_or_else(|| "交易所充值地址缺失".to_owned())?,
    )?;
    if wallet.eq_ignore_ascii_case(&destination) {
        return Err("链上来源钱包与交易所充值地址不能相同".to_owned());
    }
    let asset = leg
        .asset_address
        .as_deref()
        .ok_or_else(|| "EVM 资产地址缺失".to_owned())?;
    let native = asset.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS);
    let (to, data, value) = if native {
        (
            destination.clone(),
            "0x".to_owned(),
            hex_quantity(amount_raw),
        )
    } else {
        let contract = evm_address(asset)?;
        let data = erc20_transfer_data(&destination, amount_raw)?;
        let balance = erc20_balance(client, rpc_url, &contract, &wallet).await?;
        if balance < amount_raw {
            return Err(format!(
                "链上钱包 {} 原始余额 {balance}，低于转账数量 {amount_raw}",
                leg.asset
            ));
        }
        (contract, data, "0x0".to_owned())
    };
    let gas_price = rpc_text(client, rpc_url, "eth_gasPrice", serde_json::json!([]), 702).await?;
    let gas_price_raw = parse_hex_u128(&gas_price).ok_or_else(|| "EVM gasPrice 非法".to_owned())?;
    let call = serde_json::json!({
        "from": wallet,
        "to": to,
        "data": data,
        "value": value,
    });
    let estimated_gas = rpc_hex_u128(
        client,
        rpc_url,
        "eth_estimateGas",
        serde_json::json!([call]),
        703,
    )
    .await?;
    let gas_limit = estimated_gas
        .checked_mul(120)
        .and_then(|value| value.checked_add(99))
        .map(|value| value / 100)
        .ok_or_else(|| "EVM gas limit 溢出".to_owned())?;
    let gas_debit = gas_limit
        .checked_mul(gas_price_raw)
        .ok_or_else(|| "EVM gas 费用溢出".to_owned())?;
    let native_balance = rpc_hex_u128(
        client,
        rpc_url,
        "eth_getBalance",
        serde_json::json!([wallet, "latest"]),
        704,
    )
    .await?;
    let required_native = if native {
        amount_raw
            .checked_add(gas_debit)
            .ok_or_else(|| "EVM 原生币扣款上限溢出".to_owned())?
    } else {
        gas_debit
    };
    if native_balance < required_native {
        return Err(format!(
            "EVM 钱包原生币余额 {native_balance}，低于转账与 gas 上限 {required_native}"
        ));
    }
    Ok(OnchainUnsignedTransaction::EvmCall {
        chain_id,
        from: wallet,
        to,
        data,
        value,
        gas: hex_quantity(gas_limit),
        gas_price: Some(gas_price),
        max_priority_fee_per_gas: None,
        allowance_spender: None,
    })
}

async fn erc20_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    contract: &str,
    wallet: &str,
) -> Result<u128, String> {
    let wallet_word = abi_address_word(wallet)?;
    rpc_hex_u128(
        client,
        rpc_url,
        "eth_call",
        serde_json::json!([{
            "to": contract,
            "data": format!("0x{ERC20_BALANCE_OF_SELECTOR}{wallet_word}")
        }, "latest"]),
        705,
    )
    .await
}

fn erc20_transfer_data(destination: &str, amount_raw: u128) -> Result<String, String> {
    Ok(format!(
        "0x{ERC20_TRANSFER_SELECTOR}{}{:064x}",
        abi_address_word(destination)?,
        amount_raw
    ))
}

fn abi_address_word(value: &str) -> Result<String, String> {
    let value = evm_address(value)?;
    Ok(format!("{:0>64}", value.trim_start_matches("0x")))
}

fn evm_address(value: &str) -> Result<String, String> {
    let raw = value.trim().strip_prefix("0x").unwrap_or(value.trim());
    if raw.len() != 40 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("EVM 地址非法：{value}"));
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

async fn build_solana(
    client: &reqwest::Client,
    rpc_url: &str,
    config: &OnchainComparisonConfig,
    leg: &OnchainReplenishmentLeg,
    amount_raw: u128,
) -> Result<OnchainUnsignedTransaction, String> {
    let amount_raw =
        u64::try_from(amount_raw).map_err(|_| "Solana 转账数量超过 u64 协议上限".to_owned())?;
    let wallet = config.wallet_address.trim();
    let destination = leg
        .destination
        .address
        .as_deref()
        .ok_or_else(|| "交易所 Solana 充值地址缺失".to_owned())?
        .trim();
    public_key(wallet)?;
    public_key(destination)?;
    if wallet == destination {
        return Err("Solana 来源钱包与交易所充值地址不能相同".to_owned());
    }
    let blockhash = rpc::rpc_result(
        client,
        rpc_url,
        "getLatestBlockhash",
        serde_json::json!([{ "commitment": "confirmed" }]),
        711,
    )
    .await?;
    let value = blockhash
        .get("value")
        .ok_or_else(|| "Solana getLatestBlockhash 缺少 value".to_owned())?;
    let recent_blockhash = value
        .get("blockhash")
        .and_then(Value::as_str)
        .ok_or_else(|| "Solana getLatestBlockhash 缺少 blockhash".to_owned())?;
    let last_valid_block_height = value.get("lastValidBlockHeight").and_then(Value::as_u64);
    let asset = leg
        .asset_address
        .as_deref()
        .ok_or_else(|| "Solana Mint 缺失".to_owned())?;
    let message = if asset == SOLANA_WRAPPED_SOL_MINT {
        compile_solana_native_message(wallet, destination, recent_blockhash, amount_raw)?
    } else {
        let mint = public_key(asset)?;
        let token_program = solana_token_program(client, rpc_url, asset).await?;
        let source =
            source_token_account(client, rpc_url, wallet, asset, &token_program, amount_raw)
                .await?;
        let destination_token =
            destination_token_account(client, rpc_url, destination, asset, &token_program).await?;
        compile_solana_token_message(SolanaTokenMessage {
            authority: public_key(wallet)?,
            source,
            destination: destination_token,
            mint,
            token_program: public_key(&token_program)?,
            blockhash: public_key(recent_blockhash)?,
            amount: amount_raw,
            decimals: leg
                .asset_decimals
                .ok_or_else(|| "Solana 代币精度缺失".to_owned())?,
        })?
    };
    let fee = solana_message_fee(client, rpc_url, &message).await?;
    let wallet_lamports = rpc::rpc_result(
        client,
        rpc_url,
        "getBalance",
        serde_json::json!([wallet, { "commitment": "confirmed" }]),
        716,
    )
    .await?
    .get("value")
    .and_then(Value::as_u64)
    .ok_or_else(|| "Solana getBalance 缺少 value".to_owned())?;
    let required_lamports = if asset == SOLANA_WRAPPED_SOL_MINT {
        amount_raw
            .checked_add(fee)
            .ok_or_else(|| "Solana 扣款上限溢出".to_owned())?
    } else {
        fee
    };
    if wallet_lamports < required_lamports {
        return Err(format!(
            "Solana 钱包 lamports {wallet_lamports}，低于转账与网络费上限 {required_lamports}"
        ));
    }
    let transaction_base64 = B64_STANDARD.encode(legacy_transaction(&message));
    Ok(OnchainUnsignedTransaction::SolanaVersioned {
        transaction_base64,
        request_id: "crossline-replenishment-transfer".to_owned(),
        router: "solana-rpc".to_owned(),
        mode: "exact-transfer".to_owned(),
        last_valid_block_height,
        expire_at_ms: None,
    })
}

async fn solana_token_program(
    client: &reqwest::Client,
    rpc_url: &str,
    mint: &str,
) -> Result<String, String> {
    let value = rpc::rpc_result(
        client,
        rpc_url,
        "getAccountInfo",
        serde_json::json!([mint, { "encoding": "jsonParsed", "commitment": "confirmed" }]),
        712,
    )
    .await?;
    let account = value
        .get("value")
        .filter(|value| !value.is_null())
        .ok_or_else(|| "Solana Mint 账户不存在".to_owned())?;
    let program = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| "Solana Mint 缺少 Token Program 身份".to_owned())?;
    if !matches!(program, SOLANA_TOKEN_PROGRAM | SOLANA_TOKEN_2022_PROGRAM) {
        return Err(format!("Solana Mint 使用了不支持的程序 {program}"));
    }
    if program == SOLANA_TOKEN_2022_PROGRAM {
        let extensions = account
            .pointer("/data/parsed/info/extensions")
            .and_then(Value::as_array);
        if extensions.is_some_and(|rows| !rows.is_empty()) {
            return Err("Token-2022 Mint 含扩展；精确到账量可能变化，当前安全阻断".to_owned());
        }
    }
    Ok(program.to_owned())
}

struct SolanaTokenAccount {
    address: [u8; 32],
    amount: u64,
}

async fn source_token_account(
    client: &reqwest::Client,
    rpc_url: &str,
    owner: &str,
    mint: &str,
    token_program: &str,
    required: u64,
) -> Result<[u8; 32], String> {
    let mut rows = token_accounts(client, rpc_url, owner, mint, token_program).await?;
    rows.sort_by_key(|row| std::cmp::Reverse(row.amount));
    rows.into_iter()
        .find(|row| row.amount >= required)
        .map(|row| row.address)
        .ok_or_else(|| format!("Solana 钱包没有余额至少为 {required} 的对应 Token Account"))
}

async fn destination_token_account(
    client: &reqwest::Client,
    rpc_url: &str,
    destination: &str,
    mint: &str,
    token_program: &str,
) -> Result<[u8; 32], String> {
    if let Some(row) = token_accounts(client, rpc_url, destination, mint, token_program)
        .await?
        .into_iter()
        .next()
    {
        return Ok(row.address);
    }
    let value = rpc::rpc_result(
        client,
        rpc_url,
        "getAccountInfo",
        serde_json::json!([destination, { "encoding": "jsonParsed", "commitment": "confirmed" }]),
        714,
    )
    .await?;
    let account = value.get("value").filter(|value| !value.is_null());
    let direct_token_account = account.is_some_and(|account| {
        account.get("owner").and_then(Value::as_str) == Some(token_program)
            && account
                .pointer("/data/parsed/info/mint")
                .and_then(Value::as_str)
                == Some(mint)
    });
    if direct_token_account {
        return public_key(destination);
    }
    Err(
        "交易所 Solana 充值地址没有已存在且 Mint 匹配的 Token Account；不会自动创建或猜测 ATA"
            .to_owned(),
    )
}

async fn token_accounts(
    client: &reqwest::Client,
    rpc_url: &str,
    owner: &str,
    mint: &str,
    token_program: &str,
) -> Result<Vec<SolanaTokenAccount>, String> {
    let value = rpc::rpc_result(
        client,
        rpc_url,
        "getTokenAccountsByOwner",
        serde_json::json!([
            owner,
            { "mint": mint },
            { "encoding": "jsonParsed", "commitment": "confirmed" }
        ]),
        713,
    )
    .await?;
    let rows = value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Solana Token Account 响应缺少 value".to_owned())?;
    rows.iter()
        .filter(|row| row.pointer("/account/owner").and_then(Value::as_str) == Some(token_program))
        .filter(|row| {
            row.pointer("/account/data/parsed/info/mint")
                .and_then(Value::as_str)
                == Some(mint)
        })
        .map(|row| {
            let address = row
                .get("pubkey")
                .and_then(Value::as_str)
                .ok_or_else(|| "Solana Token Account 缺少 pubkey".to_owned())?;
            let amount = row
                .pointer("/account/data/parsed/info/tokenAmount/amount")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(|| "Solana Token Account 缺少原始余额".to_owned())?;
            Ok(SolanaTokenAccount {
                address: public_key(address)?,
                amount,
            })
        })
        .collect()
}

async fn solana_message_fee(
    client: &reqwest::Client,
    rpc_url: &str,
    message: &[u8],
) -> Result<u64, String> {
    rpc::rpc_result(
        client,
        rpc_url,
        "getFeeForMessage",
        serde_json::json!([
            B64_STANDARD.encode(message),
            { "commitment": "confirmed" }
        ]),
        715,
    )
    .await?
    .get("value")
    .and_then(Value::as_u64)
    .ok_or_else(|| "Solana getFeeForMessage 没有返回可用网络费".to_owned())
}

pub(super) fn compile_solana_native_message(
    from: &str,
    destination: &str,
    blockhash: &str,
    amount: u64,
) -> Result<Vec<u8>, String> {
    let accounts = [
        public_key(from)?,
        public_key(destination)?,
        public_key(SOLANA_SYSTEM_PROGRAM)?,
    ];
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2_u32.to_le_bytes());
    data.extend_from_slice(&amount.to_le_bytes());
    compile_legacy_message(
        [1, 0, 1],
        &accounts,
        public_key(blockhash)?,
        2,
        &[0, 1],
        &data,
    )
}

pub(super) struct SolanaTokenMessage {
    pub authority: [u8; 32],
    pub source: [u8; 32],
    pub destination: [u8; 32],
    pub mint: [u8; 32],
    pub token_program: [u8; 32],
    pub blockhash: [u8; 32],
    pub amount: u64,
    pub decimals: u8,
}

pub(super) fn compile_solana_token_message(input: SolanaTokenMessage) -> Result<Vec<u8>, String> {
    let accounts = [
        input.authority,
        input.source,
        input.destination,
        input.mint,
        input.token_program,
    ];
    let mut data = Vec::with_capacity(10);
    data.push(12);
    data.extend_from_slice(&input.amount.to_le_bytes());
    data.push(input.decimals);
    compile_legacy_message(
        [1, 0, 2],
        &accounts,
        input.blockhash,
        4,
        &[1, 3, 2, 0],
        &data,
    )
}

pub(super) fn compile_legacy_message(
    header: [u8; 3],
    accounts: &[[u8; 32]],
    blockhash: [u8; 32],
    program_index: u8,
    instruction_accounts: &[u8],
    instruction_data: &[u8],
) -> Result<Vec<u8>, String> {
    compile_legacy_instructions(
        header,
        accounts,
        blockhash,
        &[(program_index, instruction_accounts, instruction_data)],
    )
}

pub(super) fn compile_solana_token_create_message(
    input: SolanaTokenMessage,
    owner: [u8; 32],
) -> Result<Vec<u8>, String> {
    let accounts = [
        input.authority,
        input.source,
        input.destination,
        input.mint,
        input.token_program,
        owner,
        [0; 32],
        public_key("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")?,
    ];
    let mut transfer = vec![12];
    transfer.extend_from_slice(&input.amount.to_le_bytes());
    transfer.push(input.decimals);
    // Official ATA CreateIdempotent, followed by TransferChecked in the same transaction.
    compile_legacy_instructions(
        [1, 0, 5],
        &accounts,
        input.blockhash,
        &[
            (7, &[0, 2, 5, 3, 6, 4], &[1]),
            (4, &[1, 3, 2, 0], &transfer),
        ],
    )
}

fn compile_legacy_instructions(
    header: [u8; 3],
    accounts: &[[u8; 32]],
    blockhash: [u8; 32],
    instructions: &[(u8, &[u8], &[u8])],
) -> Result<Vec<u8>, String> {
    let account_count =
        u16::try_from(accounts.len()).map_err(|_| "Solana account 数量超过协议上限".to_owned())?;
    let mut message = Vec::with_capacity(128);
    message.extend_from_slice(&header);
    encode_short_vec(account_count, &mut message);
    for account in accounts {
        message.extend_from_slice(account);
    }
    message.extend_from_slice(&blockhash);
    encode_short_vec(
        u16::try_from(instructions.len()).map_err(|_| "Solana instruction 数量溢出")?,
        &mut message,
    );
    for &(program_index, instruction_accounts, instruction_data) in instructions {
        message.push(program_index);
        encode_short_vec(
            u16::try_from(instruction_accounts.len())
                .map_err(|_| "Solana instruction account 数量溢出".to_owned())?,
            &mut message,
        );
        message.extend_from_slice(instruction_accounts);
        encode_short_vec(
            u16::try_from(instruction_data.len())
                .map_err(|_| "Solana instruction data 过长".to_owned())?,
            &mut message,
        );
        message.extend_from_slice(instruction_data);
    }
    Ok(message)
}

pub(super) fn legacy_transaction(message: &[u8]) -> Vec<u8> {
    let mut transaction = Vec::with_capacity(message.len() + 65);
    encode_short_vec(1, &mut transaction);
    transaction.extend_from_slice(&[0_u8; 64]);
    transaction.extend_from_slice(message);
    transaction
}

fn encode_short_vec(mut value: u16, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn public_key(value: &str) -> Result<[u8; 32], String> {
    let bytes = bs58::decode(value.trim())
        .into_vec()
        .map_err(|_| format!("Solana public key 非法：{value}"))?;
    bytes
        .try_into()
        .map_err(|_| format!("Solana public key 长度不是 32 字节：{value}"))
}

async fn rpc_text(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: Value,
    id: u64,
) -> Result<String, String> {
    rpc::rpc_result(client, rpc_url, method, params, id)
        .await?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{method} 没有返回字符串结果"))
}

async fn rpc_hex_u128(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: Value,
    id: u64,
) -> Result<u128, String> {
    let value = rpc_text(client, rpc_url, method, params, id).await?;
    parse_hex_u128(&value).ok_or_else(|| format!("{method} 返回了非法 hex quantity"))
}

fn parse_hex_u128(value: &str) -> Option<u128> {
    let raw = value.trim().strip_prefix("0x")?;
    (!raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| u128::from_str_radix(raw, 16).ok())
        .flatten()
}

fn hex_quantity(value: u128) -> String {
    format!("0x{value:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erc20_transfer_uses_exact_recipient_and_raw_amount() {
        let data = erc20_transfer_data("0x1111111111111111111111111111111111111111", 1_250_000)
            .expect("ERC-20 transfer data");

        assert_eq!(data.len(), 2 + 8 + 64 + 64);
        assert!(data.starts_with("0xa9059cbb"));
        assert!(data.contains("1111111111111111111111111111111111111111"));
        assert!(data.ends_with("00000000000000000000000000000000000000000000000000000000001312d0"));
    }

    #[test]
    fn solana_native_message_keeps_fee_payer_destination_and_exact_lamports() {
        let message = compile_solana_native_message(
            SOLANA_SYSTEM_PROGRAM,
            SOLANA_WRAPPED_SOL_MINT,
            SOLANA_WRAPPED_SOL_MINT,
            42,
        )
        .expect("Solana transfer message");

        assert_eq!(&message[..3], &[1, 0, 1]);
        assert!(message.ends_with(&42_u64.to_le_bytes()));
        assert_eq!(legacy_transaction(&message).len(), message.len() + 65);
    }
}
