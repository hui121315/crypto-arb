use super::rpc::rpc_result;
use super::rpc_target::rpc_target;
use crate::state::AppState;
use onchain_monitor::{OnchainWalletAssetBalance, OnchainWalletInventory};
use shared_types::{OnchainComparisonConfig, OnchainRpcMode, EVM_NATIVE_TOKEN_ADDRESS};

pub(super) const WALLET_INVENTORY_INTERVAL_MS: i64 = 5_000;
pub(super) const WALLET_INVENTORY_MAX_AGE_MS: i64 = 15_000;

pub(super) const SOLANA_WRAPPED_SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const SOLANA_BALANCE_DOCS: &str = "https://solana.com/docs/rpc/http/getbalance";
const SOLANA_TOKEN_BALANCE_DOCS: &str = "https://solana.com/docs/rpc/http/gettokenaccountsbyowner";
const EVM_BALANCE_DOCS: &str = "https://ethereum.org/developers/docs/apis/json-rpc/#eth_getbalance";
const EVM_CALL_DOCS: &str = "https://ethereum.org/developers/docs/apis/json-rpc/#eth_call";

pub(super) async fn refresh_if_due(state: &AppState, now_ms: i64) {
    let config = state.onchain_monitor().snapshot().config.clone();
    if !inventory_probe_ready(state, &config)
        || !state
            .onchain_monitor()
            .try_begin_wallet_attempt(now_ms, WALLET_INVENTORY_INTERVAL_MS)
    {
        return;
    }
    let Some((rpc_url, rpc_source)) = balance_rpc_target(state, &config) else {
        return;
    };
    let inventory = fetch(&config, rpc_url.as_str(), rpc_source, now_ms).await;
    if state.onchain_monitor().snapshot().config != config {
        return;
    }
    state.onchain_monitor().publish_wallet_inventory(inventory);
    super::project_latest(state, common::time::now_ms(), true);
}

fn inventory_probe_ready(state: &AppState, config: &OnchainComparisonConfig) -> bool {
    config.enabled
        && !config.wallet_address.trim().is_empty()
        && match config.rpc.mode {
            OnchainRpcMode::Custom => state.onchain_monitor().rpc_status().ready,
            OnchainRpcMode::ProviderManaged => balance_rpc_target(state, config).is_some(),
        }
}

