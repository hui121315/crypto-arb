use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use shared_types::OnchainUnsignedTransaction;

use super::venue_credentials;

pub(crate) const SOLANA_SIGNER_ID: &str = "solana_wallet_signer";
pub(crate) const EVM_SIGNER_ID: &str = "evm_wallet_signer";
pub(crate) const SOLANA_PRIVATE_KEY_ENV: &str = "ONCHAIN_SOLANA_PRIVATE_KEY";
pub(crate) const EVM_PRIVATE_KEY_ENV: &str = "ONCHAIN_EVM_PRIVATE_KEY";

pub(crate) fn validate_provider_secret(provider: &str, value: &str) -> Result<(), String> {
    match provider {
        SOLANA_SIGNER_ID => common::signing::solana_address_from_keypair(value)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        EVM_SIGNER_ID => common::signing::secp256k1_address(value)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        _ => Ok(()),
    }
}

pub(crate) fn readiness(chain: &str, wallet_address: &str) -> Result<(), String> {
    let (env_key, derived) = if is_solana(chain) {
        let secret = secret(SOLANA_PRIVATE_KEY_ENV, "Solana")?;
        (
            SOLANA_PRIVATE_KEY_ENV,
            common::signing::solana_address_from_keypair(&secret)
                .map_err(|error| error.to_string())?,
        )
    } else {
        let secret = secret(EVM_PRIVATE_KEY_ENV, "EVM")?;
        (
            EVM_PRIVATE_KEY_ENV,
            format!(
                "0x{}",
                hex::encode(
                    common::signing::secp256k1_address(&secret)
                        .map_err(|error| error.to_string())?
                )
            ),
        )
    };
    if !addresses_equal(chain, &derived, wallet_address) {
        return Err(format!(
            "{env_key} 对应钱包 {derived}，与当前配置 {wallet_address} 不一致"
        ));
    }
    Ok(())
}

pub(crate) fn sign(
    chain: &str,
    wallet_address: &str,
    transaction: &OnchainUnsignedTransaction,
    evm_nonce: Option<&str>,
    evm_gas_price: Option<&str>,
) -> Result<String, String> {
    readiness(chain, wallet_address)?;
    match transaction {
        OnchainUnsignedTransaction::SolanaVersioned {
            transaction_base64, ..
        } => {
            let secret = secret(SOLANA_PRIVATE_KEY_ENV, "Solana")?;
            sign_solana_transaction(transaction_base64, wallet_address, &secret)
        }
        OnchainUnsignedTransaction::EvmCall { .. } => {
            let secret = secret(EVM_PRIVATE_KEY_ENV, "EVM")?;
            let nonce = evm_nonce.ok_or_else(|| "EVM nonce 未读取".to_owned())?;
            sign_evm_legacy_transaction(transaction, wallet_address, &secret, nonce, evm_gas_price)
        }
    }
}

pub(crate) fn solana_transaction_id(signed_transaction_base64: &str) -> Result<String, String> {
    let bytes = B64_STANDARD
        .decode(signed_transaction_base64)
        .map_err(|error| format!("Solana 已签名交易 base64 非法：{error}"))?;
    let (signature_count, prefix_len) = decode_short_vec(&bytes, 0)?;
    if signature_count == 0 || bytes.len() < prefix_len + 64 {
        return Err("Solana 已签名交易缺少 fee payer 签名".to_owned());
    }
    Ok(bs58::encode(&bytes[prefix_len..prefix_len + 64]).into_string())
}

