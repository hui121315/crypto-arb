//! Hyperliquid L1 action signing.
//!
//! Official SDK references:
//! - <https://github.com/hyperliquid-dex/hyperliquid-python-sdk/blob/master/hyperliquid/utils/signing.py>
//! - <https://github.com/hyperliquid-dex/hyperliquid-python-sdk/blob/master/tests/signing_test.py>

#[path = "hyperliquid_action_msgpack.rs"]
mod action_msgpack;

use k256::ecdsa::SigningKey;
use serde_json::Value;
use sha3::{Digest, Keccak256};
use thiserror::Error;

const ZERO_ADDRESS: [u8; 20] = [0_u8; 20];
const EIP712_DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const AGENT_TYPE: &str = "Agent(string source,bytes32 connectionId)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyperliquidNetwork {
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperliquidSignature {
    pub r: String,
    pub s: String,
    pub v: u8,
}

#[derive(Debug, Error)]
pub enum HyperliquidSigningError {
    #[error("invalid msgpack action: {0}")]
    Msgpack(String),
    #[error("invalid hex private key")]
    InvalidPrivateKey,
    #[error("invalid vault address")]
    InvalidVaultAddress,
    #[error("ecdsa signing failed")]
    Ecdsa,
}

pub fn sign_l1_action(
    private_key_hex: &str,
    action: &Value,
    nonce: u64,
    vault_address: Option<&str>,
    expires_after: Option<u64>,
    network: HyperliquidNetwork,
) -> Result<HyperliquidSignature, HyperliquidSigningError> {
    let connection_id = l1_action_hash(action, nonce, vault_address, expires_after)?;
    let digest = agent_typed_data_hash(connection_id, network);
    sign_digest(private_key_hex, digest)
}

pub fn l1_action_hash(
    action: &Value,
    nonce: u64,
    vault_address: Option<&str>,
    expires_after: Option<u64>,
) -> Result<[u8; 32], HyperliquidSigningError> {
    // Hyperliquid hashes msgpack bytes, so map insertion order is part of the
    // signing contract. Keep this local to L1 actions instead of changing the
    // workspace-wide serde_json map representation.
    let mut bytes = action_msgpack::to_vec(action)
        .map_err(|error| HyperliquidSigningError::Msgpack(error.to_string()))?;
    bytes.extend_from_slice(&nonce.to_be_bytes());
    append_vault_marker(&mut bytes, vault_address)?;
    if let Some(expires_after) = expires_after {
        bytes.push(0);
        bytes.extend_from_slice(&expires_after.to_be_bytes());
    }
    Ok(keccak256(&bytes))
}

pub fn agent_typed_data_hash(connection_id: [u8; 32], network: HyperliquidNetwork) -> [u8; 32] {
    let domain = domain_separator("Exchange", "1", 1_337, ZERO_ADDRESS);
    let message = agent_struct_hash(network.source(), connection_id);
    let mut bytes = Vec::with_capacity(66);
    bytes.extend_from_slice(b"\x19\x01");
    bytes.extend_from_slice(&domain);
    bytes.extend_from_slice(&message);
    keccak256(&bytes)
}

pub fn address_from_private_key(private_key_hex: &str) -> Result<String, HyperliquidSigningError> {
    let key_bytes = decode_private_key(private_key_hex)?;
    let signing_key = SigningKey::from_slice(&key_bytes)
        .map_err(|_| HyperliquidSigningError::InvalidPrivateKey)?;
    let public_key = signing_key.verifying_key().to_encoded_point(false);
    let bytes = public_key.as_bytes();
    if bytes.len() != 65 || bytes[0] != 4 {
        return Err(HyperliquidSigningError::InvalidPrivateKey);
    }
    let hash = keccak256(&bytes[1..]);
    Ok(format!("0x{}", hex::encode(&hash[12..])))
}

impl HyperliquidNetwork {
    pub const fn source(self) -> &'static str {
        match self {
            Self::Mainnet => "a",
            Self::Testnet => "b",
        }
    }
}

fn sign_digest(
    private_key_hex: &str,
    digest: [u8; 32],
) -> Result<HyperliquidSignature, HyperliquidSigningError> {
    let key_bytes = decode_private_key(private_key_hex)?;
    let signing_key = SigningKey::from_slice(&key_bytes)
        .map_err(|_| HyperliquidSigningError::InvalidPrivateKey)?;
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .map_err(|_| HyperliquidSigningError::Ecdsa)?;
    Ok(HyperliquidSignature {
        r: hex_quantity(&signature.r().to_bytes()),
        s: hex_quantity(&signature.s().to_bytes()),
        v: 27 + recovery_id.to_byte(),
    })
}

