//! Kraken Spot and Derivatives authentication.
//!
//! Official contracts:
//! - <https://docs.kraken.com/exchange/guides/rest/authentication>
//! - <https://docs.kraken.com/exchange/guides/futures/rest>
//! - <https://docs.kraken.com/exchange/guides/futures/websockets>

use common::signing::{decode_base64, hmac_sha512_base64, SigningError};
use sha2::{Digest, Sha256};

/// Spot REST `API-Sign`.
pub fn spot_rest_sign(
    secret_base64: &str,
    uri_path: &str,
    nonce: &str,
    encoded_post_data: &str,
) -> Result<String, SigningError> {
    let secret = decode_base64(secret_base64)?;
    let digest = Sha256::digest(format!("{nonce}{encoded_post_data}").as_bytes());
    let mut message = Vec::with_capacity(uri_path.len() + digest.len());
    message.extend_from_slice(uri_path.as_bytes());
    message.extend_from_slice(&digest);
    Ok(hmac_sha512_base64(&secret, &message))
}

/// Derivatives REST `Authent` for the post-2024 URL-encoded flow.
pub fn futures_rest_sign(
    secret_base64: &str,
    encoded_post_data: &str,
    nonce: &str,
    endpoint_path: &str,
) -> Result<String, SigningError> {
    let secret = decode_base64(secret_base64)?;
    let digest = Sha256::digest(format!("{encoded_post_data}{nonce}{endpoint_path}").as_bytes());
    Ok(hmac_sha512_base64(&secret, &digest))
}

/// Derivatives private WebSocket signed challenge.
pub fn futures_ws_challenge_sign(
    secret_base64: &str,
    challenge: &str,
) -> Result<String, SigningError> {
    let secret = decode_base64(secret_base64)?;
    let digest = Sha256::digest(challenge.as_bytes());
    Ok(hmac_sha512_base64(&secret, &digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_signature_matches_official_vector() {
        let signature = spot_rest_sign(
            "kQH5HW/8p1uGOVjbgWA7FunAmGO8lsSUXNsu3eow76sz84Q18fWxnyRzBHCd3pd5nE9qa99HAZtuZuj6F1huXg==",
            "/0/private/AddOrder",
            "1616492376594",
            "nonce=1616492376594&ordertype=limit&pair=XBTUSD&price=37500&type=buy&volume=1.25",
        )
        .expect("official secret decodes");
        assert_eq!(
            signature,
            "4/dpxb3iT4tp/ZCVEwSnEsLxx0bqyhLpdfOpc6fn7OR8+UClSV5n9E6aSS8MPtnRfp32bAb0nmbRn6H8ndwLUQ=="
        );
    }

    #[test]
    fn futures_challenge_matches_official_vector() {
        let signature = futures_ws_challenge_sign(
            "7zxMEF5p/Z8l2p2U7Ghv6x14Af+Fx+92tPgUdVQ748FOIrEoT9bgT+bTRfXc5pz8na+hL/QdrCVG7bh9KpT0eMTm",
            "c100b894-1729-464d-ace1-52dbce11db42",
        )
        .expect("official secret decodes");
        assert_eq!(
            signature,
            "4JEpF3ix66GA2B+ooK128Ift4XQVtc137N9yeg4Kqsn9PI0Kpzbysl9M1IeCEdjg0zl00wkVqcsnG4bmnlMb3A=="
        );
    }

    #[test]
    fn futures_rest_signature_binds_encoded_payload_nonce_and_path() {
        let secret = "c2VjcmV0";
        let signed = futures_rest_sign(
            secret,
            "symbol=PF_XBTUSD&size=1",
            "1700000000000",
            "/derivatives/api/v3/sendorder",
        )
        .expect("secret decodes");
        assert_eq!(signed.len(), 88);
        assert_ne!(
            signed,
            futures_rest_sign(
                secret,
                "symbol=PF_XBTUSD&size=2",
                "1700000000000",
                "/derivatives/api/v3/sendorder",
            )
            .expect("secret decodes")
        );
    }
}