fn secret(env_key: &str, label: &str) -> Result<String, String> {
    venue_credentials::secret(env_key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{label} 钱包签名器未配置"))
}

fn is_solana(chain: &str) -> bool {
    chain.eq_ignore_ascii_case("solana")
}

fn addresses_equal(chain: &str, left: &str, right: &str) -> bool {
    if is_solana(chain) {
        left.trim() == right.trim()
    } else {
        left.trim().eq_ignore_ascii_case(right.trim())
    }
}

fn sign_solana_transaction(
    transaction_base64: &str,
    wallet_address: &str,
    secret_value: &str,
) -> Result<String, String> {
    let mut bytes = B64_STANDARD
        .decode(transaction_base64)
        .map_err(|error| format!("Solana 交易 base64 非法：{error}"))?;
    let (signature_count, signature_prefix_len) = decode_short_vec(&bytes, 0)?;
    let signatures_len = signature_count
        .checked_mul(64)
        .ok_or_else(|| "Solana 签名数量溢出".to_owned())?;
    let message_offset = signature_prefix_len
        .checked_add(signatures_len)
        .filter(|offset| *offset < bytes.len())
        .ok_or_else(|| "Solana 交易缺少序列化消息".to_owned())?;
    let message = &bytes[message_offset..];
    let (header_offset, version) = if message[0] & 0x80 != 0 {
        (1, message[0] & 0x7f)
    } else {
        (0, 0)
    };
    if version != 0 {
        return Err(format!("暂不支持 Solana v{version} 交易"));
    }
    if message.len() < header_offset + 3 {
        return Err("Solana 消息 header 不完整".to_owned());
    }
    let required_signatures = usize::from(message[header_offset]);
    if required_signatures != signature_count {
        return Err("Solana 签名槽数量与 message header 不一致".to_owned());
    }
    let account_count_offset = header_offset + 3;
    let (account_count, account_count_len) = decode_short_vec(message, account_count_offset)?;
    let account_keys_offset = account_count_offset + account_count_len;
    let account_keys_len = account_count
        .checked_mul(32)
        .ok_or_else(|| "Solana 账户数量溢出".to_owned())?;
    if account_keys_offset + account_keys_len > message.len() {
        return Err("Solana 消息账户列表不完整".to_owned());
    }
    let expected = bs58::decode(wallet_address.trim())
        .into_vec()
        .map_err(|_| "当前 Solana 钱包地址不是合法 base58".to_owned())?;
    if expected.len() != 32 {
        return Err("当前 Solana 钱包地址长度非法".to_owned());
    }
    let signer_index = (0..required_signatures)
        .find(|index| {
            let start = account_keys_offset + index * 32;
            message[start..start + 32] == expected
        })
        .ok_or_else(|| "Jupiter 交易的签名账户中不包含当前钱包".to_owned())?;
    let secret =
        common::signing::decode_solana_keypair(secret_value).map_err(|error| error.to_string())?;
    let signature =
        common::signing::ed25519_sign_bytes(&secret, message).map_err(|error| error.to_string())?;
    let slot = signature_prefix_len + signer_index * 64;
    bytes[slot..slot + 64].copy_from_slice(&signature);
    Ok(B64_STANDARD.encode(bytes))
}

pub(super) fn decode_short_vec(bytes: &[u8], offset: usize) -> Result<(usize, usize), String> {
    let mut value = 0_usize;
    let mut shift = 0_u32;
    for consumed in 0..3 {
        let byte = *bytes
            .get(offset + consumed)
            .ok_or_else(|| "compact-u16 长度不完整".to_owned())?;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, consumed + 1));
        }
        shift += 7;
    }
    Err("compact-u16 长度超过协议上限".to_owned())
}

fn sign_evm_legacy_transaction(
    transaction: &OnchainUnsignedTransaction,
    wallet_address: &str,
    private_key: &str,
    nonce: &str,
    fallback_gas_price: Option<&str>,
) -> Result<String, String> {
    let OnchainUnsignedTransaction::EvmCall {
        chain_id,
        from,
        to,
        data,
        value,
        gas,
        gas_price,
        ..
    } = transaction
    else {
        return Err("不是 EVM 交易".to_owned());
    };
    if !from.eq_ignore_ascii_case(wallet_address) {
        return Err("EVM firm quote 的 from 与当前钱包不一致".to_owned());
    }
    let derived = format!(
        "0x{}",
        hex::encode(common::signing::secp256k1_address(private_key).map_err(|e| e.to_string())?)
    );
    if !derived.eq_ignore_ascii_case(wallet_address) {
        return Err(format!(
            "EVM 私钥对应钱包 {derived}，与当前配置 {wallet_address} 不一致"
        ));
    }
    let gas_price = gas_price
        .as_deref()
        .or(fallback_gas_price)
        .ok_or_else(|| "firm quote 与 RPC 均未返回 gasPrice".to_owned())?;
    let nonce = quantity_bytes(nonce)?;
    let gas_price = quantity_bytes(gas_price)?;
    let gas = quantity_bytes(gas)?;
    let to = fixed_hex_bytes(to, 20, "to")?;
    let value = quantity_bytes(value)?;
    let data = unformatted_hex_bytes(data, "data")?;
    let chain = u64_bytes(*chain_id);
    let signing_payload = rlp_list(&[
        &nonce,
        &gas_price,
        &gas,
        &to,
        &value,
        &data,
        &chain,
        &[],
        &[],
    ]);
    let signature = common::signing::secp256k1_sign_prehash_recoverable(
        private_key,
        common::signing::keccak256(&signing_payload),
    )
    .map_err(|error| error.to_string())?;
    let v = chain_id
        .checked_mul(2)
        .and_then(|value| value.checked_add(35 + u64::from(signature.recovery_id)))
        .ok_or_else(|| "EVM chain id 溢出".to_owned())?;
    let v = u64_bytes(v);
    let r = trim_leading_zeroes(&signature.r);
    let s = trim_leading_zeroes(&signature.s);
    let signed = rlp_list(&[&nonce, &gas_price, &gas, &to, &value, &data, &v, r, s]);
    Ok(format!("0x{}", hex::encode(signed)))
}

fn quantity_bytes(value: &str) -> Result<Vec<u8>, String> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix("0x") {
        return decode_hex_quantity(hex);
    }
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("EVM quantity 非法：{value}"));
    }
    let mut bytes = vec![0_u8];
    for digit in value.bytes().map(|byte| byte - b'0') {
        let mut carry = u16::from(digit);
        for byte in bytes.iter_mut().rev() {
            let next = u16::from(*byte) * 10 + carry;
            *byte = next as u8;
            carry = next >> 8;
        }
        while carry > 0 {
            bytes.insert(0, carry as u8);
            carry >>= 8;
        }
    }
    Ok(trim_leading_zeroes(&bytes).to_vec())
}