fn balance_rpc_target(
    state: &AppState,
    config: &OnchainComparisonConfig,
) -> Option<(String, &'static str)> {
    match config.rpc.mode {
        OnchainRpcMode::Custom => {
            crate::services::onchain_rpc_registry::configured_url(&config.chain)
                .map(|url| (url, "secure_custom_rpc"))
                .or_else(|| {
                    (state
                        .onchain_monitor()
                        .snapshot()
                        .config
                        .chain
                        .eq_ignore_ascii_case(&config.chain)
                        && state.onchain_monitor().rpc_status().ready)
                        .then(|| {
                            state
                                .onchain_monitor()
                                .custom_rpc_url()
                                .map(|url| (url.as_str().to_owned(), "custom_rpc"))
                        })
                        .flatten()
                })
        }
        OnchainRpcMode::ProviderManaged if config.chain.eq_ignore_ascii_case("solana") => Some((
            super::solana_token_precision::SOLANA_MAINNET_RPC.to_owned(),
            "system_public_rpc",
        )),
        OnchainRpcMode::ProviderManaged => managed_evm_rpc_target(&config.chain),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExactAssetBalance {
    pub(super) amount_raw: u128,
    pub(super) source: &'static str,
}

pub(super) async fn exact_asset_balance(
    state: &AppState,
    config: &OnchainComparisonConfig,
    asset_address: &str,
) -> Result<ExactAssetBalance, String> {
    let rpc_url = exact_read_rpc_url(state, config).await?;
    let (url, client) = rpc_target(&rpc_url).await?;
    let (amount_raw, source) = if config.chain.eq_ignore_ascii_case("solana") {
        solana_raw_balance(
            &client,
            url.as_str(),
            &config.wallet_address,
            asset_address,
            21,
        )
        .await
    } else {
        evm_raw_balance(
            &client,
            url.as_str(),
            &config.wallet_address,
            asset_address,
            22,
        )
        .await
    }
    .map_err(|(source, problem)| format!("{problem} · source {source}"))?;
    Ok(ExactAssetBalance { amount_raw, source })
}

pub(super) async fn exact_read_rpc_url(
    state: &AppState,
    config: &OnchainComparisonConfig,
) -> Result<String, String> {
    if crate::services::onchain_rpc_registry::configured_url(&config.chain).is_some() {
        return super::execution_submit::verified_submission_rpc(state, config).await;
    }
    balance_rpc_target(state, config)
        .map(|(url, _)| url)
        .ok_or_else(|| "当前链没有可用于精确读取的已验证 RPC".to_owned())
}

fn managed_evm_rpc_target(chain: &str) -> Option<(String, &'static str)> {
    super::evm_token_identity::public_rpc(chain)
        .map(|endpoint| (endpoint.url.to_owned(), "system_public_rpc"))
}

async fn fetch(
    config: &OnchainComparisonConfig,
    rpc_url: &str,
    rpc_source: &'static str,
    started_at_ms: i64,
) -> OnchainWalletInventory {
    let target = rpc_target(rpc_url).await;
    let (base, quote, gas) = match target {
        Ok((url, client)) => {
            let base = fetch_asset(
                &client,
                url.as_str(),
                config,
                &config.base_mint,
                config.base_decimals,
                11,
            );
            let quote = fetch_asset(
                &client,
                url.as_str(),
                config,
                &config.quote_mint,
                config.quote_decimals,
                12,
            );
            let gas_address = gas_asset_address(config);
            let gas = fetch_asset(
                &client,
                url.as_str(),
                config,
                gas_address,
                gas_asset_decimals(config),
                13,
            );
            tokio::join!(base, quote, gas)
        }
        Err(problem) => (
            OnchainWalletAssetBalance::unavailable(&config.base_mint, rpc_source, problem.clone()),
            OnchainWalletAssetBalance::unavailable(&config.quote_mint, rpc_source, problem.clone()),
            OnchainWalletAssetBalance::unavailable(gas_asset_address(config), rpc_source, problem),
        ),
    };
    let mut inventory = OnchainWalletInventory {
        chain: config.chain.clone(),
        wallet_address: config.wallet_address.clone(),
        base,
        quote,
        gas,
        observed_at_ms: common::time::now_ms().max(started_at_ms),
    };
    qualify_rpc_problem(&mut inventory.base, rpc_source);
    qualify_rpc_problem(&mut inventory.quote, rpc_source);
    qualify_rpc_problem(&mut inventory.gas, rpc_source);
    inventory
}

fn qualify_rpc_problem(balance: &mut OnchainWalletAssetBalance, rpc_source: &str) {
    let Some(problem) = balance.problem.as_mut() else {
        return;
    };
    if !problem.starts_with("RPC ") {
        return;
    }
    let label = if rpc_source == "system_public_rpc" {
        "系统公共 RPC"
    } else {
        "自定义 RPC"
    };
    *problem = match problem.as_str() {
        "RPC request timed out" => format!("{label} 请求超时，系统会自动重试"),
        "RPC connection failed" => format!("{label} 连接失败，系统会自动重试"),
        "RPC transport failed" => format!("{label} 传输失败，系统会自动重试"),
        _ => format!("{label}{}", &problem[3..]),
    };
}

pub(super) fn gas_asset_address(config: &OnchainComparisonConfig) -> &'static str {
    if config.chain.eq_ignore_ascii_case("solana") {
        SOLANA_WRAPPED_SOL_MINT
    } else {
        EVM_NATIVE_TOKEN_ADDRESS
    }
}

pub(super) fn gas_asset_decimals(config: &OnchainComparisonConfig) -> u8 {
    if config.chain.eq_ignore_ascii_case("solana") {
        9
    } else {
        18
    }
}

async fn fetch_asset(
    client: &reqwest::Client,
    rpc_url: &str,
    config: &OnchainComparisonConfig,
    address: &str,
    decimals: u8,
    id: u64,
) -> OnchainWalletAssetBalance {
    let result = if config.chain.eq_ignore_ascii_case("solana") {
        solana_balance(
            client,
            rpc_url,
            &config.wallet_address,
            address,
            decimals,
            id,
        )
        .await
    } else {
        evm_balance(
            client,
            rpc_url,
            &config.wallet_address,
            address,
            decimals,
            id,
        )
        .await
    };
    match result {
        Ok((available, source)) => OnchainWalletAssetBalance::available(address, available, source),
        Err((source, problem)) => OnchainWalletAssetBalance::unavailable(address, source, problem),
    }
}

async fn solana_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    wallet: &str,
    mint: &str,
    decimals: u8,
    id: u64,
) -> Result<(f64, &'static str), (&'static str, String)> {
    let (raw, source) = solana_raw_balance(client, rpc_url, wallet, mint, id).await?;
    units(raw, decimals)
        .map(|available| (available, source))
        .ok_or_else(|| (source, "Solana balance cannot be represented".to_owned()))
}