fn hex_quantity(bytes: &[u8]) -> String {
    let encoded = hex::encode(bytes);
    let quantity = encoded.trim_start_matches('0');
    format!("0x{}", if quantity.is_empty() { "0" } else { quantity })
}

fn append_vault_marker(
    bytes: &mut Vec<u8>,
    vault_address: Option<&str>,
) -> Result<(), HyperliquidSigningError> {
    if let Some(address) = vault_address {
        bytes.push(1);
        bytes.extend_from_slice(&decode_address(address)?);
    } else {
        bytes.push(0);
    }
    Ok(())
}

fn agent_struct_hash(source: &str, connection_id: [u8; 32]) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(96);
    bytes.extend_from_slice(&keccak256(AGENT_TYPE.as_bytes()));
    bytes.extend_from_slice(&keccak256(source.as_bytes()));
    bytes.extend_from_slice(&connection_id);
    keccak256(&bytes)
}

fn domain_separator(
    name: &str,
    version: &str,
    chain_id: u64,
    verifying_contract: [u8; 20],
) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(160);
    bytes.extend_from_slice(&keccak256(EIP712_DOMAIN_TYPE.as_bytes()));
    bytes.extend_from_slice(&keccak256(name.as_bytes()));
    bytes.extend_from_slice(&keccak256(version.as_bytes()));
    bytes.extend_from_slice(&uint256(chain_id));
    bytes.extend_from_slice(&address_word(verifying_contract));
    keccak256(&bytes)
}

fn decode_private_key(value: &str) -> Result<[u8; 32], HyperliquidSigningError> {
    let cleaned = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(cleaned).map_err(|_| HyperliquidSigningError::InvalidPrivateKey)?;
    bytes
        .try_into()
        .map_err(|_| HyperliquidSigningError::InvalidPrivateKey)
}

fn decode_address(value: &str) -> Result<[u8; 20], HyperliquidSigningError> {
    let cleaned = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(cleaned).map_err(|_| HyperliquidSigningError::InvalidVaultAddress)?;
    bytes
        .try_into()
        .map_err(|_| HyperliquidSigningError::InvalidVaultAddress)
}

fn uint256(value: u64) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out[24..].copy_from_slice(&value.to_be_bytes());
    out
}

