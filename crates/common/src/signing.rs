//! 加密 / 签名工具。
//!
//! 仅提供低层原语（HMAC、SHA、Base64、Ed25519），各交易所的具体签名规则在
//! `exchange` crate 内组装。

use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, Mac};
use k256::ecdsa::SigningKey as Secp256k1SigningKey;
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use sha3::{Digest, Keccak256};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;
type HmacSha512 = Hmac<Sha512>;
type HmacSha1 = Hmac<Sha1>;

#[derive(Debug, Error)]
pub enum SigningError {
    #[error("invalid base64 secret: {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("ed25519 secret must be 32-byte seed or 64-byte expanded key")]
    InvalidEd25519Secret,
    #[error("invalid Solana keypair; expected base58 or JSON array with 32/64 bytes")]
    InvalidSolanaKeypair,
    #[error("invalid secp256k1 private key")]
    InvalidSecp256k1Secret,
    #[error("recoverable secp256k1 signing failed")]
    Secp256k1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoverableSecp256k1Signature {
    pub r: [u8; 32],
    pub s: [u8; 32],
    pub recovery_id: u8,
}

/// HMAC-SHA256 → 十六进制小写。
pub fn hmac_sha256_hex(secret: &[u8], message: &[u8]) -> String {
    let mut mac = hmac_sha256(secret);
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

/// HMAC-SHA256 → Base64。
pub fn hmac_sha256_base64(secret: &[u8], message: &[u8]) -> String {
    let mut mac = hmac_sha256(secret);
    mac.update(message);
    B64_STANDARD.encode(mac.finalize().into_bytes())
}

/// HMAC-SHA512 → 十六进制小写。
pub fn hmac_sha512_hex(secret: &[u8], message: &[u8]) -> String {
    let mut mac = hmac_sha512(secret);
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

/// HMAC-SHA512 -> Base64.
pub fn hmac_sha512_base64(secret: &[u8], message: &[u8]) -> String {
    let mut mac = hmac_sha512(secret);
    mac.update(message);
    B64_STANDARD.encode(mac.finalize().into_bytes())
}

/// Decode a standard Base64-encoded secret for venue-specific signing.
pub fn decode_base64(value: &str) -> Result<Vec<u8>, SigningError> {
    Ok(B64_STANDARD.decode(value)?)
}

/// HMAC-SHA1 → Base64（部分老接口仍在使用）。
pub fn hmac_sha1_base64(secret: &[u8], message: &[u8]) -> String {
    let mut mac = hmac_sha1(secret);
    mac.update(message);
    B64_STANDARD.encode(mac.finalize().into_bytes())
}

pub fn ed25519_sign_base64(secret_base64: &str, message: &[u8]) -> Result<String, SigningError> {
    let secret = B64_STANDARD.decode(secret_base64)?;
    ed25519_sign_bytes_base64(&secret, message)
}

pub fn ed25519_sign_bytes_base64(secret: &[u8], message: &[u8]) -> Result<String, SigningError> {
    Ok(B64_STANDARD.encode(ed25519_sign_bytes(secret, message)?))
}

pub fn ed25519_sign_bytes(secret: &[u8], message: &[u8]) -> Result<[u8; 64], SigningError> {
    let seed = ed25519_seed(secret)?;
    let signing_key = SigningKey::from_bytes(&seed);
    Ok(signing_key.sign(message).to_bytes())
}

pub fn ed25519_public_key(secret: &[u8]) -> Result<[u8; 32], SigningError> {
    let seed = ed25519_seed(secret)?;
    Ok(SigningKey::from_bytes(&seed).verifying_key().to_bytes())
}

pub fn decode_solana_keypair(value: &str) -> Result<Vec<u8>, SigningError> {
    let value = value.trim();
    let bytes = if value.starts_with('[') {
        serde_json::from_str::<Vec<u8>>(value).map_err(|_| SigningError::InvalidSolanaKeypair)?
    } else {
        bs58::decode(value)
            .into_vec()
            .map_err(|_| SigningError::InvalidSolanaKeypair)?
    };
    match bytes.len() {
        32 | 64 => Ok(bytes),
        _ => Err(SigningError::InvalidSolanaKeypair),
    }
}

pub fn solana_address_from_keypair(value: &str) -> Result<String, SigningError> {
    let secret = decode_solana_keypair(value)?;
    Ok(bs58::encode(ed25519_public_key(&secret)?).into_string())
}

pub fn secp256k1_address(private_key_hex: &str) -> Result<[u8; 20], SigningError> {
    let key = secp256k1_signing_key(private_key_hex)?;
    let point = key.verifying_key().to_encoded_point(false);
    let bytes = point.as_bytes();
    if bytes.len() != 65 || bytes[0] != 4 {
        return Err(SigningError::InvalidSecp256k1Secret);
    }
    let hash: [u8; 32] = Keccak256::digest(&bytes[1..]).into();
    hash[12..]
        .try_into()
        .map_err(|_| SigningError::InvalidSecp256k1Secret)
}

pub fn secp256k1_sign_prehash_recoverable(
    private_key_hex: &str,
    digest: [u8; 32],
) -> Result<RecoverableSecp256k1Signature, SigningError> {
    let key = secp256k1_signing_key(private_key_hex)?;
    let (signature, recovery_id) = key
        .sign_prehash_recoverable(&digest)
        .map_err(|_| SigningError::Secp256k1)?;
    Ok(RecoverableSecp256k1Signature {
        r: signature.r().to_bytes().into(),
        s: signature.s().to_bytes().into(),
        recovery_id: recovery_id.to_byte(),
    })
}

pub fn keccak256(bytes: &[u8]) -> [u8; 32] {
    Keccak256::digest(bytes).into()
}

fn ed25519_seed(secret: &[u8]) -> Result<[u8; 32], SigningError> {
    let seed = match secret.len() {
        32 => secret,
        64 => &secret[..32],
        _ => return Err(SigningError::InvalidEd25519Secret),
    };
    seed.try_into()
        .map_err(|_| SigningError::InvalidEd25519Secret)
}

fn secp256k1_signing_key(private_key_hex: &str) -> Result<Secp256k1SigningKey, SigningError> {
    let value = private_key_hex
        .trim()
        .strip_prefix("0x")
        .unwrap_or(private_key_hex.trim());
    let bytes = hex::decode(value).map_err(|_| SigningError::InvalidSecp256k1Secret)?;
    Secp256k1SigningKey::from_slice(&bytes).map_err(|_| SigningError::InvalidSecp256k1Secret)
}

fn hmac_sha256(secret: &[u8]) -> HmacSha256 {
    match HmacSha256::new_from_slice(secret) {
        Ok(mac) => mac,
        Err(_) => unreachable!("HMAC accepts any key length"),
    }
}

fn hmac_sha512(secret: &[u8]) -> HmacSha512 {
    match HmacSha512::new_from_slice(secret) {
        Ok(mac) => mac,
        Err(_) => unreachable!("HMAC accepts any key length"),
    }
}

fn hmac_sha1(secret: &[u8]) -> HmacSha1 {
    match HmacSha1::new_from_slice(secret) {
        Ok(mac) => mac,
        Err(_) => unreachable!("HMAC accepts any key length"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn hmac_sha256_hex_known_vector() {
        // RFC 4231 test case 1
        let key = b"\x0b".repeat(20);
        let data = b"Hi There";
        let expected = "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7";
        assert_eq!(hmac_sha256_hex(&key, data), expected);
    }

    #[test]
    fn hmac_sha256_base64_round_trip() -> Result<(), base64::DecodeError> {
        let signed = hmac_sha256_base64(b"key", b"payload");
        let decoded = B64_STANDARD.decode(signed)?;
        assert_eq!(decoded.len(), 32);
        Ok(())
    }

    #[test]
    fn hmac_sha512_base64_round_trip() -> Result<(), base64::DecodeError> {
        let signed = hmac_sha512_base64(b"key", b"payload");
        let decoded = B64_STANDARD.decode(signed)?;
        assert_eq!(decoded.len(), 64);
        Ok(())
    }

    #[test]
    fn ed25519_signature_is_base64_encoded() {
        let seed = [7_u8; 32];
        let signature = ed25519_sign_bytes_base64(&seed, b"payload").expect("sign");
        let decoded = B64_STANDARD.decode(signature).expect("base64");
        assert_eq!(decoded.len(), 64);
    }

    #[test]
    fn solana_keypair_decodes_common_formats_and_derives_public_key() {
        let seed = [7_u8; 32];
        let encoded = bs58::encode(seed).into_string();
        assert_eq!(
            decode_solana_keypair(&encoded).expect("base58 keypair"),
            seed.to_vec()
        );
        assert_eq!(
            decode_solana_keypair(&serde_json::to_string(&seed.to_vec()).expect("json"))
                .expect("json keypair"),
            seed.to_vec()
        );
        assert_eq!(ed25519_public_key(&seed).expect("public key").len(), 32);
    }

    #[test]
    fn secp256k1_key_one_matches_the_known_ethereum_address() {
        let key = format!("{:064x}", 1);
        assert_eq!(
            hex::encode(secp256k1_address(&key).expect("address")),
            "7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );
        let signature =
            secp256k1_sign_prehash_recoverable(&key, keccak256(b"payload")).expect("signature");
        assert!(signature.recovery_id <= 1);
        assert_ne!(signature.r, [0; 32]);
        assert_ne!(signature.s, [0; 32]);
    }
}