async fn solana_raw_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    wallet: &str,
    mint: &str,
    id: u64,
) -> Result<(u128, &'static str), (&'static str, String)> {
    if mint == SOLANA_WRAPPED_SOL_MINT {
        let value = rpc_result(
            client,
            rpc_url,
            "getBalance",
            serde_json::json!([wallet, { "commitment": "confirmed" }]),
            id,
        )
        .await
        .map_err(|problem| (SOLANA_BALANCE_DOCS, problem))?;
        let raw = value
            .get("value")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                (
                    SOLANA_BALANCE_DOCS,
                    "Solana getBalance response is missing a u64 value".to_owned(),
                )
            })?;
        return Ok((u128::from(raw), SOLANA_BALANCE_DOCS));
    }
    let value = rpc_result(
        client,
        rpc_url,
        "getTokenAccountsByOwner",
        serde_json::json!([
            wallet,
            { "mint": mint },
            { "commitment": "confirmed", "encoding": "jsonParsed" }
        ]),
        id,
    )
    .await
    .map_err(|problem| (SOLANA_TOKEN_BALANCE_DOCS, problem))?;
    let rows = value
        .get("value")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            (
                SOLANA_TOKEN_BALANCE_DOCS,
                "Solana token balance response is missing value rows".to_owned(),
            )
        })?;
    let raw = rows.iter().try_fold(0_u128, |total, row| {
        row.pointer("/account/data/parsed/info/tokenAmount/amount")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<u128>().ok())
            .and_then(|value| total.checked_add(value))
    });
    raw.map(|raw| (raw, SOLANA_TOKEN_BALANCE_DOCS))
        .ok_or_else(|| {
            (
                SOLANA_TOKEN_BALANCE_DOCS,
                "Solana token account amount is invalid or exceeds supported precision".to_owned(),
            )
        })
}

async fn evm_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    wallet: &str,
    token: &str,
    decimals: u8,
    id: u64,
) -> Result<(f64, &'static str), (&'static str, String)> {
    let (raw, source) = evm_raw_balance(client, rpc_url, wallet, token, id).await?;
    units(raw, decimals)
        .map(|available| (available, source))
        .ok_or_else(|| (source, "EVM balance cannot be represented".to_owned()))
}

async fn evm_raw_balance(
    client: &reqwest::Client,
    rpc_url: &str,
    wallet: &str,
    token: &str,
    id: u64,
) -> Result<(u128, &'static str), (&'static str, String)> {
    let (method, params, source) = if token.eq_ignore_ascii_case(EVM_NATIVE_TOKEN_ADDRESS) {
        (
            "eth_getBalance",
            serde_json::json!([wallet, "latest"]),
            EVM_BALANCE_DOCS,
        )
    } else {
        let calldata = erc20_balance_of_calldata(wallet).ok_or_else(|| {
            (
                EVM_CALL_DOCS,
                "EVM wallet address cannot compile ERC-20 balanceOf calldata".to_owned(),
            )
        })?;
        (
            "eth_call",
            serde_json::json!([{ "to": token, "data": calldata }, "latest"]),
            EVM_CALL_DOCS,
        )
    };
    let value = rpc_result(client, rpc_url, method, params, id)
        .await
        .map_err(|problem| (source, problem))?;
    let raw = value.as_str().and_then(parse_hex_quantity).ok_or_else(|| {
        (
            source,
            format!("{method} response is not a supported hex quantity"),
        )
    })?;
    Ok((raw, source))
}

fn erc20_balance_of_calldata(wallet: &str) -> Option<String> {
    let address = wallet.strip_prefix("0x")?;
    (address.len() == 40 && address.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("0x70a08231{}{}", "0".repeat(12 * 2), address))
}

fn parse_hex_quantity(value: &str) -> Option<u128> {
    let hex = value.strip_prefix("0x")?;
    (!hex.is_empty())
        .then(|| u128::from_str_radix(hex, 16).ok())
        .flatten()
}

fn units(raw: u128, decimals: u8) -> Option<f64> {
    let scale = 10_f64.powi(i32::from(decimals));
    let value = raw as f64 / scale;
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erc20_balance_call_pads_the_wallet_to_one_abi_word() {
        let wallet = "0x00000000000000000000000000000000000000ab";
        let calldata = erc20_balance_of_calldata(wallet);
        assert!(calldata.is_some());
        let calldata = calldata.unwrap_or_default();
        assert_eq!(calldata.len(), 2 + 8 + 64);
        assert!(calldata.ends_with("00000000000000000000000000000000000000ab"));
    }

    #[test]
    fn zero_balances_remain_proven_zero() {
        assert_eq!(units(0, 18), Some(0.0));
        assert_eq!(parse_hex_quantity("0x0"), Some(0));
    }

    #[test]
    fn managed_mode_reuses_the_same_public_evm_rpc_as_contract_identity() {
        assert_eq!(
            managed_evm_rpc_target("base"),
            Some(("https://mainnet.base.org".to_owned(), "system_public_rpc"))
        );
        assert!(managed_evm_rpc_target("unsupported").is_none());
    }

    #[test]
    fn wallet_problem_names_the_active_rpc_mode() {
        let mut balance = OnchainWalletAssetBalance::unavailable(
            "mint",
            SOLANA_BALANCE_DOCS,
            "RPC request timed out",
        );
        qualify_rpc_problem(&mut balance, "system_public_rpc");
        assert_eq!(
            balance.problem.as_deref(),
            Some("系统公共 RPC 请求超时，系统会自动重试")
        );
    }
}
