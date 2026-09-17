use shared_types::{OnchainExecutionToken, EVM_NATIVE_TOKEN_ADDRESS};

pub(super) fn symbol(chain: &str, token: &OnchainExecutionToken) -> Result<String, String> {
    let same = |a: &str, b: &str| {
        if chain == "solana" {
            a == b
        } else {
            a.eq_ignore_ascii_case(b)
        }
    };
    let symbol = token.symbol.to_ascii_uppercase();
    let preset = shared_types::onchain_chain_preset(chain).ok_or("折算链身份未知")?;
    if symbol == preset.base_token
        && token.decimals == preset.base_decimals
        && (same(&token.address, preset.base_address)
            || preset.chain_id.is_some()
                && token.address == "0x0000000000000000000000000000000000000000"
                && preset.base_address == EVM_NATIVE_TOKEN_ADDRESS)
    {
        return Ok(symbol);
    }
    // Circle mainnet registry: https://developers.circle.com/stablecoins/usdc-contract-addresses
    // The BNB preset is a Binance-pegged representation, not Circle-issued USDC.
    if symbol == "USDC"
        && chain != "bnb-smart-chain"
        && token.decimals == 6
        && same(&token.address, preset.quote_address)
    {
        return Ok(symbol);
    }
    // Issuer contract list: https://tether.to/en/supported-protocols/
    let usdt = match chain {
        "ethereum" => Some("0xdac17f958d2ee523a2206206994597c13d831ec7"),
        "avalanche" => Some("0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7"),
        "solana" => Some("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"),
        _ => None,
    };
    if symbol == "USDT"
        && token.decimals == 6
        && usdt.is_some_and(|address| same(&token.address, address))
    {
        return Ok(symbol);
    }
    Err(format!(
        "{chain} {symbol} 合约 {} 缺少美元市场资产映射；原币收支已保留，不按同名币或 1 美元估值",
        token.address
    ))
}