fn decode_hex_quantity(value: &str) -> Result<Vec<u8>, String> {
    if value.is_empty() {
        return Err("EVM hex quantity 缺少数字".to_owned());
    }
    let padded;
    let value = if value.len() % 2 == 1 {
        padded = format!("0{value}");
        padded.as_str()
    } else {
        value
    };
    let bytes = hex::decode(value).map_err(|_| "EVM hex quantity 非法".to_owned())?;
    Ok(trim_leading_zeroes(&bytes).to_vec())
}

fn fixed_hex_bytes(value: &str, length: usize, field: &str) -> Result<Vec<u8>, String> {
    let bytes = unformatted_hex_bytes(value, field)?;
    if bytes.len() != length {
        return Err(format!("EVM {field} 长度应为 {length} 字节"));
    }
    Ok(bytes)
}

fn unformatted_hex_bytes(value: &str, field: &str) -> Result<Vec<u8>, String> {
    let value = value
        .trim()
        .strip_prefix("0x")
        .ok_or_else(|| format!("EVM {field} 必须以 0x 开头"))?;
    if value.len() % 2 != 0 {
        return Err(format!("EVM {field} hex 长度必须为偶数"));
    }
    hex::decode(value).map_err(|_| format!("EVM {field} hex 非法"))
}

fn u64_bytes(value: u64) -> Vec<u8> {
    trim_leading_zeroes(&value.to_be_bytes()).to_vec()
}

fn trim_leading_zeroes(bytes: &[u8]) -> &[u8] {
    let first = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    &bytes[first..]
}

fn rlp_list(fields: &[&[u8]]) -> Vec<u8> {
    let payload = fields
        .iter()
        .flat_map(|field| rlp_bytes(field))
        .collect::<Vec<_>>();
    let mut encoded = rlp_length_prefix(payload.len(), 0xc0, 0xf7);
    encoded.extend_from_slice(&payload);
    encoded
}

fn rlp_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return bytes.to_vec();
    }
    let mut encoded = rlp_length_prefix(bytes.len(), 0x80, 0xb7);
    encoded.extend_from_slice(bytes);
    encoded
}

fn rlp_length_prefix(length: usize, short_base: u8, long_base: u8) -> Vec<u8> {
    if length < 56 {
        return vec![short_base + length as u8];
    }
    let length = u64_bytes(length as u64);
    let mut prefix = vec![long_base + length.len() as u8];
    prefix.extend_from_slice(&length);
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solana_versioned_transaction_signs_only_the_matching_slot() {
        let seed = [7_u8; 32];
        let secret = bs58::encode(seed).into_string();
        let wallet = common::signing::solana_address_from_keypair(&secret).expect("wallet");
        let public_key = bs58::decode(&wallet).into_vec().expect("pubkey");
        let mut message = vec![0x80, 1, 0, 0, 1];
        message.extend_from_slice(&public_key);
        message.extend_from_slice(&[0_u8; 32]);
        message.extend_from_slice(&[0, 0]);
        let mut transaction = vec![1];
        transaction.extend_from_slice(&[0_u8; 64]);
        transaction.extend_from_slice(&message);

        let signed = sign_solana_transaction(&B64_STANDARD.encode(&transaction), &wallet, &secret)
            .expect("signed transaction");
        let signed = B64_STANDARD.decode(signed).expect("base64");
        assert_eq!(&signed[65..], message.as_slice());
        assert_eq!(
            &signed[1..65],
            common::signing::ed25519_sign_bytes(&seed, &message)
                .expect("signature")
                .as_slice()
        );
        assert_eq!(
            solana_transaction_id(&B64_STANDARD.encode(&signed)).expect("transaction id"),
            bs58::encode(&signed[1..65]).into_string()
        );
    }

    #[test]
    fn evm_legacy_signing_matches_eip_155_official_vector() {
        let private_key = "4646464646464646464646464646464646464646464646464646464646464646";
        let wallet = format!(
            "0x{}",
            hex::encode(common::signing::secp256k1_address(private_key).expect("address"))
        );
        let transaction = OnchainUnsignedTransaction::EvmCall {
            chain_id: 1,
            from: wallet.clone(),
            to: "0x3535353535353535353535353535353535353535".to_owned(),
            data: "0x".to_owned(),
            value: "1000000000000000000".to_owned(),
            gas: "21000".to_owned(),
            gas_price: Some("20000000000".to_owned()),
            max_priority_fee_per_gas: None,
            allowance_spender: None,
        };
        assert_eq!(
            sign_evm_legacy_transaction(&transaction, &wallet, private_key, "9", None)
                .expect("signed transaction"),
            "0xf86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"
        );
    }

    #[test]
    fn decimal_quantities_are_not_truncated_to_u64() {
        assert_eq!(
            hex::encode(quantity_bytes("1000000000000000000000000").expect("quantity")),
            "d3c21bcecceda1000000"
        );
    }
}