fn address_word(address: [u8; 20]) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out[12..].copy_from_slice(&address);
    out
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    Keccak256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn action_hash_is_stable_and_32_bytes() {
        let action = json!({"type": "cancel", "cancels": [{"a": 0, "o": 123}]});
        let first = l1_action_hash(&action, 1_700_000_000_000, None, None).expect("hash");
        let second = l1_action_hash(&action, 1_700_000_000_000, None, None).expect("hash");
        assert_eq!(first, second);
    }

    #[test]
    fn signs_l1_action_with_recoverable_signature_shape() {
        let action = json!({"type": "noop"});
        let signature = sign_l1_action(
            "0101010101010101010101010101010101010101010101010101010101010101",
            &action,
            1_700_000_000_000,
            None,
            None,
            HyperliquidNetwork::Testnet,
        )
        .expect("sign");
        assert!(signature.r.starts_with("0x") && signature.r.len() <= 66);
        assert!(signature.s.starts_with("0x") && signature.s.len() <= 66);
        assert!((27..=28).contains(&signature.v));
    }

    #[test]
    fn matches_official_python_sdk_l1_action_signing_vectors() {
        let private_key = "0123456789012345678901234567890123456789012345678901234567890123";
        let action = json!({"type": "dummy", "num": 100_000_000_000_u64});

        let mainnet = sign_l1_action(
            private_key,
            &action,
            0,
            None,
            None,
            HyperliquidNetwork::Mainnet,
        )
        .expect("mainnet SDK vector");
        assert_eq!(
            mainnet,
            HyperliquidSignature {
                r: "0x53749d5b30552aeb2fca34b530185976545bb22d0b3ce6f62e31be961a59298".to_owned(),
                s: "0x755c40ba9bf05223521753995abb2f73ab3229be8ec921f350cb447e384d8ed8".to_owned(),
                v: 27,
            }
        );

        let testnet = sign_l1_action(
            private_key,
            &action,
            0,
            None,
            None,
            HyperliquidNetwork::Testnet,
        )
        .expect("testnet SDK vector");
        assert_eq!(
            testnet,
            HyperliquidSignature {
                r: "0x542af61ef1f429707e3c76c5293c80d01f74ef853e34b76efffcb57e574f9510".to_owned(),
                s: "0x17b8b32f086e8cdede991f1e2c529f5dd5297cbe8128500e00cbaf766204a613".to_owned(),
                v: 28,
            }
        );
    }

    #[test]
    fn matches_official_python_sdk_order_signing_vectors() {
        let private_key = "0123456789012345678901234567890123456789012345678901234567890123";
        let action = json!({
            "type": "order",
            "orders": [{
                "a": 1,
                "b": true,
                "p": "100",
                "s": "100",
                "r": false,
                "t": {"limit": {"tif": "Gtc"}},
            }],
            "grouping": "na",
        });

        let mainnet = sign_l1_action(
            private_key,
            &action,
            0,
            None,
            None,
            HyperliquidNetwork::Mainnet,
        )
        .expect("mainnet order SDK vector");
        assert_eq!(
            mainnet,
            HyperliquidSignature {
                r: "0xd65369825a9df5d80099e513cce430311d7d26ddf477f5b3a33d2806b100d78e".to_owned(),
                s: "0x2b54116ff64054968aa237c20ca9ff68000f977c93289157748a3162b6ea940e".to_owned(),
                v: 28,
            }
        );

        let testnet = sign_l1_action(
            private_key,
            &action,
            0,
            None,
            None,
            HyperliquidNetwork::Testnet,
        )
        .expect("testnet order SDK vector");
        assert_eq!(
            testnet,
            HyperliquidSignature {
                r: "0x82b2ba28e76b3d761093aaded1b1cdad4960b3af30212b343fb2e6cdfa4e3d54".to_owned(),
                s: "0x6b53878fc99d26047f4d7e8c90eb98955a109f44209163f52d8dc4278cbbd9f5".to_owned(),
                v: 27,
            }
        );
    }

    #[test]
    fn matches_official_python_sdk_order_with_cloid_signing_vectors() {
        let private_key = "0123456789012345678901234567890123456789012345678901234567890123";
        let action = json!({
            "type": "order",
            "orders": [{
                "a": 1,
                "b": true,
                "p": "100",
                "s": "100",
                "r": false,
                "t": {"limit": {"tif": "Gtc"}},
                "c": "0x00000000000000000000000000000001",
            }],
            "grouping": "na",
        });

        let mainnet = sign_l1_action(
            private_key,
            &action,
            0,
            None,
            None,
            HyperliquidNetwork::Mainnet,
        )
        .expect("mainnet cloid SDK vector");
        assert_eq!(
            mainnet,
            HyperliquidSignature {
                r: "0x41ae18e8239a56cacbc5dad94d45d0b747e5da11ad564077fcac71277a946e3".to_owned(),
                s: "0x3c61f667e747404fe7eea8f90ab0e76cc12ce60270438b2058324681a00116da".to_owned(),
                v: 27,
            }
        );

        let testnet = sign_l1_action(
            private_key,
            &action,
            0,
            None,
            None,
            HyperliquidNetwork::Testnet,
        )
        .expect("testnet cloid SDK vector");
        assert_eq!(
            testnet,
            HyperliquidSignature {
                r: "0xeba0664bed2676fc4e5a743bf89e5c7501aa6d870bdb9446e122c9466c5cd16d".to_owned(),
                s: "0x7f3e74825c9114bc59086f1eebea2928c190fdfbfde144827cb02b85bbe90988".to_owned(),
                v: 28,
            }
        );
    }

    #[test]
    fn rejects_unsupported_l1_action_before_signing() {
        let error = l1_action_hash(&json!({"type": "futureAction", "value": 1}), 0, None, None)
            .expect_err("unknown action schema must fail closed");

        assert!(matches!(error, HyperliquidSigningError::Msgpack(_)));
        assert!(error
            .to_string()
            .contains("unsupported Hyperliquid L1 action"));
    }

    #[test]
    fn rejects_invalid_vault_address() {
        let action = json!({"type": "noop"});
        let error = l1_action_hash(&action, 1, Some("0x1234"), None).unwrap_err();
        assert!(matches!(
            error,
            HyperliquidSigningError::InvalidVaultAddress
        ));
    }

    #[test]
    fn derives_evm_address_from_private_key() {
        let address = address_from_private_key(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("address");

        assert_eq!(address, "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf");
    }
}
